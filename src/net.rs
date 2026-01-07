use core::convert::TryInto;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::io::Write as _;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::modem::Modem;
use esp_idf_hal::sys::EspError;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::client::EspHttpConnection;
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use serde::Deserialize;

use crate::api::{
    BATCH_RECORDS_PATH, CARD_REGISTER_PATH, CARD_STATE_BATCH_PATH, CARDS_PATH, CONFIG_PATH,
};
use crate::model::{
    CardRegistration, CardStateSnapshot, FareRule, FareType, GatewaySettings, PassengerTone,
    RouteConfig, StationConfig, TapMode, UploadRecord,
};
use crate::state::GatewayState;
use crate::upload::BatchUpload;

// Wi-Fi 与后端地址来自编译期环境变量。
const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");
const BACKEND_BASE_URL: &str = env!("BACKEND_BASE_URL");

/// 网络控制命令（来自 UI 或业务逻辑）。
#[derive(Clone, Debug)]
pub enum NetCommand {
    SyncConfig { route_id: u16 },
    UploadNow,
    SetBackend { base_url: String },
    LookupCard { card_id: String },
    RegisterCard { payload: CardRegistration },
}

/// 网络请求错误类型。
#[derive(Debug)]
pub enum NetError {
    Io(EspIOError),
    Json(serde_json::Error),
    HttpStatus(u16),
    Api(String),
}

impl From<EspIOError> for NetError {
    fn from(err: EspIOError) -> Self {
        NetError::Io(err)
    }
}

impl From<EspError> for NetError {
    fn from(err: EspError) -> Self {
        NetError::Io(EspIOError::from(err))
    }
}

impl From<serde_json::Error> for NetError {
    fn from(err: serde_json::Error) -> Self {
        NetError::Json(err)
    }
}

/// 通用 API 响应格式（与后端保持一致）。
#[derive(Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct CardStateBatchResponse {
    accepted: Option<Vec<String>>,
    rejected: Option<Vec<CardStateReject>>,
}

#[derive(Deserialize)]
struct CardStateReject {
    card_id: String,
    reason: Option<String>,
}

/// 连接 Wi-Fi（阻塞直到联网）。
pub fn connect_wifi(modem: Modem) -> Result<BlockingWifi<EspWifi<'static>>, EspError> {
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take().ok();
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), nvs)?, sys_loop)?;

    log::info!(
        "Wi-Fi connecting to SSID='{}' (pass_len={})",
        WIFI_SSID,
        WIFI_PASS.len()
    );

    fn try_connect(
        wifi: &mut BlockingWifi<EspWifi<'static>>,
        auth_method: AuthMethod,
    ) -> Result<(), EspError> {
        let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
            ssid: WIFI_SSID.try_into().unwrap(),
            bssid: None,
            auth_method,
            password: WIFI_PASS.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });

        wifi.set_configuration(&wifi_configuration)?;
        wifi.start()?;
        log::info!("Wi-Fi started (auth={:?})", auth_method);
        wifi.connect()?;
        log::info!("Wi-Fi connected to {}", WIFI_SSID);
        wifi.wait_netif_up()?;
        log::info!("Wi-Fi netif up");
        Ok(())
    }

    // 连接策略：
    // - 无密码：开放网络
    // - 有密码：默认优先 WPA2（符合常见热点/课堂环境），失败则尝试 WPA2/WPA3 兼容
    if WIFI_PASS.is_empty() {
        try_connect(&mut wifi, AuthMethod::None)?;
        return Ok(wifi);
    }

    if try_connect(&mut wifi, AuthMethod::WPA2Personal).is_ok() {
        return Ok(wifi);
    }
    log::warn!("Wi-Fi connect retrying with WPA2WPA3Personal...");
    try_connect(&mut wifi, AuthMethod::WPA2WPA3Personal)?;
    Ok(wifi)
}

