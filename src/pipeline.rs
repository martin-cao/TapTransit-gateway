use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::UploadRecord;
use crate::net::NetCommand;
use crate::processor::GatewayProcessor;
use crate::serial::{CardDetected, CardWriteResult, SerialCommand};

/// 处理管线的通道集合（刷卡事件、ACK、上传）。
pub struct GatewayChannels {
    pub card_tx: Sender<CardDetected>,
    pub card_rx: Receiver<CardDetected>,
    pub cmd_tx: Sender<SerialCommand>,
    pub cmd_rx: Receiver<SerialCommand>,
    pub upload_tx: Sender<UploadRecord>,
    pub upload_rx: Receiver<UploadRecord>,
    pub write_result_tx: Sender<CardWriteResult>,
    pub write_result_rx: Receiver<CardWriteResult>,
}

impl GatewayChannels {
    /// 创建默认的 mpsc 通道。
    pub fn new() -> Self {
        let (card_tx, card_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (upload_tx, upload_rx) = mpsc::channel();
        let (write_result_tx, write_result_rx) = mpsc::channel();
        Self {
            card_tx,
            card_rx,
            cmd_tx,
            cmd_rx,
            upload_tx,
            upload_rx,
            write_result_tx,
            write_result_rx,
        }
    }
}

/// 启动处理器线程：消费刷卡事件并产出 ACK/上传记录。
pub fn spawn_processor_loop(
    mut processor: GatewayProcessor,
    card_rx: Receiver<CardDetected>,
    cmd_tx: Sender<SerialCommand>,
    upload_tx: Sender<UploadRecord>,
    net_cmd_tx: Sender<NetCommand>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        // 阻塞等待刷卡事件
        while let Ok(card) = card_rx.recv() {
            let now = current_epoch();
            // 无论是否能解析卡内数据，都先尝试从后端查询卡片信息（用于补全余额/状态）。
            let _ = net_cmd_tx.send(NetCommand::LookupCard {
                card_id: card.card_id.clone(),
            });
            let decision = processor.handle_card(card, now);
            // 发送写卡请求（如有）
            if let Some(write_req) = decision.write_request {
                let _ = cmd_tx.send(SerialCommand::Write(write_req));
            }
            // 发送串口 ACK
            let _ = cmd_tx.send(SerialCommand::Ack(decision.ack));
            if let Some(record) = decision.upload_record {
                // 推送上报记录
                let _ = upload_tx.send(record);
            }
            if let Some(registration) = decision.registration {
                let _ = net_cmd_tx.send(NetCommand::RegisterCard { payload: registration });
            }
        }
    })
}

/// 写卡结果处理线程：更新网关状态提示。
pub fn spawn_write_result_loop(
    state: std::sync::Arc<std::sync::Mutex<crate::state::GatewayState>>,
    write_result_rx: Receiver<CardWriteResult>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(result) = write_result_rx.recv() {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if let Ok(mut state) = state.lock() {
                state.handle_write_result(result, now_ms);
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
