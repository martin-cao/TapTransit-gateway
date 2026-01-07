use std::fmt;

use serde::Serialize;

/// 刷卡类型（上车/下车）。
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

/// 刷卡模式（单次/进出站）。
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

/// 线路方向。
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

/// 计价类型。
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

/// 乘客提示音色/标签（用于 UI 或蜂鸣提示）。
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

/// 网关运行参数（可配置项）。
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
    /// 使用指定网关 ID 构建默认参数。
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

/// 站点配置（来自后端下发）。
#[derive(Clone, Debug)]
pub struct StationConfig {
    pub id: u16,
    pub name: String,
    pub sequence: u16,
    pub zone_id: Option<u16>,
    pub is_transfer: bool,
}

/// 票价规则（简化字段）。
#[derive(Clone, Debug)]
pub struct FareRule {
    pub base_price: f32,
    pub fare_type: Option<String>,
    pub segment_count: Option<u16>,
    pub extra_price: Option<f32>,
    pub start_station: Option<u16>,
    pub end_station: Option<u16>,
}

/// 线路配置（站点 + 票价 + 模式）。
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

/// 刷卡事件（网关内部事件模型）。
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
    /// 构造刷卡事件。
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

/// 上传到后端的记录结构体。
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
    /// 从 tap_in 事件构建上报记录。
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

    /// 从 tap_out 事件构建上报记录。
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

/// 将 epoch 秒转换为字符串（后端接受 string 时间）。
fn format_time(epoch_secs: u64) -> String {
    epoch_secs.to_string()
}

impl RouteConfig {
    /// 获取线路的基础票价（取最小非零值作为默认）。
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
    /// 便于日志输出的格式化展示。
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
