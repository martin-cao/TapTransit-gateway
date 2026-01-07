use std::sync::{Arc, Mutex};

use crate::serial::CardDetected;
use crate::state::{Decision, GatewayState};

/// 网关业务处理器（串口事件 -> 决策）。
pub struct GatewayProcessor {
    pub state: Arc<Mutex<GatewayState>>,
}

impl GatewayProcessor {
    /// 创建处理器，持有共享状态。
    pub fn new(state: Arc<Mutex<GatewayState>>) -> Self {
        Self { state }
    }

    /// 处理刷卡事件，生成 ACK 与上传记录。
    pub fn handle_card(&mut self, detected: CardDetected, now: u64) -> Decision {
        let mut state = self.state.lock().expect("state lock poisoned");
        let decision = state.handle_card_detected(detected, now);
        if decision.upload_record.is_some() {
            if let Some(ref event) = decision.event {
                // 缓存 tap 事件，供 UI 或离线上报
                let _ = state.tap_cache.push(event.clone());
            }
        }
        decision
    }
}