pub fn spawn_network_loop(
    state: Arc<Mutex<GatewayState>>,
    upload_rx: Receiver<UploadRecord>,
    command_rx: Receiver<NetCommand>,
    settings: GatewaySettings,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        // 上传缓冲区与配置刷新计时
        let mut buffer: Vec<UploadRecord> = Vec::with_capacity(settings.batch_size);
        let mut card_state_buffer: Vec<CardStateSnapshot> = Vec::with_capacity(settings.batch_size);
        let mut route_id: Option<u16> = None;
        let mut last_upload = Instant::now();
        let mut last_state_upload = Instant::now();
        let refresh_secs = settings
            .config_ttl_secs
            .min(settings.blacklist_ttl_secs) as u64;
        let mut last_sync = Instant::now()
            .checked_sub(Duration::from_secs(refresh_secs))
            .unwrap_or_else(Instant::now);
        loop {
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    NetCommand::SyncConfig { route_id: next_route } => {
                        // 立即刷新配置
                        route_id = Some(next_route);
                        if sync_config(&state, next_route) {
                            last_sync = Instant::now();
                        }
                    }
                    NetCommand::UploadNow => {
                        // 立即上报当前缓冲
                        while let Ok(record) = upload_rx.try_recv() {
                            buffer.push(record);
                        }
                        if let Err(err) = flush_batch(&state, &mut buffer) {
                            log::warn!("Upload batch failed: {:?}", err);
                        }
                        if let Err(err) = flush_card_state_batch(&state, &mut card_state_buffer) {
                            log::warn!("Card state upload failed: {:?}", err);
                        }
                    }
                    NetCommand::SetBackend { base_url } => {
                        // 切换后端地址
                        if let Ok(mut state) = state.lock() {
                            state.update_backend_base_url(base_url);
                        }
                    }
                    NetCommand::LookupCard { card_id } => {
                        // 查询卡片信息（票种/折扣/状态）
                        let base_url = resolve_base_url(&state);
                        match fetch_card_profile(&base_url, &card_id) {
                            Ok(Some(profile)) => {
                                apply_card_profile(&state, &card_id, profile);
                            }
                            Ok(None) => {}
                            Err(err) => {
                                log::warn!("Card lookup failed: {:?}", err);
                            }
                        }
                    }
                    NetCommand::RegisterCard { payload } => {
                        let base_url = resolve_base_url(&state);
                        if let Err(err) = register_card(&base_url, payload) {
                            log::warn!("Card register failed: {:?}", err);
                        }
                    }
                }
            }

            if let Some(route_id) = route_id {
                if last_sync.elapsed() >= Duration::from_secs(refresh_secs) {
                    // 定期刷新配置与黑名单
                    if sync_config(&state, route_id) {
                        last_sync = Instant::now();
                    }
                }
            }

            match upload_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(record) => {
                    buffer.push(record);
                    last_upload = Instant::now();
                    if buffer.len() >= settings.batch_size {
                        // 达到批量阈值触发上传
                        if let Err(err) = flush_batch(&state, &mut buffer) {
                            log::warn!("Upload batch failed: {:?}", err);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // 超时且有缓存，按时间间隔触发上传
                    if !buffer.is_empty() && last_upload.elapsed() >= Duration::from_secs(5) {
                        if let Err(err) = flush_batch(&state, &mut buffer) {
                            log::warn!("Upload batch failed: {:?}", err);
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }

            // 按时间间隔刷新卡片状态快照
            if last_state_upload.elapsed() >= Duration::from_secs(5) {
                if let Ok(mut state) = state.lock() {
                    let drained = state.card_state_cache.drain_batch(settings.batch_size);
                    card_state_buffer.extend(drained);
                }
                if !card_state_buffer.is_empty() {
                    if let Err(err) = flush_card_state_batch(&state, &mut card_state_buffer) {
                        log::warn!("Card state upload failed: {:?}", err);
                    } else {
                        last_state_upload = Instant::now();
                    }
                }
            }
        }
    })
}

