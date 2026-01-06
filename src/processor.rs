use std::sync::{Arc, Mutex};

use crate::serial::CardDetected;
use crate::state::{Decision, GatewayState};

pub struct GatewayProcessor {
    pub state: Arc<Mutex<GatewayState>>,
}

impl GatewayProcessor {
    pub fn new(state: Arc<Mutex<GatewayState>>) -> Self {
        Self { state }
    }

    pub fn handle_card(&mut self, detected: CardDetected, now: u64) -> Decision {
        let mut state = self.state.lock().expect("state lock poisoned");
        let decision = state.handle_card_detected(detected, now);
        if decision.upload_record.is_some() {
            if let Some(ref event) = decision.event {
                let _ = state.tap_cache.push(event.clone());
            }
        }
        decision
    }
}
