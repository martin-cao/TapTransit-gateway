use crate::cache::{
    ActiveTripCache, BlacklistCache, CardStateSnapshotCache, ConfigCache, TapDebounce, TapEventCache,
};
use crate::card_data::{decode_uid_hex, CardData, CardStatus, CARD_DATA_BLOCK_COUNT, CARD_DATA_BLOCK_START, CARD_DATA_LEN};
use crate::model::{
    CardRegistration, CardStateSnapshot, Direction, GatewaySettings, PassengerTone, RouteConfig,
    TapEvent, TapMode, TapType, UploadRecord,
};
use crate::serial::{CardAck, CardDetected, CardWriteRequest, CardWriteResult};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// 卡片缓存过期时间（10 分钟）。
const CARD_CACHE_TTL_MS: u64 = 10 * 60 * 1000;
const RECHARGE_MODE_TTL_MS: u64 = 60 * 1000;
const REGISTER_MODE_TTL_MS: u64 = 60 * 1000;
// 乘客屏消息显示时长（毫秒）。
// “调高一点”：默认成功提示 2s；错误/写卡失败/注册充值提示 3s。
const PASSENGER_MSG_TTL_OK_MS: u64 = 2000;
const PASSENGER_MSG_TTL_ACTION_MS: u64 = 3000;
const PASSENGER_MSG_TTL_ERROR_MS: u64 = 3000;
const DEFAULT_REGISTER_BALANCE_CENTS: u32 = 0;
const MAX_RECHARGE_CENTS: u32 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteContext {
    TapIn,
    TapOut,
    Recharge,
    Register,
    Blacklist,
}

#[derive(Clone, Debug)]
pub struct RechargeMode {
    pub amount_cents: u32,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug)]
pub struct RegisterMode {
    pub expires_at_ms: u64,
}

/// 卡片缓存的用户画像（票种/状态/优惠）。
#[derive(Clone, Debug)]
pub struct CachedCardProfile {
    pub card_type: Option<String>,
    pub status: Option<String>,
    pub discount_rate: Option<f32>,
    pub discount_amount: Option<f32>,
    pub balance_cents: Option<u32>,
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
    pub last_card_data_len: usize,
    pub last_card_data_prefix_hex: Option<String>,
    pub last_card_data_error: Option<String>,
    pub last_tap_nonce: u32,
    pub last_message_deadline_ms: u64,
    pub last_passenger_tone: PassengerTone,
    pub last_passenger_message: String,
    pub last_fare_base: Option<f32>,
    pub last_fare: Option<f32>,
    pub last_fare_label: String,
    // 最近一次从“卡内数据”读到的余额（不经过后端校验）。
    // 注意：如果本次刷卡卡内数据无效（读不出/UID 不匹配），这里会是 None。
    pub last_balance_cents: Option<u32>,
    pub last_tap_type: Option<TapType>,
    pub card_cache: HashMap<String, CachedCardProfile>,
    pub card_state_cache: CardStateSnapshotCache,
    pub recharge_mode: Option<RechargeMode>,
    pub register_mode: Option<RegisterMode>,
    last_write_context: Option<WriteContext>,
    // 保存最近一次写卡时的新余额，用于在写卡成功后更新显示
    last_written_balance_cents: Option<u32>,
    record_seq: u32,
}

/// 处理一次刷卡后的决策输出（ACK + 上报记录）。
pub struct Decision {
    pub ack: CardAck,
    pub event: Option<TapEvent>,
    pub upload_record: Option<UploadRecord>,
    pub write_request: Option<CardWriteRequest>,
    pub registration: Option<CardRegistration>,
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
        let tap_cache_max = settings.tap_cache_max;
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
            last_card_data_len: 0,
            last_card_data_prefix_hex: None,
            last_card_data_error: None,
            last_tap_nonce: 0,
            last_message_deadline_ms: 0,
            last_passenger_tone: PassengerTone::Normal,
            last_passenger_message: "等待刷卡".to_string(),
            last_fare_base: None,
            last_fare: None,
            last_fare_label: "应付".to_string(),
            last_balance_cents: None,
            last_tap_type: None,
            card_cache: HashMap::new(),
            card_state_cache: CardStateSnapshotCache::new(tap_cache_max),
            recharge_mode: None,
            register_mode: None,
            last_write_context: None,
            last_written_balance_cents: None,
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

