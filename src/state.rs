use crate::cache::{ActiveTripCache, BlacklistCache, ConfigCache, TapDebounce, TapEventCache};
use crate::model::{
    Direction, GatewaySettings, PassengerTone, RouteConfig, TapEvent, TapMode, TapType, UploadRecord,
};
use crate::serial::{CardAck, CardDetected};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// 卡片缓存过期时间（10 分钟）。
const CARD_CACHE_TTL_MS: u64 = 10 * 60 * 1000;

/// 卡片缓存的用户画像（票种/状态/优惠）。
#[derive(Clone, Debug)]
pub struct CachedCardProfile {
    pub card_type: Option<String>,
    pub status: Option<String>,
    pub discount_rate: Option<f32>,
    pub discount_amount: Option<f32>,
    pub updated_at_ms: u64,
}

/// 当前线路/站点/方向状态。
#[derive(Clone, Debug)]
pub struct RouteState {
    pub route_id: u16,
    pub station_id: u16,
    pub station_name: String,
    pub direction: Direction,
}

impl RouteState {
    /// 构造线路状态。
    pub fn new(route_id: u16, station_id: u16, station_name: String, direction: Direction) -> Self {
        Self {
            route_id,
            station_id,
            station_name,
            direction,
        }
    }
}

/// 网关全局状态（缓存、健康、上次刷卡等）。
pub struct GatewayState {
    pub settings: GatewaySettings,
    pub route_state: RouteState,
    pub config_cache: ConfigCache,
    pub blacklist_cache: BlacklistCache,
    pub tap_cache: TapEventCache,
    pub debounce: TapDebounce,
    pub active_trips: ActiveTripCache,
    pub wifi_connected: bool,
    pub backend_reachable: bool,
    pub backend_base_url: String,
    pub last_card_id: String,
    pub last_tap_nonce: u32,
    pub last_message_deadline_ms: u64,
    pub last_passenger_tone: PassengerTone,
    pub last_passenger_message: String,
    pub last_fare_base: Option<f32>,
    pub last_fare: Option<f32>,
    pub last_fare_label: String,
    pub last_tap_type: Option<TapType>,
    pub card_cache: HashMap<String, CachedCardProfile>,
    record_seq: u32,
}

/// 处理一次刷卡后的决策输出（ACK + 上报记录）。
pub struct Decision {
    pub ack: CardAck,
    pub event: Option<TapEvent>,
    pub upload_record: Option<UploadRecord>,
}

impl GatewayState {
    /// 构造完整状态（供测试/初始化用）。
    pub fn new(
        settings: GatewaySettings,
        route_state: RouteState,
        config_cache: ConfigCache,
        blacklist_cache: BlacklistCache,
        tap_cache: TapEventCache,
        debounce: TapDebounce,
        active_trips: ActiveTripCache,
    ) -> Self {
        Self {
            settings,
            route_state,
            config_cache,
            blacklist_cache,
            tap_cache,
            debounce,
            active_trips,
            wifi_connected: false,
            backend_reachable: false,
            backend_base_url: String::new(),
            last_card_id: String::new(),
            last_tap_nonce: 0,
            last_message_deadline_ms: 0,
            last_passenger_tone: PassengerTone::Normal,
            last_passenger_message: "等待刷卡".to_string(),
            last_fare_base: None,
            last_fare: None,
            last_fare_label: "应付".to_string(),
            last_tap_type: None,
            card_cache: HashMap::new(),
            record_seq: 0,
        }
    }

    pub fn bootstrap(settings: GatewaySettings) -> Self {
        // 启动时默认线路未设置
        let route_state = RouteState::new(0, 0, "未设置".to_string(), Direction::Up);
        Self::new(
            settings.clone(),
            route_state,
            ConfigCache::new(settings.config_ttl_secs),
            BlacklistCache::new(settings.blacklist_ttl_secs),
            TapEventCache::new(settings.tap_cache_max),
            TapDebounce::new(settings.debounce_window_secs, 128),
            ActiveTripCache::new(settings.active_trip_ttl_secs),
        )
    }

    pub fn update_route(
        &mut self,
        route_id: u16,
        station_id: u16,
        station_name: String,
        direction: Direction,
    ) {
        // 更新线路与站点信息
        self.route_state.route_id = route_id;
        self.route_state.station_id = station_id;
        self.route_state.station_name = station_name;
        self.route_state.direction = direction;
    }

