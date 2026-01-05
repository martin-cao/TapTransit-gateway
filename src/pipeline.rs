use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::UploadRecord;
use crate::net::NetCommand;
use crate::processor::GatewayProcessor;
use crate::serial::{CardAck, CardDetected};

pub struct GatewayChannels {
    pub card_tx: Sender<CardDetected>,
    pub card_rx: Receiver<CardDetected>,
    pub ack_tx: Sender<CardAck>,
    pub ack_rx: Receiver<CardAck>,
    pub upload_tx: Sender<UploadRecord>,
    pub upload_rx: Receiver<UploadRecord>,
}

impl GatewayChannels {
    pub fn new() -> Self {
        let (card_tx, card_rx) = mpsc::channel();
        let (ack_tx, ack_rx) = mpsc::channel();
        let (upload_tx, upload_rx) = mpsc::channel();
        Self {
            card_tx,
            card_rx,
            ack_tx,
            ack_rx,
            upload_tx,
            upload_rx,
        }
    }
}

pub fn spawn_processor_loop(
    mut processor: GatewayProcessor,
    card_rx: Receiver<CardDetected>,
    ack_tx: Sender<CardAck>,
    upload_tx: Sender<UploadRecord>,
    net_cmd_tx: Sender<NetCommand>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(card) = card_rx.recv() {
            let now = current_epoch();
            let decision = processor.handle_card(card, now);
            let _ = ack_tx.send(decision.ack);
            if let Some(record) = decision.upload_record {
                let _ = upload_tx.send(record);
            }
            if let Some(ref event) = decision.event {
                let _ = net_cmd_tx.send(NetCommand::LookupCard {
                    card_id: event.card_id.clone(),
                });
            }
        }
    })
}

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
