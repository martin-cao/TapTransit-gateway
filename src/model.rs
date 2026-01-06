use std::fmt;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapType {
    TapIn,
    TapOut,
}

impl TapType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TapType::TapIn => "tap_in",
            TapType::TapOut => "tap_out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapMode {
    SingleTap,
    TapInOut,
}

impl TapMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TapMode::SingleTap => "single_tap",
            TapMode::TapInOut => "tap_in_out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FareType {
    Uniform,
    Segment,
    Distance,
}

impl FareType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FareType::Uniform => "uniform",
            FareType::Segment => "segment",
            FareType::Distance => "distance",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassengerTone {
    Normal,
    Student,
    Elder,
    Disabled,
    Error,
}

impl PassengerTone {
    pub fn css_class(&self) -> &'static str {
        match self {
            PassengerTone::Normal => "tone-normal",
            PassengerTone::Student => "tone-student",
            PassengerTone::Elder => "tone-elder",
            PassengerTone::Disabled => "tone-disabled",
            PassengerTone::Error => "tone-error",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PassengerTone::Normal => "普通票",
            PassengerTone::Student => "学生票",
            PassengerTone::Elder => "长者票",
            PassengerTone::Disabled => "残障票",
            PassengerTone::Error => "异常",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GatewaySettings {
    pub gateway_id: String,
    pub reader_id: u16,
    pub debounce_window_secs: u32,
    pub tap_cache_max: usize,
    pub config_ttl_secs: u32,
    pub blacklist_ttl_secs: u32,
    pub active_trip_ttl_secs: u32,
    pub batch_size: usize,
}

impl GatewaySettings {
    pub fn with_gateway_id(id: impl Into<String>) -> Self {
        Self {
            gateway_id: id.into(),
            reader_id: 1,
            debounce_window_secs: 2,
            tap_cache_max: 512,
            config_ttl_secs: 300,
            blacklist_ttl_secs: 300,
            active_trip_ttl_secs: 3600,
            batch_size: 50,
        }
    }
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self::with_gateway_id("gateway-unknown")
    }
}

#[derive(Clone, Debug)]
pub struct StationConfig {
    pub id: u16,
    pub name: String,
    pub sequence: u16,
    pub zone_id: Option<u16>,
    pub is_transfer: bool,
}

#[derive(Clone, Debug)]
pub struct FareRule {
    pub base_price: f32,
    pub fare_type: Option<String>,
    pub segment_count: Option<u16>,
    pub extra_price: Option<f32>,
    pub start_station: Option<u16>,
    pub end_station: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct RouteConfig {
    pub route_id: u16,
    pub route_name: String,
    pub fare_type: FareType,
    pub tap_mode: TapMode,
    pub max_fare: Option<f32>,
    pub stations: Vec<StationConfig>,
    pub fares: Vec<FareRule>,
}

#[derive(Clone, Debug)]
pub struct TapEvent {
    pub record_id: String,
    pub card_id: String,
    pub route_id: u16,
    pub station_id: u16,
    pub station_name: String,
    pub tap_type: TapType,
    pub tap_time: u64,
    pub gateway_id: String,
}

impl TapEvent {
    pub fn new(
        record_id: String,
        card_id: String,
        route_id: u16,
        station_id: u16,
        station_name: String,
        tap_type: TapType,
        tap_time: u64,
        gateway_id: String,
    ) -> Self {
        Self {
            record_id,
            card_id,
            route_id,
            station_id,
            station_name,
            tap_type,
            tap_time,
            gateway_id,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UploadRecord {
    pub record_id: String,
    pub card_id: String,
    pub route_id: Option<u16>,
    pub board_time: String,
    pub alight_time: Option<String>,
    pub board_station_id: Option<u16>,
    pub board_station: Option<String>,
    pub alight_station_id: Option<u16>,
    pub alight_station: Option<String>,
    pub gateway_id: Option<String>,
}

impl UploadRecord {
    pub fn from_tap_in(event: &TapEvent) -> Self {
        Self {
            record_id: event.record_id.clone(),
            card_id: event.card_id.clone(),
            route_id: Some(event.route_id),
            board_time: format_time(event.tap_time),
            alight_time: None,
            board_station_id: Some(event.station_id),
            board_station: Some(event.station_name.clone()),
            alight_station_id: None,
            alight_station: None,
            gateway_id: Some(event.gateway_id.clone()),
        }
    }

    pub fn from_tap_out(
        event: &TapEvent,
        board_time: u64,
        board_station_id: Option<u16>,
        board_station: Option<String>,
    ) -> Self {
        Self {
            record_id: event.record_id.clone(),
            card_id: event.card_id.clone(),
            route_id: Some(event.route_id),
            board_time: format_time(board_time),
            alight_time: Some(format_time(event.tap_time)),
            board_station_id,
            board_station,
            alight_station_id: Some(event.station_id),
            alight_station: Some(event.station_name.clone()),
            gateway_id: Some(event.gateway_id.clone()),
        }
    }
}

fn format_time(epoch_secs: u64) -> String {
    epoch_secs.to_string()
}

impl RouteConfig {
    pub fn standard_fare(&self) -> Option<f32> {
        let mut best: Option<f32> = None;
        for fare in &self.fares {
            let base = fare.base_price;
            if base <= 0.0 {
                continue;
            }
            best = Some(match best {
                Some(current) => current.min(base),
                None => base,
            });
        }
        best
    }
}

impl fmt::Display for TapEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} route={} station={}({}) type={}",
            self.record_id,
            self.card_id,
            self.route_id,
            self.station_id,
            self.station_name,
            self.tap_type.as_str()
        )
    }
}
