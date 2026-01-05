use std::fmt::Write as _;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use esp_idf_hal::delay;
use esp_idf_hal::uart::{UartRxDriver, UartTxDriver};

use crate::serial::{CardAck, CardDetected};
use crate::serial_io::{push_bytes_to_channel, CardFrameCodec};

pub fn spawn_uart_tasks(
    rx: UartRxDriver<'static>,
    mut tx: UartTxDriver<'static>,
    card_tx: Sender<CardDetected>,
    ack_rx: Receiver<CardAck>,
) -> (thread::JoinHandle<()>, thread::JoinHandle<()>) {
    let rx_handle = thread::spawn(move || {
        let mut codec = CardFrameCodec::new();
        let mut buf = [0u8; 128];
        loop {
            match rx.read(&mut buf, delay::BLOCK) {
                Ok(count) if count > 0 => {
                    log_bytes("UART RX:", &buf[..count]);
                    push_bytes_to_channel(&mut codec, &buf[..count], &card_tx);
                }
                Ok(_) => {}
                Err(err) => {
                    log::warn!("UART RX error: {:?}", err);
                }
            }
        }
    });

    let tx_handle = thread::spawn(move || {
        while let Ok(ack) = ack_rx.recv() {
            let bytes = CardFrameCodec::ack_to_bytes(&ack);
            log_bytes("UART TX:", &bytes);
            if let Err(err) = tx.write(&bytes) {
                log::warn!("UART TX error: {:?}", err);
            }
            let _ = tx.wait_done(delay::BLOCK);
        }
    });

    (rx_handle, tx_handle)
}

fn log_bytes(prefix: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut line = String::with_capacity(prefix.len() + bytes.len() * 3);
    line.push_str(prefix);
    line.push(' ');
    for (idx, byte) in bytes.iter().enumerate() {
        if idx > 0 {
            line.push(' ');
        }
        let _ = write!(line, "{:02X}", byte);
    }
    log::info!("{}", line);
}
