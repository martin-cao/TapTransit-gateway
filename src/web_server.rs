use std::sync::{mpsc::Sender, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use embedded_svc::http::Method;
use embedded_svc::io::Write as _;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::io::EspIOError;
use serde_json::json;

use crate::net::NetCommand;
use crate::model::{FareType, TapMode};
use crate::state::GatewayState;
use crate::web::{parse_action, render_index, DriverAction, StatusPanel};

/// 启动内置 HTTP 服务（司机操作页）。
pub fn start_server(
    state: Arc<Mutex<GatewayState>>,
    net_cmd_tx: Sender<NetCommand>,
) -> Result<EspHttpServer<'static>, EspIOError> {
    let mut server = EspHttpServer::new(&Configuration {
        stack_size: 8192,
        ..Default::default()
    })?;

    // 首页：渲染 HTML
    let state_root = state.clone();
    server.fn_handler("/", Method::Get, move |req| {
        let status = status_from_state(&state_root);
        req.into_response(200, Some("OK"), &[("content-type", "text/html; charset=utf-8")])?
            .write_all(render_index(&status).as_bytes())
            .map(|_| ())
    })?;

    // 状态接口：JSON
    let state_status = state.clone();
    server.fn_handler("/status", Method::Get, move |req| {
        let status = status_from_state(&state_status);
        let direction_label = match status.direction {
            crate::model::Direction::Up => "上行",
            crate::model::Direction::Down => "下行",
        };
        let tone_class = status.passenger_tone.css_class();
        let tone_label = status.passenger_tone.label();
        let payload = json!({
            "route_id": status.route_id,
            "route_name": status.route_name,
            "station_id": status.station_id,
            "station_name": status.station_name,
            "direction": direction_label,
            "tap_mode_label": status.tap_mode_label,
            "fare_type_label": status.fare_type_label,
            "cache_count": status.cache_count,
            "wifi_connected": status.wifi_connected,
            "backend_reachable": status.backend_reachable,
            "backend_base_url": status.backend_base_url,
            "passenger": {
                "tone_class": tone_class,
                "tone_label": tone_label,
                "message": status.passenger_message,
            },
            "fare": {
                "standard": status.standard_fare,
                "actual": status.last_fare,
                "label": status.last_fare_label,
            }
        });
        let body = payload.to_string();
        req.into_response(200, Some("OK"), &[("content-type", "application/json")])?
            .write_all(body.as_bytes())
            .map(|_| ())
    })?;

    // 操作接口：通过 query 参数触发动作
    let state_action = state.clone();
    let net_cmd_action = net_cmd_tx.clone();
    server.fn_handler("/action", Method::Get, move |req| {
        if let Some(query) = req.uri().splitn(2, '?').nth(1) {
            if let Some(action) = parse_action(query) {
                apply_action(&state_action, &net_cmd_action, action);
            }
        }
        req.into_response(303, Some("See Other"), &[("Location", "/")])?
            .write_all(b"")
            .map(|_| ())
    })?;

    Ok(server)
}

/// 执行司机操作指令，并触发必要的同步/上传。
fn apply_action(state: &Arc<Mutex<GatewayState>>, net_cmd_tx: &Sender<NetCommand>, action: DriverAction) {
    match action {
        DriverAction::SetRoute { route_id } => {
            if let Ok(mut state) = state.lock() {
                let direction = state.route_state.direction;
                state.update_route(route_id, 0, "未设置".to_string(), direction);
            }
            let _ = net_cmd_tx.send(NetCommand::SyncConfig { route_id });
        }
        DriverAction::SetDirection { direction } => {
            if let Ok(mut state) = state.lock() {
                state.set_direction(direction);
            }
        }
        DriverAction::SetStation { station_id } => {
            if let Ok(mut state) = state.lock() {
                let _ = state.set_station_by_id(station_id);
            }
            let _ = net_cmd_tx.send(NetCommand::UploadNow);
        }
        DriverAction::NextStation => {
            if let Ok(mut state) = state.lock() {
                let _ = state.step_station(true);
            }
            let _ = net_cmd_tx.send(NetCommand::UploadNow);
        }
        DriverAction::PrevStation => {
            if let Ok(mut state) = state.lock() {
                let _ = state.step_station(false);
            }
            let _ = net_cmd_tx.send(NetCommand::UploadNow);
        }
        DriverAction::SyncConfig => {
            let route_id = state
                .lock()
                .map(|s| s.route_state.route_id)
                .unwrap_or(0);
            let _ = net_cmd_tx.send(NetCommand::SyncConfig { route_id });
        }
        DriverAction::UploadNow => {
            let _ = net_cmd_tx.send(NetCommand::UploadNow);
        }
        DriverAction::SetBackend { base_url } => {
            let normalized = normalize_backend_url(base_url);
            if let Ok(mut state) = state.lock() {
                state.update_backend_base_url(normalized.clone());
            }
            let _ = net_cmd_tx.send(NetCommand::SetBackend { base_url: normalized });
        }
    }
}