    pub fn update_route_config(&mut self, config: RouteConfig, now: u64) {
        let route_id = config.route_id;
        let station_ids: Vec<u16> = config.stations.iter().map(|s| s.id).collect();
        self.config_cache.update(config.clone(), now);

        // 若当前站点不在新配置中，则重置到最小序号站点
        if self.route_state.route_id != route_id || !station_ids.contains(&self.route_state.station_id)
        {
            if let Some(station) = config.stations.iter().min_by_key(|s| s.sequence) {
                self.route_state.route_id = route_id;
                self.route_state.station_id = station.id;
                self.route_state.station_name = station.name.clone();
            } else {
                self.route_state.route_id = route_id;
                self.route_state.station_id = 0;
                self.route_state.station_name = "未设置".to_string();
            }
        } else if let Some(station) = config
            .stations
            .iter()
            .find(|s| s.id == self.route_state.station_id)
        {
            self.route_state.station_name = station.name.clone();
        }
    }

    pub fn set_direction(&mut self, direction: Direction) {
        self.route_state.direction = direction;
    }

    /// 更新黑名单缓存。
    pub fn update_blacklist(&mut self, cards: Vec<String>, now: u64) {
        self.blacklist_cache.replace(cards, now);
    }

    /// 更新后端基础 URL。
    pub fn update_backend_base_url(&mut self, url: String) {
        self.backend_base_url = url;
    }

    /// 更新网络健康状态。
    pub fn update_health(&mut self, wifi_connected: Option<bool>, backend_reachable: Option<bool>) {
        if let Some(connected) = wifi_connected {
            self.wifi_connected = connected;
        }
        if let Some(reachable) = backend_reachable {
            self.backend_reachable = reachable;
        }
    }

    pub fn set_station_by_id(&mut self, station_id: u16) -> bool {
        // 根据站点 ID 直接跳转
        let Some(cfg) = self.config_cache.route.as_ref() else {
            return false;
        };
        if let Some(station) = cfg.stations.iter().find(|s| s.id == station_id) {
            self.route_state.station_id = station.id;
            self.route_state.station_name = station.name.clone();
            return true;
        }
        false
    }

    pub fn step_station(&mut self, forward: bool) -> bool {
        // 按顺序切换站点（上一站/下一站）
        let Some(cfg) = self.config_cache.route.as_ref() else {
            return false;
        };
        let mut stations = cfg.stations.clone();
        stations.sort_by_key(|s| s.sequence);
        let Some(pos) = stations.iter().position(|s| s.id == self.route_state.station_id) else {
            return false;
        };
        let next = if forward {
            pos.saturating_add(1)
        } else if pos == 0 {
            0
        } else {
            pos - 1
        };
        let Some(station) = stations.get(next) else {
            return false;
        };
        self.route_state.station_id = station.id;
        self.route_state.station_name = station.name.clone();
        true
    }

    pub fn handle_card_detected(&mut self, detected: CardDetected, now: u64) -> Decision {
        // 生成刷卡事件并决定是否上报
        let now_ms = current_epoch_millis();
        self.last_tap_nonce = self.last_tap_nonce.wrapping_add(1);
        let card_id = detected.card_id.clone();
        self.last_card_id = card_id.clone();
        // 防抖：刷卡过快直接拒绝
        if !self.debounce.allow(&detected.card_id, now) {
            self.last_passenger_tone = PassengerTone::Error;
            self.last_passenger_message = "刷卡过快".to_string();
            self.last_fare_base = None;
            self.last_fare = None;
            self.last_message_deadline_ms = now_ms.saturating_add(1000);
            return Decision {
                ack: CardAck::rejected(),
                event: None,
                upload_record: None,
            };
        }

        if self.blacklist_cache.is_blocked(&detected.card_id) {
            self.last_passenger_tone = PassengerTone::Error;
            self.last_passenger_message = "卡已冻结".to_string();
            self.last_fare_base = None;
            self.last_fare = None;
            self.last_message_deadline_ms = now_ms.saturating_add(1000);
            return Decision {
                ack: CardAck::rejected(),
                event: None,
                upload_record: None,
            };
        }

        let tap_mode = self
            .config_cache
            .route
            .as_ref()
            .map(|cfg| cfg.tap_mode)
            .unwrap_or(TapMode::SingleTap);

        let mut board_event: Option<TapEvent> = None;
        let tap_type = match tap_mode {
            TapMode::SingleTap => TapType::TapIn,
            TapMode::TapInOut => {
                // 进出站：若存在未完成行程则判定为下车
                if let Some(prev) = self.active_trips.take(&card_id, now) {
                    board_event = Some(prev);
                    TapType::TapOut
                } else {
                    TapType::TapIn
                }
            }
        };

        let record_id = self.next_record_id(now);
        let event = TapEvent::new(
            record_id,
            card_id.clone(),
            self.route_state.route_id,
            self.route_state.station_id,
            self.route_state.station_name.clone(),
            tap_type,
            detected.tap_time,
            self.settings.gateway_id.clone(),
        );

        let mut upload_record = None;
        let standard_fare = self.standard_fare();
        match (tap_mode, tap_type) {
            (TapMode::SingleTap, TapType::TapIn) => {
                // 单次刷卡：上车即上报
                upload_record = Some(UploadRecord::from_tap_in(&event));
                self.last_fare_base = standard_fare;
                self.last_fare = standard_fare;
                self.last_fare_label = "应付".to_string();
            }
            (TapMode::TapInOut, TapType::TapIn) => {
                // 进站：先缓存行程
                self.active_trips.insert(event.clone(), now);
                upload_record = Some(UploadRecord::from_tap_in(&event));
                let fare = self.estimate_trip_fare(event.station_id, event.station_id);
                self.last_fare_base = fare.or(standard_fare);
                self.last_fare = fare.or(standard_fare);
                self.last_fare_label = "起步价".to_string();
            }
            (TapMode::TapInOut, TapType::TapOut) => {
                // 出站：合并行程并估算价格
                if let Some(board) = board_event {
                    upload_record = Some(UploadRecord::from_tap_out(
                        &event,
                        board.tap_time,
                        Some(board.station_id),
                        Some(board.station_name.clone()),
                    ));
                    let fare = self
                        .estimate_trip_fare(board.station_id, event.station_id)
                        .or(standard_fare);
                    self.last_fare_base = fare;
                    self.last_fare = fare;
                } else {
                    self.last_fare_base = standard_fare;
                    self.last_fare = standard_fare;
                }
                self.last_fare_label = "结算价".to_string();
            }
            _ => {}
        }
        self.last_tap_type = Some(tap_type);

        // 默认成功提示
        self.last_passenger_tone = PassengerTone::Normal;
        self.last_passenger_message = "刷卡成功".to_string();
        self.last_message_deadline_ms = now_ms.saturating_add(1000);
        self.apply_cached_profile(&card_id, now_ms);

        Decision {
            ack: CardAck::accepted(),
            event: Some(event),
            upload_record,
        }
    }