/// 上报一批记录到后端。
fn flush_batch(state: &Arc<Mutex<GatewayState>>, buffer: &mut Vec<UploadRecord>) -> Result<(), NetError> {
    if buffer.is_empty() {
        return Ok(());
    }
    let payload = BatchUpload::new(buffer.clone()).to_json_string();
    let base_url = resolve_base_url(state);
    let url = format!("{}{}", base_url, BATCH_RECORDS_PATH);
    let content_length = payload.len().to_string();
    let headers = [
        ("content-type", "application/json"),
        ("content-length", content_length.as_str()),
    ];

    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let mut request = client.request(Method::Post, &url, &headers)?;
    request.write_all(payload.as_bytes())?;
    request.flush()?;
    log::info!("Uploading batch to {}", url);
    let response = request.submit()?;
    let status = response.status();
    log::info!("Upload response status {}", status);
    if !(200..300).contains(&status) {
        update_backend_status(state, false);
        return Err(NetError::HttpStatus(status));
    }
    buffer.clear();
    if let Ok(mut state) = state.lock() {
        state.tap_cache.clear();
    }
    update_backend_status(state, true);
    Ok(())
}

/// 上报卡片状态快照批次。
fn flush_card_state_batch(
    state: &Arc<Mutex<GatewayState>>,
    buffer: &mut Vec<CardStateSnapshot>,
) -> Result<(), NetError> {
    if buffer.is_empty() {
        return Ok(());
    }
    let payload = serde_json::to_string(&buffer)?;
    let base_url = resolve_base_url(state);
    let url = format!("{}{}", base_url, CARD_STATE_BATCH_PATH);
    let content_length = payload.len().to_string();
    let headers = [
        ("content-type", "application/json"),
        ("content-length", content_length.as_str()),
    ];

    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let mut request = client.request(Method::Post, &url, &headers)?;
    request.write_all(payload.as_bytes())?;
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        update_backend_status(state, false);
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<CardStateBatchResponse> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(
            payload.message.unwrap_or_else(|| "card state upload failed".to_string()),
        ));
    }
    if let Some(result) = payload.data {
        if let Some(rejected) = result.rejected {
            // rejected 代表“状态校验失败”，不等价于“应封禁”。
            // 只在后端明确返回“card blocked”时，才将卡加入黑名单缓存。
            let mut to_blacklist: Vec<String> = Vec::new();
            for item in rejected {
                if matches!(item.reason.as_deref(), Some("card blocked")) {
                    to_blacklist.push(item.card_id);
                } else {
                    log::warn!(
                        "Card state rejected (not blacklisting): card_id={}, reason={:?}",
                        item.card_id,
                        item.reason
                    );
                }
            }
            if !to_blacklist.is_empty() {
                let now = current_epoch();
                if let Ok(mut state) = state.lock() {
                    for card_id in to_blacklist {
                        if !state.blacklist_cache.is_blocked(&card_id) {
                            state.blacklist_cache.cards.push(card_id);
                        }
                    }
                    state.blacklist_cache.fetched_at = now;
                }
            }
        }
    }
    buffer.clear();
    update_backend_status(state, true);
    Ok(())
}

/// 同步线路配置与黑名单。
fn sync_config(state: &Arc<Mutex<GatewayState>>, route_id: u16) -> bool {
    let now = current_epoch();
    let mut ok = false;
    let base_url = resolve_base_url(state);

    match fetch_route_config(&base_url, route_id) {
        Ok(config) => {
            if let Ok(mut state) = state.lock() {
                state.update_route_config(config, now);
            }
            ok = true;
        }
        Err(err) => {
            log::warn!("Route config fetch failed: {:?}", err);
        }
    }

    match fetch_blacklist(&base_url) {
        Ok(cards) => {
            if let Ok(mut state) = state.lock() {
                state.update_blacklist(cards, now);
            }
            ok = true;
        }
        Err(err) => {
            log::warn!("Blacklist fetch failed: {:?}", err);
        }
    }

    update_backend_status(state, ok);
    ok
}

/// 请求后端线路配置。
fn fetch_route_config(base_url: &str, route_id: u16) -> Result<RouteConfig, NetError> {
    let url = format!("{}{}?route_id={}", base_url, CONFIG_PATH, route_id);
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<RouteConfigResponse> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(payload.message.unwrap_or_else(|| "request failed".to_string())));
    }
    let config = payload
        .data
        .ok_or_else(|| NetError::Api("empty config response".to_string()))?;
    Ok(config.into())
}

