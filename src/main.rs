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

use std::sync::{mpsc, Arc, Mutex};

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{AnyInputPin, AnyOutputPin};
use esp_idf_hal::prelude::*;
use esp_idf_hal::uart;
use pipeline::spawn_processor_loop;
use processor::GatewayProcessor;

fn main() {
    // Required by esp-idf-sys for link patches.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("TapTransit gateway booting (ESP-IDF)...");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let modem = peripherals.modem;
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

    let settings = model::GatewaySettings::default();
    let state = Arc::new(Mutex::new(state::GatewayState::bootstrap(settings.clone())));
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

    let _net_handle = net::spawn_network_loop(state.clone(), upload_rx, net_cmd_rx, settings);
    let _server = match web_server::start_server(state.clone(), net_cmd_tx.clone()) {
        Ok(server) => Some(server),
        Err(err) => {
            log::warn!("Web server start failed: {:?}", err);
            None
        }
    };
    let _ = card_tx;

    loop {
        FreeRtos::delay_ms(1000);
    }
}
