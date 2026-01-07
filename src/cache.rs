use crate::model::{RouteConfig, TapEvent};

/// 刷卡事件缓存（用于批量上报或 UI 显示）。
pub struct TapEventCache {
    max_len: usize,
    events: Vec<TapEvent>,
}

impl TapEventCache {
    /// 创建事件缓存，指定最大容量。
    pub fn new(max_len: usize) -> Self {
        Self {
            max_len,
            events: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 是否已达到容量上限。
    pub fn is_full(&self) -> bool {
        self.events.len() >= self.max_len
    }

    /// 推入事件，若超容量返回原事件。
    pub fn push(&mut self, event: TapEvent) -> Result<(), TapEvent> {
        if self.is_full() {
            return Err(event);
        }
        self.events.push(event);
        Ok(())
    }

    /// 取出一批事件（FIFO）。
    pub fn drain_batch(&mut self, limit: usize) -> Vec<TapEvent> {
        let take = core::cmp::min(limit, self.events.len());
        self.events.drain(0..take).collect()
    }

    /// 清空缓存。
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

/// 线路配置缓存（含过期时间）。
pub struct ConfigCache {
    pub route: Option<RouteConfig>,
    pub fetched_at: u64,
    pub ttl_secs: u32,
}

impl ConfigCache {
    /// 创建配置缓存。
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

    /// 更新缓存内容与时间戳。
    pub fn update(&mut self, route: RouteConfig, now: u64) {
        self.route = Some(route);
        self.fetched_at = now;
    }
}

/// 黑名单缓存（用于快速拒绝刷卡）。
pub struct BlacklistCache {
    pub cards: Vec<String>,
    pub fetched_at: u64,
    pub ttl_secs: u32,
}

impl BlacklistCache {
    /// 创建黑名单缓存。
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

    /// 替换全部黑名单并更新时间戳。
    pub fn replace(&mut self, cards: Vec<String>, now: u64) {
        self.cards = cards;
        self.fetched_at = now;
    }

    /// 判断卡号是否被拉黑。
    pub fn is_blocked(&self, card_id: &str) -> bool {
        self.cards.iter().any(|id| id == card_id)
    }
}

/// 刷卡防抖缓存（避免短时间重复刷卡）。
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
    /// 创建防抖缓存。
    pub fn new(window_secs: u32, max_len: usize) -> Self {
        Self {
            window_secs,
            max_len,
            entries: Vec::new(),
        }
    }

    pub fn allow(&mut self, card_id: &str, now: u64) -> bool {
        // 清理过期条目
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

    /// 清理超出时间窗口的条目。
    fn purge_expired(&mut self, now: u64) {
        let window = self.window_secs as u64;
        self.entries.retain(|e| now.saturating_sub(e.last_seen) <= window);
    }

    /// 丢弃最旧的一条记录。
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

/// 进出站模式下的未完成行程缓存。
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
    /// 创建行程缓存。
    pub fn new(ttl_secs: u32) -> Self {
        Self {
            ttl_secs,
            entries: Vec::new(),
        }
    }

    pub fn insert(&mut self, event: TapEvent, now: u64) {
        // 插入前清理过期记录
        self.purge_expired(now);
        self.entries.retain(|e| e.card_id != event.card_id);
        self.entries.push(ActiveTrip {
            card_id: event.card_id.clone(),
            event,
            last_seen: now,
        });
    }

    /// 取出并移除指定卡号的未完成行程。
    pub fn take(&mut self, card_id: &str, now: u64) -> Option<TapEvent> {
        self.purge_expired(now);
        if let Some(pos) = self.entries.iter().position(|e| e.card_id == card_id) {
            return Some(self.entries.swap_remove(pos).event);
        }
        None
    }

    /// 清理过期行程。
    fn purge_expired(&mut self, now: u64) {
        let ttl = self.ttl_secs as u64;
        self.entries.retain(|e| now.saturating_sub(e.last_seen) <= ttl);
    }
}
