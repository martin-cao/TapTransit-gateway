// 模块划分：串口、协议、处理管线、网络与 Web UI
mod api;
mod cache;
mod model;
mod net;
mod pipeline;
mod processor;
mod proto;
mod serial;
mod serial_io;
mod state;
mod upload;
mod web;
mod web_server;
mod uart_link;
mod smart_led;

use std::sync::{mpsc, Arc, Mutex};

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{AnyInputPin, AnyOutputPin};
use esp_idf_hal::prelude::*;
use esp_idf_hal::uart;
use pipeline::spawn_processor_loop;
use processor::GatewayProcessor;

fn main() {
    // ESP-IDF 运行时初始化（链接补丁 & 日志）
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("TapTransit gateway booting (ESP-IDF)...");

    // 外设初始化：UART + GPIO + RMT
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let modem = peripherals.modem;
    let rmt_channel = peripherals.rmt.channel0;
    let uart_config = uart::config::Config::new().baudrate(Hertz(115_200));
    let uart = uart::UartDriver::new(
        peripherals.uart0,
        pins.gpio43,
        pins.gpio44,
        AnyInputPin::none(),
        AnyOutputPin::none(),
        &uart_config,
    )
    .unwrap();
    let (uart_tx, uart_rx) = uart.into_split();

    // 共享状态（线路、站点、健康状态等）
    let settings = model::GatewaySettings::default();
    let state = Arc::new(Mutex::new(state::GatewayState::bootstrap(settings.clone())));
    // 智能灯条任务：反映系统状态
    smart_led::spawn_led_task(rmt_channel, pins.gpio48, state.clone());

    // 处理管线：串口输入 -> 业务处理 -> 上报
    let pipeline::GatewayChannels {
        card_tx,
        card_rx,
        ack_tx,
        ack_rx,
        upload_tx,
        upload_rx,
    } = pipeline::GatewayChannels::new();
    let (net_cmd_tx, net_cmd_rx) = mpsc::channel();
    let processor = GatewayProcessor::new(state.clone());
    let _processor_handle =
        spawn_processor_loop(processor, card_rx, ack_tx.clone(), upload_tx.clone(), net_cmd_tx.clone());
    let (_uart_rx_handle, _uart_tx_handle) =
        uart_link::spawn_uart_tasks(uart_rx, uart_tx, card_tx.clone(), ack_rx);

    // 连接 Wi-Fi（失败不阻塞主流程，保持离线可用）
    let _wifi = match net::connect_wifi(modem) {
        Ok(wifi) => {
            if let Ok(mut state) = state.lock() {
                state.update_health(Some(true), None);
            }
            Some(wifi)
        }
        Err(err) => {
            log::warn!("Wi-Fi connect failed: {:?}", err);
            None
        }
    };

    // 可选：编译期配置默认线路
    let default_route_id = option_env!("DEFAULT_ROUTE_ID")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    if default_route_id > 0 {
        if let Ok(mut state) = state.lock() {
            let direction = state.route_state.direction;
            state.update_route(default_route_id, 0, "未设置".to_string(), direction);
        }
        let _ = net_cmd_tx.send(net::NetCommand::SyncConfig {
            route_id: default_route_id,
        });
    }

    // 启动网络上传与 Web 管理界面
    let _net_handle = net::spawn_network_loop(state.clone(), upload_rx, net_cmd_rx, settings);
    let _server = match web_server::start_server(state.clone(), net_cmd_tx.clone()) {
        Ok(server) => Some(server),
        Err(err) => {
            log::warn!("Web server start failed: {:?}", err);
            None
        }
    };
    let _ = card_tx;

    // 主循环保持任务存活
    loop {
        FreeRtos::delay_ms(1000);
    }
}