    /// 根据 UI/后端反馈更新提示音色。
    pub fn update_passenger_tone(&mut self, tone: PassengerTone) {
        if tone != PassengerTone::Error {
            self.last_passenger_tone = tone;
        }
    }

    /// 依据卡类型应用默认折扣策略（网关侧预估）。
    pub fn apply_card_discount(&mut self, card_type: &str) {
        let base = self.last_fare_base.or(self.last_fare);
        let Some(base) = base else {
            return;
        };
        let card_type = card_type.trim().to_lowercase();
        let (discount_rate, label): (f32, &str) = match card_type.as_str() {
            "student" => (0.50_f32, "学生优惠"),
            "elder" => (1.00_f32, "长者免费"),
            "disabled" => (1.00_f32, "残障免费"),
            _ => return,
        };
        let discount_rate = discount_rate.clamp(0.0_f32, 1.0_f32);
        let value = base * (1.0 - discount_rate);
        let discounted = round_currency(value);
        self.last_fare = Some(discounted);
        let _ = label;
        self.last_fare_label = self.discount_label().to_string();
    }

    /// 依据后端下发折扣策略应用票价修正。
    pub fn apply_card_discount_policy(
        &mut self,
        card_type: &str,
        discount_rate: Option<f32>,
        discount_amount: Option<f32>,
    ) {
        let base = self.last_fare_base.or(self.last_fare);
        let Some(base) = base else {
            return;
        };
        let card_type = card_type.trim().to_lowercase();
        let _ = card_type;
        let has_policy = discount_rate.is_some() || discount_amount.is_some();
        let mut discount = 0.0;
        if let Some(amount) = discount_amount {
            if amount > 0.0 {
                discount = amount;
            }
        }
        if discount == 0.0 {
            if let Some(rate) = discount_rate {
                if rate >= 0.0 {
                    let rate = rate.clamp(0.0, 1.0);
                    discount = base * rate;
                }
            }
        }
        if discount == 0.0 && !has_policy {
            self.apply_card_discount(&card_type);
            return;
        }
        if discount > base {
            discount = base;
        }
        let value = base - discount;
        let discounted = round_currency(value);
        self.last_fare = Some(discounted);
        self.last_fare_label = self.discount_label().to_string();
    }