/// 请求后端黑名单列表。
fn fetch_blacklist(base_url: &str) -> Result<Vec<String>, NetError> {
    let url = format!("{}{}?status=blocked", base_url, CARDS_PATH);
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<Vec<CardResponse>> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(payload.message.unwrap_or_else(|| "request failed".to_string())));
    }
    let cards = payload.data.unwrap_or_default();
    Ok(cards.into_iter().filter_map(|card| card.card_id).collect())
}

/// 上报卡片注册信息。
fn register_card(base_url: &str, payload: CardRegistration) -> Result<(), NetError> {
    let url = format!("{}{}", base_url, CARD_REGISTER_PATH);
    let body = serde_json::to_string(&payload)?;
    let content_length = body.len().to_string();
    let headers = [
        ("content-type", "application/json"),
        ("content-length", content_length.as_str()),
    ];
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let mut request = client.request(Method::Post, &url, &headers)?;
    request.write_all(body.as_bytes())?;
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    let resp_body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<serde_json::Value> = serde_json::from_slice(&resp_body)?;
    if !payload.success {
        return Err(NetError::Api(
            payload.message.unwrap_or_else(|| "register failed".to_string()),
        ));
    }
    Ok(())
}

/// 查询卡片详细信息（票种/状态/折扣）。
fn fetch_card_profile(base_url: &str, card_id: &str) -> Result<Option<CardProfile>, NetError> {
    let url = format!("{}{}?card_id={}", base_url, CARDS_PATH, card_id);
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<Vec<CardResponse>> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(payload.message.unwrap_or_else(|| "request failed".to_string())));
    }
    let cards = payload.data.unwrap_or_default();
    Ok(cards.into_iter().next().map(|card| CardProfile {
        card_type: card.card_type,
        status: card.status,
        balance_cents: card.balance.map(|value| (value.max(0.0) * 100.0).round() as u32),
        discount_rate: card.discount_rate,
        discount_amount: card.discount_amount,
    }))
}

/// 读取 HTTP 响应体。
fn read_response_body(
    response: &mut embedded_svc::http::client::Response<&mut EspHttpConnection>,
) -> Result<Vec<u8>, EspIOError> {
    let mut body = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let len = response.read(&mut buf)?;
        if len == 0 {
            break;
        }
        body.extend_from_slice(&buf[..len]);
    }
    Ok(body)
}

/// 更新后端可达性状态。
fn update_backend_status(state: &Arc<Mutex<GatewayState>>, reachable: bool) {
    if let Ok(mut state) = state.lock() {
        state.update_health(None, Some(reachable));
    }
}

/// 获取当前后端地址（优先使用运行时设置）。
fn resolve_base_url(state: &Arc<Mutex<GatewayState>>) -> String {
    if let Ok(state) = state.lock() {
        if !state.backend_base_url.is_empty() {
            return state.backend_base_url.clone();
        }
    }
    BACKEND_BASE_URL.to_string()
}

/// 将卡片画像应用到网关状态与 UI 提示。
fn apply_card_profile(state: &Arc<Mutex<GatewayState>>, card_id: &str, profile: CardProfile) {
    let Some(tone) = tone_from_profile(&profile) else {
        return;
    };
    if let Ok(mut state) = state.lock() {
        let now_ms = current_epoch_millis();
        state.update_card_cache(
            card_id.to_string(),
            profile.card_type.clone(),
            profile.status.clone(),
            profile.discount_rate,
            profile.discount_amount,
            profile.balance_cents,
            now_ms,
        );
        if state.last_card_id == card_id {
            if tone == PassengerTone::Error {
                state.last_passenger_tone = tone;
                if let Some(status) = profile.status.as_deref() {
                    if status == "blocked" {
                        state.last_passenger_message = "卡已冻结".to_string();
                    } else if status == "lost" {
                        state.last_passenger_message = "卡已挂失".to_string();
                    }
                }
            } else {
                state.update_passenger_tone(tone);
                if let Some(card_type) = profile.card_type.as_deref() {
                    state.apply_card_discount_policy(
                        card_type,
                        profile.discount_rate,
                        profile.discount_amount,
                    );
                }
            }
        }
    }
}

