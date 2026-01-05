use crate::cache::{ActiveTripCache, BlacklistCache, ConfigCache, TapDebounce, TapEventCache};
use crate::model::{
    Direction, GatewaySettings, PassengerTone, RouteConfig, TapEvent, TapMode, TapType, UploadRecord,
};
use crate::serial::{CardAck, CardDetected};

#[derive(Clone, Debug)]
pub struct RouteState {
    pub route_id: u16,
    pub station_id: u16,
    pub station_name: String,
    pub direction: Direction,
}

impl RouteState {
    pub fn new(route_id: u16, station_id: u16, station_name: String, direction: Direction) -> Self {
        Self {
            route_id,
            station_id,
            station_name,
            direction,
        }
    }
}

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
    pub last_passenger_tone: PassengerTone,
    pub last_passenger_message: String,
    pub last_fare: Option<f32>,
    pub last_fare_label: String,
    record_seq: u32,
}

pub struct Decision {
    pub ack: CardAck,
    pub event: Option<TapEvent>,
    pub upload_record: Option<UploadRecord>,
}

impl GatewayState {
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
            last_passenger_tone: PassengerTone::Normal,
            last_passenger_message: "等待刷卡".to_string(),
            last_fare: None,
            last_fare_label: "应付".to_string(),
            record_seq: 0,
        }
    }

    pub fn bootstrap(settings: GatewaySettings) -> Self {
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
        self.route_state.route_id = route_id;
        self.route_state.station_id = station_id;
        self.route_state.station_name = station_name;
        self.route_state.direction = direction;
    }

    pub fn update_route_config(&mut self, config: RouteConfig, now: u64) {
        let route_id = config.route_id;
        let station_ids: Vec<u16> = config.stations.iter().map(|s| s.id).collect();
        self.config_cache.update(config.clone(), now);

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

    pub fn update_blacklist(&mut self, cards: Vec<String>, now: u64) {
        self.blacklist_cache.replace(cards, now);
    }

    pub fn update_backend_base_url(&mut self, url: String) {
        self.backend_base_url = url;
    }

    pub fn update_health(&mut self, wifi_connected: Option<bool>, backend_reachable: Option<bool>) {
        if let Some(connected) = wifi_connected {
            self.wifi_connected = connected;
        }
        if let Some(reachable) = backend_reachable {
            self.backend_reachable = reachable;
        }
    }

    pub fn set_station_by_id(&mut self, station_id: u16) -> bool {
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
        self.last_card_id = detected.card_id.clone();
        if !self.debounce.allow(&detected.card_id, now) {
            self.last_passenger_tone = PassengerTone::Error;
            self.last_passenger_message = "刷卡过快".to_string();
            self.last_fare = None;
            return Decision {
                ack: CardAck::rejected(),
                event: None,
                upload_record: None,
            };
        }

        if self.blacklist_cache.is_blocked(&detected.card_id) {
            self.last_passenger_tone = PassengerTone::Error;
            self.last_passenger_message = "卡已冻结".to_string();
            self.last_fare = None;
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
                if let Some(prev) = self.active_trips.take(&detected.card_id, now) {
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
            detected.card_id,
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
                upload_record = Some(UploadRecord::from_tap_in(&event));
                self.last_fare = standard_fare;
                self.last_fare_label = "应付".to_string();
            }
            (TapMode::TapInOut, TapType::TapIn) => {
                self.active_trips.insert(event.clone(), now);
                self.last_fare = standard_fare;
                self.last_fare_label = "起步价".to_string();
            }
            (TapMode::TapInOut, TapType::TapOut) => {
                if let Some(board) = board_event {
                    upload_record = Some(UploadRecord::from_tap_out(
                        &event,
                        board.tap_time,
                        Some(board.station_id),
                        Some(board.station_name.clone()),
                    ));
                }
                self.last_fare = standard_fare;
                self.last_fare_label = "结算价".to_string();
            }
            _ => {}
        }

        self.last_passenger_tone = PassengerTone::Normal;
        self.last_passenger_message = "刷卡成功".to_string();

        Decision {
            ack: CardAck::accepted(),
            event: Some(event),
            upload_record,
        }
    }

    pub fn update_passenger_tone(&mut self, tone: PassengerTone) {
        if tone != PassengerTone::Error {
            self.last_passenger_tone = tone;
        }
    }

    pub fn standard_fare(&self) -> Option<f32> {
        self.config_cache.route.as_ref().and_then(|cfg| cfg.max_fare)
    }

    fn next_record_id(&mut self, now: u64) -> String {
        let seq = self.record_seq;
        self.record_seq = self.record_seq.wrapping_add(1);
        format!("{}-{}-{}", self.settings.gateway_id, now, seq)
    }
}