    fn discount_label(&self) -> &'static str {
        let tap_mode = self
            .config_cache
            .route
            .as_ref()
            .map(|cfg| cfg.tap_mode)
            .unwrap_or(TapMode::SingleTap);
        if tap_mode == TapMode::TapInOut {
            match self.last_tap_type {
                Some(TapType::TapIn) => "优惠起步价",
                Some(TapType::TapOut) => "优惠结算价",
                None => "优惠票价",
            }
        } else {
            "优惠票价"
        }
    }

    /// 更新卡片缓存（LRU 简化策略）。
    pub fn update_card_cache(
        &mut self,
        card_id: String,
        card_type: Option<String>,
        status: Option<String>,
        discount_rate: Option<f32>,
        discount_amount: Option<f32>,
        now_ms: u64,
    ) {
        if self.card_cache.len() >= 256 && !self.card_cache.contains_key(&card_id) {
            if let Some((oldest_id, _)) = self
                .card_cache
                .iter()
                .min_by_key(|(_, profile)| profile.updated_at_ms)
                .map(|(id, profile)| (id.clone(), profile.clone()))
            {
                self.card_cache.remove(&oldest_id);
            }
        }
        self.card_cache.insert(
            card_id,
            CachedCardProfile {
                card_type,
                status,
                discount_rate,
                discount_amount,
                updated_at_ms: now_ms,
            },
        );
    }

    pub fn apply_cached_profile(&mut self, card_id: &str, now_ms: u64) {
        let Some(profile) = self.card_cache.get(card_id).cloned() else {
            return;
        };
        // 过期缓存直接忽略
        if now_ms.saturating_sub(profile.updated_at_ms) > CARD_CACHE_TTL_MS {
            return;
        }
        if let Some(status) = profile.status.as_deref() {
            if status == "blocked" {
                self.last_passenger_tone = PassengerTone::Error;
                self.last_passenger_message = "卡已冻结".to_string();
                self.last_fare_base = None;
                self.last_fare = None;
                return;
            }
            if status == "lost" {
                self.last_passenger_tone = PassengerTone::Error;
                self.last_passenger_message = "卡已挂失".to_string();
                self.last_fare_base = None;
                self.last_fare = None;
                return;
            }
        }
        if let Some(card_type) = profile.card_type.as_deref() {
            match card_type {
                "student" => self.last_passenger_tone = PassengerTone::Student,
                "elder" => self.last_passenger_tone = PassengerTone::Elder,
                "disabled" => self.last_passenger_tone = PassengerTone::Disabled,
                _ => {}
            }
            self.apply_card_discount_policy(
                card_type,
                profile.discount_rate,
                profile.discount_amount,
            );
        }
    }

    pub fn standard_fare(&self) -> Option<f32> {
        self.config_cache
            .route
            .as_ref()
            .and_then(|cfg| cfg.standard_fare())
            .map(round_currency)
    }

    /// 网关侧估算票价（用于即时提示，不作为最终结算）。
    fn estimate_trip_fare(&self, start_station_id: u16, end_station_id: u16) -> Option<f32> {
        let cfg = self.config_cache.route.as_ref()?;
        if start_station_id == 0 || end_station_id == 0 {
            return cfg.standard_fare().map(round_currency);
        }
        if let Some(rule) = cfg.fares.iter().find(|fare| {
            fare.start_station == Some(start_station_id) && fare.end_station == Some(end_station_id)
        }) {
            if rule.base_price > 0.0 {
                return Some(round_currency(rule.base_price));
            }
        }
        match cfg.fare_type {
            crate::model::FareType::Uniform => cfg.standard_fare().map(round_currency),
            crate::model::FareType::Segment | crate::model::FareType::Distance => {
                let start_seq = cfg
                    .stations
                    .iter()
                    .find(|s| s.id == start_station_id)
                    .map(|s| s.sequence)?;
                let end_seq = cfg
                    .stations
                    .iter()
                    .find(|s| s.id == end_station_id)
                    .map(|s| s.sequence)?;
                let diff = if start_seq >= end_seq {
                    start_seq - end_seq
                } else {
                    end_seq - start_seq
                };
                let base_rule = cfg.fares.iter().find(|fare| {
                    fare.start_station.unwrap_or(0) == 0 && fare.end_station.unwrap_or(0) == 0
                });
                let base_price = base_rule.map(|r| r.base_price).unwrap_or(0.0);
                if base_price <= 0.0 {
                    return cfg.standard_fare().map(round_currency);
                }
                let extra = base_rule.and_then(|r| r.extra_price).unwrap_or(0.0);
                let included = base_rule.and_then(|r| r.segment_count).unwrap_or(1);
                if diff <= included || extra <= 0.0 {
                    return Some(round_currency(base_price));
                }
                let extra_segments = diff.saturating_sub(included) as f32;
                Some(round_currency(base_price + extra * extra_segments))
            }
        }
    }

    fn next_record_id(&mut self, now: u64) -> String {
        // 生成幂等记录 ID
        let seq = self.record_seq;
        self.record_seq = self.record_seq.wrapping_add(1);
        format!("{}-{}-{}", self.settings.gateway_id, now, seq)
    }
}

/// 获取当前毫秒时间戳。
fn current_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 金额保留两位小数（四舍五入）。
fn round_currency(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}