/// 当前时间戳（秒）。
fn current_epoch() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    }
}

/// 当前时间戳（毫秒）。
fn current_epoch_millis() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => 0,
    }
}

/// 后端返回的线路配置（网关侧解析用）。
#[derive(Deserialize)]
struct RouteConfigResponse {
    route_id: u16,
    route_name: String,
    #[serde(default)]
    fare_type: Option<String>,
    #[serde(default)]
    tap_mode: Option<String>,
    max_fare: Option<f32>,
    #[serde(default)]
    stations: Vec<StationResponse>,
    #[serde(default)]
    fares: Vec<FareRuleResponse>,
}

#[derive(Deserialize)]
struct StationResponse {
    #[serde(default)]
    id: Option<u16>,
    name: String,
    sequence: u16,
    zone_id: Option<u16>,
    #[serde(default)]
    is_transfer: Option<bool>,
}

#[derive(Deserialize)]
struct FareRuleResponse {
    #[serde(default)]
    base_price: Option<f32>,
    #[serde(default)]
    fare_type: Option<String>,
    #[serde(default)]
    segment_count: Option<u16>,
    #[serde(default)]
    extra_price: Option<f32>,
    #[serde(default)]
    start_station: Option<u16>,
    #[serde(default)]
    end_station: Option<u16>,
}

#[derive(Deserialize)]
struct CardResponse {
    #[serde(default)]
    card_id: Option<String>,
    #[serde(default)]
    card_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    balance: Option<f64>,
    #[serde(default)]
    discount_rate: Option<f32>,
    #[serde(default)]
    discount_amount: Option<f32>,
}

/// 卡片画像（用于状态与优惠更新）。
struct CardProfile {
    card_type: Option<String>,
    status: Option<String>,
    balance_cents: Option<u32>,
    discount_rate: Option<f32>,
    discount_amount: Option<f32>,
}

/// 根据卡片画像确定提示音色。
fn tone_from_profile(profile: &CardProfile) -> Option<PassengerTone> {
    if let Some(status) = profile.status.as_deref() {
        if status == "blocked" || status == "lost" {
            return Some(PassengerTone::Error);
        }
    }
    match profile.card_type.as_deref() {
        Some("student") => Some(PassengerTone::Student),
        Some("elder") => Some(PassengerTone::Elder),
        Some("disabled") => Some(PassengerTone::Disabled),
        Some("normal") => Some(PassengerTone::Normal),
        _ => None,
    }
}

/// 将后端响应转换为网关内部模型。
impl From<RouteConfigResponse> for RouteConfig {
    fn from(value: RouteConfigResponse) -> Self {
        let fare_type = match value.fare_type.as_deref() {
            Some("segment") => FareType::Segment,
            Some("distance") => FareType::Distance,
            _ => FareType::Uniform,
        };
        let tap_mode = match value.tap_mode.as_deref() {
            Some("tap_in_out") => TapMode::TapInOut,
            _ => TapMode::SingleTap,
        };
        let fares = value
            .fares
            .into_iter()
            .map(|fare| FareRule {
                base_price: fare.base_price.unwrap_or(0.0),
                fare_type: fare.fare_type,
                segment_count: fare.segment_count,
                extra_price: fare.extra_price,
                start_station: fare.start_station,
                end_station: fare.end_station,
            })
            .collect();
        let stations = value
            .stations
            .into_iter()
            .map(|station| StationConfig {
                id: station.id.unwrap_or(0),
                name: station.name,
                sequence: station.sequence,
                zone_id: station.zone_id,
                is_transfer: station.is_transfer.unwrap_or(false),
            })
            .collect();
        RouteConfig {
            route_id: value.route_id,
            route_name: value.route_name,
            fare_type,
            tap_mode,
            max_fare: value.max_fare,
            stations,
            fares,
        }
    }
}