/// 从全局状态构建前端面板展示数据。
fn status_from_state(state: &Arc<Mutex<GatewayState>>) -> StatusPanel {
    if let Ok(mut state) = state.lock() {
        // 清理过期提示
        let now_ms = current_epoch_millis();
        if state.last_message_deadline_ms > 0 && now_ms >= state.last_message_deadline_ms {
            state.last_message_deadline_ms = 0;
            state.last_passenger_tone = crate::model::PassengerTone::Normal;
            state.last_passenger_message = "等待刷卡".to_string();
            state.last_fare_base = None;
            state.last_fare = None;
            state.last_fare_label = "应付".to_string();
            state.last_tap_type = None;
        }
        let mut route_name = String::new();
        let mut tap_mode_label = "未同步".to_string();
        let mut fare_type_label = "未同步".to_string();
        // 若已同步配置，使用更友好的标签
        if let Some(cfg) = state.config_cache.route.as_ref() {
            route_name = cfg.route_name.clone();
            tap_mode_label = match cfg.tap_mode {
                TapMode::SingleTap => "单次刷卡",
                TapMode::TapInOut => "上下车刷卡",
            }
            .to_string();
            fare_type_label = match cfg.fare_type {
                FareType::Uniform => "统一票价",
                FareType::Segment => "分段计价",
                FareType::Distance => "距离计价",
            }
            .to_string();
        }
        StatusPanel {
            route_id: state.route_state.route_id,
            route_name,
            station_id: state.route_state.station_id,
            station_name: state.route_state.station_name.clone(),
            direction: state.route_state.direction,
            tap_mode_label,
            fare_type_label,
            cache_count: state.tap_cache.len(),
            wifi_connected: state.wifi_connected,
            backend_reachable: state.backend_reachable,
            backend_base_url: state.backend_base_url.clone(),
            passenger_tone: state.last_passenger_tone,
            passenger_message: state.last_passenger_message.clone(),
            standard_fare: state.standard_fare(),
            last_fare: state.last_fare,
            last_fare_label: state.last_fare_label.clone(),
        }
    } else {
        // 无法获取锁时返回默认状态
        StatusPanel {
            route_id: 0,
            route_name: String::new(),
            station_id: 0,
            station_name: "未设置".to_string(),
            direction: crate::model::Direction::Up,
            tap_mode_label: "未同步".to_string(),
            fare_type_label: "未同步".to_string(),
            cache_count: 0,
            wifi_connected: false,
            backend_reachable: false,
            backend_base_url: String::new(),
            passenger_tone: crate::model::PassengerTone::Normal,
            passenger_message: "等待刷卡".to_string(),
            standard_fare: None,
            last_fare: None,
            last_fare_label: "应付".to_string(),
        }
    }
}

/// 规范化后端地址（自动补齐协议/去尾斜杠）。
fn normalize_backend_url(input: String) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut url = trimmed.to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("http://{}", url);
    }
    url.trim_end_matches('/').to_string()
}

/// 当前时间戳（毫秒）。
fn current_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
