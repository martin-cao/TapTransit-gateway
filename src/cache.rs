use crate::model::{RouteConfig, TapEvent};

pub struct TapEventCache {
    max_len: usize,
    events: Vec<TapEvent>,
}

impl TapEventCache {
    pub fn new(max_len: usize) -> Self {
        Self {
            max_len,
            events: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_full(&self) -> bool {
        self.events.len() >= self.max_len
    }

    pub fn push(&mut self, event: TapEvent) -> Result<(), TapEvent> {
        if self.is_full() {
            return Err(event);
        }
        self.events.push(event);
        Ok(())
    }

    pub fn drain_batch(&mut self, limit: usize) -> Vec<TapEvent> {
        let take = core::cmp::min(limit, self.events.len());
        self.events.drain(0..take).collect()
    }
}

pub struct ConfigCache {
    pub route: Option<RouteConfig>,
    pub fetched_at: u64,
    pub ttl_secs: u32,
}

impl ConfigCache {
    pub fn new(ttl_secs: u32) -> Self {
        Self {
            route: None,
            fetched_at: 0,
            ttl_secs,
        }
    }

    pub fn is_expired(&self, now: u64) -> bool {
        self.route.is_none() || now.saturating_sub(self.fetched_at) > self.ttl_secs as u64
    }

    pub fn update(&mut self, route: RouteConfig, now: u64) {
        self.route = Some(route);
        self.fetched_at = now;
    }
}

pub struct BlacklistCache {
    pub cards: Vec<String>,
    pub fetched_at: u64,
    pub ttl_secs: u32,
}

impl BlacklistCache {
    pub fn new(ttl_secs: u32) -> Self {
        Self {
            cards: Vec::new(),
            fetched_at: 0,
            ttl_secs,
        }
    }

    pub fn is_expired(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) > self.ttl_secs as u64
    }

    pub fn replace(&mut self, cards: Vec<String>, now: u64) {
        self.cards = cards;
        self.fetched_at = now;
    }

    pub fn is_blocked(&self, card_id: &str) -> bool {
        self.cards.iter().any(|id| id == card_id)
    }
}

pub struct TapDebounce {
    window_secs: u32,
    max_len: usize,
    entries: Vec<TapSeen>,
}

struct TapSeen {
    card_id: String,
    last_seen: u64,
}

impl TapDebounce {
    pub fn new(window_secs: u32, max_len: usize) -> Self {
        Self {
            window_secs,
            max_len,
            entries: Vec::new(),
        }
    }

    pub fn allow(&mut self, card_id: &str, now: u64) -> bool {
        self.purge_expired(now);

        if let Some(entry) = self.entries.iter_mut().find(|e| e.card_id == card_id) {
            if now.saturating_sub(entry.last_seen) <= self.window_secs as u64 {
                return false;
            }
            entry.last_seen = now;
            return true;
        }

        if self.entries.len() >= self.max_len {
            self.drop_oldest();
        }
        self.entries.push(TapSeen {
            card_id: card_id.to_string(),
            last_seen: now,
        });
        true
    }

    fn purge_expired(&mut self, now: u64) {
        let window = self.window_secs as u64;
        self.entries.retain(|e| now.saturating_sub(e.last_seen) <= window);
    }

    fn drop_oldest(&mut self) {
        if let Some((idx, _)) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.last_seen)
        {
            self.entries.swap_remove(idx);
        }
    }
}

pub struct ActiveTripCache {
    ttl_secs: u32,
    entries: Vec<ActiveTrip>,
}

struct ActiveTrip {
    card_id: String,
    event: TapEvent,
    last_seen: u64,
}

impl ActiveTripCache {
    pub fn new(ttl_secs: u32) -> Self {
        Self {
            ttl_secs,
            entries: Vec::new(),
        }
    }

    pub fn insert(&mut self, event: TapEvent, now: u64) {
        self.purge_expired(now);
        self.entries.retain(|e| e.card_id != event.card_id);
        self.entries.push(ActiveTrip {
            card_id: event.card_id.clone(),
            event,
            last_seen: now,
        });
    }

    pub fn take(&mut self, card_id: &str, now: u64) -> Option<TapEvent> {
        self.purge_expired(now);
        if let Some(pos) = self.entries.iter().position(|e| e.card_id == card_id) {
            return Some(self.entries.swap_remove(pos).event);
        }
        None
    }

    fn purge_expired(&mut self, now: u64) {
        let ttl = self.ttl_secs as u64;
        self.entries.retain(|e| now.saturating_sub(e.last_seen) <= ttl);
    }
}