    pub fn set_recharge_mode(&mut self, amount_cents: u32, now_ms: u64) {
        if amount_cents == 0 || amount_cents > MAX_RECHARGE_CENTS {
            return;
        }
        self.register_mode = None;
        self.recharge_mode = Some(RechargeMode {
            amount_cents,
            expires_at_ms: now_ms.saturating_add(RECHARGE_MODE_TTL_MS),
        });
    }

    pub fn clear_recharge_mode(&mut self) {
        self.recharge_mode = None;
    }

    pub fn set_register_mode(&mut self, now_ms: u64) {
        self.recharge_mode = None;
        self.register_mode = Some(RegisterMode {
            expires_at_ms: now_ms.saturating_add(REGISTER_MODE_TTL_MS),
        });
    }

    pub fn clear_register_mode(&mut self) {
        self.register_mode = None;
    }

    fn refresh_modes(&mut self, now_ms: u64) {
        if let Some(mode) = &self.recharge_mode {
            if now_ms >= mode.expires_at_ms {
                self.recharge_mode = None;
            }
        }
        if let Some(mode) = &self.register_mode {
            if now_ms >= mode.expires_at_ms {
                self.register_mode = None;
            }
        }
    }

    pub fn handle_write_result(&mut self, result: CardWriteResult, now_ms: u64) {
        let context = self.last_write_context.take();
        if result.result == 1 {
            // 写卡成功，更新显示的余额为刚刚写入的新余额
            if let Some(new_balance) = self.last_written_balance_cents.take() {
                self.last_balance_cents = Some(new_balance);
            }
            if matches!(context, Some(WriteContext::Recharge)) {
                self.recharge_mode = None;
            }
            return;
        }
        // 写卡失败，清除保存的余额
        self.last_written_balance_cents = None;
        let message = match context {
            Some(WriteContext::Recharge) => "充值写卡失败",
            Some(WriteContext::Register) => "注册写卡失败",
            Some(WriteContext::Blacklist) => "冻结写卡失败",
            _ => "写卡失败",
        };
        self.last_passenger_tone = PassengerTone::Error;
        self.last_passenger_message = message.to_string();
        self.last_message_deadline_ms = now_ms.saturating_add(PASSENGER_MSG_TTL_ERROR_MS);
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
        let now_ms = current_epoch_millis();
        self.refresh_modes(now_ms);
        self.last_tap_nonce = self.last_tap_nonce.wrapping_add(1);
        let card_id = detected.card_id.clone();
        self.last_card_id = card_id.clone();
        self.last_card_data_len = detected.card_data.len();
        self.last_card_data_prefix_hex = if detected.card_data.is_empty() {
            None
        } else {
            Some(hex_prefix(&detected.card_data, 16))
        };
        self.last_card_data_error = None;

        if !self.debounce.allow(&detected.card_id, now) {
            return self.reject_card("刷卡过快", now_ms);
        }

        let uid = decode_uid_hex(&card_id);
        let mut card_data = if detected.card_data.len() >= CARD_DATA_LEN {
            match CardData::from_bytes_verbose(&detected.card_data) {
                Ok(data) => Some(data),
                Err(err) => {
                    self.last_card_data_error = Some(err.as_str().to_string());
                    None
                }
            }
        } else {
            self.last_card_data_error = Some("short_card_data".to_string());
            None
        };
        if let (Some(uid), Some(ref data)) = (uid, card_data.as_ref()) {
            if data.uid != uid {
                self.last_card_data_error = Some("uid_mismatch".to_string());
                card_data = None;
            }
        }

        // 余额展示以“读到的卡内数据”为准（不使用后端补全的数据）。
        self.last_balance_cents = card_data.as_ref().map(|data| data.balance_cents);

        if self.blacklist_cache.is_blocked(&detected.card_id) {
            return self.reject_blacklisted(&card_id, card_data, now_ms);
        }

        if self.register_mode.is_some() {
            return self.handle_register(card_id, uid, card_data, now_ms);
        }

        if self.recharge_mode.is_some() {
            return self.handle_recharge(card_id, card_data, now_ms);
        }

        let mut card_data = match card_data {
            Some(data) => data,
            None => {
                // 若读到的卡内数据无效，但后端已存在该卡，则允许按后端余额进行“补全”。
                // 这能修复“数据库已注册但仍提示未注册”的情况（例如卡片未写入/数据损坏/读错块）。
                let Some(uid) = uid else {
                    return self.reject_card("卡未注册", now_ms);
                };
                let Some(profile) = self.cached_profile(&card_id, now_ms) else {
                    return self.reject_card("卡未注册", now_ms);
                };
                if let Some(status) = profile.status.as_deref() {
                    if status == "blocked" {
                        return self.reject_card("卡已冻结", now_ms);
                    }
                    if status == "lost" {
                        return self.reject_card("卡已挂失", now_ms);
                    }
                }
                let mut data = CardData::new(uid);
                if let Some(balance_cents) = profile.balance_cents {
                    data.balance_cents = balance_cents;
                }
                data
            }
        };

        if card_data.status == CardStatus::Blocked {
            return self.reject_card("卡已冻结", now_ms);
        }

        let tap_mode = self
            .config_cache
            .route
            .as_ref()
            .map(|cfg| cfg.tap_mode)
            .unwrap_or(TapMode::SingleTap);

        let mut board_event: Option<TapEvent> = None;
        let mut removed_trip: Option<TapEvent> = None;
        let tap_type = match tap_mode {
            TapMode::SingleTap => TapType::TapIn,
            TapMode::TapInOut => {
                if let Some(prev) = self.active_trips.take(&card_id, now) {
                    removed_trip = Some(prev.clone());
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

        self.last_passenger_tone = PassengerTone::Normal;
        let mut upload_record = None;
        let mut write_request = None;
        let standard_fare = self.standard_fare();
        match (tap_mode, tap_type) {
            (TapMode::SingleTap, TapType::TapIn) => {
                upload_record = Some(UploadRecord::from_tap_in(&event));
                self.last_fare_base = standard_fare;
                self.last_fare = standard_fare;
                self.last_fare_label = "应付".to_string();
                self.apply_cached_profile(&card_id, now_ms);
                let fare_cents = self.fare_to_cents();
                if !self.apply_balance(&mut card_data, fare_cents) {
                    return self.reject_card("余额不足", now_ms);
                }
                self.update_last_trip(&mut card_data, None, Some(event.station_id));
                card_data.status = CardStatus::Idle;
                card_data.entry_station_id = None;
                write_request = Some(self.build_write_request(&card_id, &card_data, WriteContext::TapIn));
                self.push_card_snapshot(&card_id, &card_data, "tap_in", now_ms);
            }
            (TapMode::TapInOut, TapType::TapIn) => {
                self.active_trips.insert(event.clone(), now);
                upload_record = Some(UploadRecord::from_tap_in(&event));
                let fare = self.estimate_trip_fare(event.station_id, event.station_id);
                self.last_fare_base = fare.or(standard_fare);
                self.last_fare = fare.or(standard_fare);
                self.last_fare_label = "起步价".to_string();
                self.apply_cached_profile(&card_id, now_ms);
                card_data.status = CardStatus::InTrip;
                card_data.entry_station_id = Some(event.station_id);
                write_request = Some(self.build_write_request(&card_id, &card_data, WriteContext::TapIn));
                self.push_card_snapshot(&card_id, &card_data, "tap_in", now_ms);
            }
            (TapMode::TapInOut, TapType::TapOut) => {
                if let Some(board) = board_event.as_ref() {
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
                self.apply_cached_profile(&card_id, now_ms);
                let fare_cents = self.fare_to_cents();
                if !self.apply_balance(&mut card_data, fare_cents) {
                    if let Some(prev) = removed_trip {
                        self.active_trips.insert(prev, now);
                    }
                    return self.reject_card("余额不足", now_ms);
                }
                let board_station = board_event.as_ref().map(|e| e.station_id);
                self.update_last_trip(&mut card_data, board_station, Some(event.station_id));
                card_data.status = CardStatus::Idle;
                card_data.entry_station_id = None;
                write_request = Some(self.build_write_request(&card_id, &card_data, WriteContext::TapOut));
                self.push_card_snapshot(&card_id, &card_data, "tap_out", now_ms);
            }
            _ => {}
        }

        self.last_tap_type = Some(tap_type);

        if self.last_passenger_tone != PassengerTone::Error {
        self.last_passenger_message = "刷卡成功".to_string();
        }
        self.last_message_deadline_ms = now_ms.saturating_add(PASSENGER_MSG_TTL_OK_MS);

        Decision {
            ack: CardAck::accepted(),
            event: Some(event),
            upload_record,
            write_request,
            registration: None,
        }
    }

    fn handle_register(
        &mut self,
        card_id: String,
        uid: Option<[u8; 4]>,
        card_data: Option<CardData>,
        now_ms: u64,
    ) -> Decision {
        let uid = match uid {
            Some(uid) => uid,
            None => return self.reject_card("卡号异常", now_ms),
        };
        if card_data.is_some() {
            return self.reject_card("卡已注册", now_ms);
        }

        let mut new_data = CardData::new(uid);
        new_data.balance_cents = DEFAULT_REGISTER_BALANCE_CENTS;
        new_data.status = CardStatus::Idle;
        let write_request = self.build_write_request(&card_id, &new_data, WriteContext::Register);
        let registration = CardRegistration {
            card_id: card_id.clone(),
            balance_cents: new_data.balance_cents,
            status: "active".to_string(),
            registered_at: now_ms,
            gateway_id: self.settings.gateway_id.clone(),
        };
        self.push_card_snapshot(&card_id, &new_data, "register", now_ms);
        // 注册时本次刷卡不存在“已读出的卡内余额”，保持 None。
        self.last_balance_cents = None;
        self.last_passenger_tone = PassengerTone::Normal;
        self.last_passenger_message = "注册成功".to_string();
        self.last_message_deadline_ms = now_ms.saturating_add(PASSENGER_MSG_TTL_ACTION_MS);
        Decision {
            ack: CardAck::accepted(),
            event: None,
            upload_record: None,
            write_request: Some(write_request),
            registration: Some(registration),
        }
    }

    fn handle_recharge(
        &mut self,
        card_id: String,
        card_data: Option<CardData>,
        now_ms: u64,
    ) -> Decision {
        let Some(mode) = self.recharge_mode.clone() else {
            return self.reject_card("充值模式已结束", now_ms);
        };
        let mut card_data = match card_data {
            Some(data) => data,
            None => {
                // 充值时也允许使用后端已注册卡的余额进行补全，避免“已注册但无法充值”。
                let Some(uid) = decode_uid_hex(&card_id) else {
                    return self.reject_card("卡未注册", now_ms);
                };
                let Some(profile) = self.cached_profile(&card_id, now_ms) else {
                    return self.reject_card("卡未注册", now_ms);
                };
                if let Some(status) = profile.status.as_deref() {
                    if status == "blocked" {
                        return self.reject_card("卡已冻结", now_ms);
                    }
                    if status == "lost" {
                        return self.reject_card("卡已挂失", now_ms);
                    }
                }
                let mut data = CardData::new(uid);
                if let Some(balance_cents) = profile.balance_cents {
                    data.balance_cents = balance_cents;
                }
                data
            }
        };
        // 充值展示的余额以“刷卡时读到的卡内余额”为准。
        self.last_balance_cents = Some(card_data.balance_cents);

        if card_data.status != CardStatus::Idle {
            return self.reject_card("卡状态异常", now_ms);
        }
        card_data.balance_cents = card_data.balance_cents.saturating_add(mode.amount_cents);
        let write_request = self.build_write_request(&card_id, &card_data, WriteContext::Recharge);
        self.push_card_snapshot(&card_id, &card_data, "recharge", now_ms);
        self.last_passenger_tone = PassengerTone::Normal;
        self.last_passenger_message = "充值成功".to_string();
        self.last_message_deadline_ms = now_ms.saturating_add(PASSENGER_MSG_TTL_ACTION_MS);
        Decision {
            ack: CardAck::accepted(),
            event: None,
            upload_record: None,
            write_request: Some(write_request),
            registration: None,
        }
    }

    fn reject_card(&mut self, message: &str, now_ms: u64) -> Decision {
        self.reject_with_write(message, None, now_ms)
    }

    fn reject_with_write(
        &mut self,
        message: &str,
        write_request: Option<CardWriteRequest>,
        now_ms: u64,
    ) -> Decision {
        self.last_passenger_tone = PassengerTone::Error;
        self.last_passenger_message = message.to_string();
        self.last_fare_base = None;
        self.last_fare = None;
        self.last_message_deadline_ms = now_ms.saturating_add(PASSENGER_MSG_TTL_ERROR_MS);
        Decision {
            ack: CardAck::rejected(),
            event: None,
            upload_record: None,
            write_request,
            registration: None,
        }
    }

    fn reject_blacklisted(
        &mut self,
        card_id: &str,
        card_data: Option<CardData>,
        now_ms: u64,
    ) -> Decision {
        let mut write_request = None;
        if let Some(mut data) = card_data {
            if data.status != CardStatus::Blocked {
                data.status = CardStatus::Blocked;
                data.entry_station_id = None;
                write_request = Some(self.build_write_request(card_id, &data, WriteContext::Blacklist));
                self.push_card_snapshot(card_id, &data, "blacklist", now_ms);
            }
        }
        self.reject_with_write("卡已冻结", write_request, now_ms)
    }

    fn fare_to_cents(&self) -> u32 {
        self.last_fare
            .or(self.last_fare_base)
            .map(|fare| (fare * 100.0).round().max(0.0) as u32)
            .unwrap_or(0)
    }

    fn apply_balance(&mut self, card_data: &mut CardData, fare_cents: u32) -> bool {
        if fare_cents == 0 {
            return true;
        }
        if card_data.balance_cents < fare_cents {
            return false;
        }
        card_data.balance_cents = card_data.balance_cents.saturating_sub(fare_cents);
        true
    }

    fn update_last_trip(
        &self,
        card_data: &mut CardData,
        board_station_id: Option<u16>,
        alight_station_id: Option<u16>,
    ) {
        card_data.last_route_id = Some(self.route_state.route_id);
        card_data.last_direction = Some(self.route_state.direction);
        card_data.last_board_station_id = board_station_id;
        card_data.last_alight_station_id = alight_station_id;
    }

    fn build_write_request(
        &mut self,
        card_id: &str,
        card_data: &CardData,
        context: WriteContext,
    ) -> CardWriteRequest {
        self.last_write_context = Some(context);
        // 保存写入的新余额，以便写卡成功后更新显示
        self.last_written_balance_cents = Some(card_data.balance_cents);

        // 写卡块大小为 16B；当前卡数据格式固定 32B（2 个 block）。
        // 这些断言用于防止未来改动导致写卡长度/块数不一致。
        debug_assert_eq!(CARD_DATA_BLOCK_COUNT as usize * 16, CARD_DATA_LEN);
        let bytes = card_data.to_bytes();
        debug_assert_eq!(bytes.len(), CARD_DATA_LEN);

        CardWriteRequest {
            card_id: card_id.to_string(),
            card_data: bytes.to_vec(),
            block_start: CARD_DATA_BLOCK_START,
            block_count: CARD_DATA_BLOCK_COUNT,
        }
    }

    fn push_card_snapshot(&mut self, card_id: &str, card_data: &CardData, source: &str, now_ms: u64) {
        let snapshot = CardStateSnapshot {
            card_id: card_id.to_string(),
            balance_cents: card_data.balance_cents,
            card_status: card_data.status.as_str().to_string(),
            entry_station_id: card_data.entry_station_id,
            last_route_id: card_data.last_route_id,
            last_direction: card_data
                .last_direction
                .map(|direction| direction.as_str().to_string()),
            last_board_station_id: card_data.last_board_station_id,
            last_alight_station_id: card_data.last_alight_station_id,
            updated_at: now_ms,
            source: source.to_string(),
        };
        let _ = self.card_state_cache.push(snapshot);
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
        balance_cents: Option<u32>,
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
                balance_cents,
                updated_at_ms: now_ms,
            },
        );
    }

    fn cached_profile(&self, card_id: &str, now_ms: u64) -> Option<CachedCardProfile> {
        let Some(profile) = self.card_cache.get(card_id).cloned() else {
            return None;
        };
        if now_ms.saturating_sub(profile.updated_at_ms) > CARD_CACHE_TTL_MS {
            return None;
        }
        Some(profile)
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

fn hex_prefix(bytes: &[u8], max_len: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let take_len = core::cmp::min(bytes.len(), max_len);
    let mut out = String::with_capacity(take_len * 2);
    for &b in &bytes[..take_len] {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
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
