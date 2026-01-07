use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::UploadRecord;
use crate::net::NetCommand;
use crate::processor::GatewayProcessor;
use crate::serial::{CardAck, CardDetected};

/// 处理管线的通道集合（刷卡事件、ACK、上传）。
pub struct GatewayChannels {
    pub card_tx: Sender<CardDetected>,
    pub card_rx: Receiver<CardDetected>,
    pub ack_tx: Sender<CardAck>,
    pub ack_rx: Receiver<CardAck>,
    pub upload_tx: Sender<UploadRecord>,
    pub upload_rx: Receiver<UploadRecord>,
}

impl GatewayChannels {
    /// 创建默认的 mpsc 通道。
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

/// 启动处理器线程：消费刷卡事件并产出 ACK/上传记录。
pub fn spawn_processor_loop(
    mut processor: GatewayProcessor,
    card_rx: Receiver<CardDetected>,
    ack_tx: Sender<CardAck>,
    upload_tx: Sender<UploadRecord>,
    net_cmd_tx: Sender<NetCommand>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        // 阻塞等待刷卡事件
        while let Ok(card) = card_rx.recv() {
            let now = current_epoch();
            let decision = processor.handle_card(card, now);
            // 发送串口 ACK
            let _ = ack_tx.send(decision.ack);
            if let Some(record) = decision.upload_record {
                // 推送上报记录
                let _ = upload_tx.send(record);
            }
            if let Some(ref event) = decision.event {
                // 触发卡片信息查询（后台可回写折扣）
                let _ = net_cmd_tx.send(NetCommand::LookupCard {
                    card_id: event.card_id.clone(),
                });
            }
        }
    })
}

/// 获取当前时间戳（秒）。
fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
