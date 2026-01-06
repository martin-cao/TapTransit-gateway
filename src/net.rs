use core::convert::TryInto;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::io::Write as _;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::modem::Modem;
use esp_idf_hal::sys::EspError;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::client::EspHttpConnection;
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use serde::Deserialize;

use crate::api::{BATCH_RECORDS_PATH, CARDS_PATH, CONFIG_PATH};
use crate::model::{
    FareRule, FareType, GatewaySettings, PassengerTone, RouteConfig, StationConfig, TapMode, UploadRecord,
};
use crate::state::GatewayState;
use crate::upload::BatchUpload;

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");
const BACKEND_BASE_URL: &str = env!("BACKEND_BASE_URL");

#[derive(Clone, Debug)]
pub enum NetCommand {
    SyncConfig { route_id: u16 },
    UploadNow,
    SetBackend { base_url: String },
    LookupCard { card_id: String },
}

#[derive(Debug)]
pub enum NetError {
    Io(EspIOError),
    Json(serde_json::Error),
    HttpStatus(u16),
    Api(String),
}

impl From<EspIOError> for NetError {
    fn from(err: EspIOError) -> Self {
        NetError::Io(err)
    }
}

impl From<EspError> for NetError {
    fn from(err: EspError) -> Self {
        NetError::Io(EspIOError::from(err))
    }
}

impl From<serde_json::Error> for NetError {
    fn from(err: serde_json::Error) -> Self {
        NetError::Json(err)
    }
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    message: Option<String>,
}

pub fn connect_wifi(modem: Modem) -> Result<BlockingWifi<EspWifi<'static>>, EspError> {
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take().ok();
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), nvs)?, sys_loop)?;

    let auth_method = if WIFI_PASS.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    let wifi_configuration: Configuration = Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        bssid: None,
        auth_method,
        password: WIFI_PASS.try_into().unwrap(),
        channel: None,
        ..Default::default()
    });

    wifi.set_configuration(&wifi_configuration)?;
    wifi.start()?;
    log::info!("Wi-Fi started");
    wifi.connect()?;
    log::info!("Wi-Fi connected to {}", WIFI_SSID);
    wifi.wait_netif_up()?;
    log::info!("Wi-Fi netif up");
    Ok(wifi)
}

pub fn spawn_network_loop(
    state: Arc<Mutex<GatewayState>>,
    upload_rx: Receiver<UploadRecord>,
    command_rx: Receiver<NetCommand>,
    settings: GatewaySettings,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer: Vec<UploadRecord> = Vec::with_capacity(settings.batch_size);
        let mut route_id: Option<u16> = None;
        let mut last_upload = Instant::now();
        let refresh_secs = settings
            .config_ttl_secs
            .min(settings.blacklist_ttl_secs) as u64;
        let mut last_sync = Instant::now()
            .checked_sub(Duration::from_secs(refresh_secs))
            .unwrap_or_else(Instant::now);
        loop {
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    NetCommand::SyncConfig { route_id: next_route } => {
                        route_id = Some(next_route);
                        if sync_config(&state, next_route) {
                            last_sync = Instant::now();
                        }
                    }
                    NetCommand::UploadNow => {
                        while let Ok(record) = upload_rx.try_recv() {
                            buffer.push(record);
                        }
                        if let Err(err) = flush_batch(&state, &mut buffer) {
                            log::warn!("Upload batch failed: {:?}", err);
                        }
                    }
                    NetCommand::SetBackend { base_url } => {
                        if let Ok(mut state) = state.lock() {
                            state.update_backend_base_url(base_url);
                        }
                    }
                    NetCommand::LookupCard { card_id } => {
                        let base_url = resolve_base_url(&state);
                        match fetch_card_profile(&base_url, &card_id) {
                            Ok(Some(profile)) => {
                                apply_card_profile(&state, &card_id, profile);
                            }
                            Ok(None) => {}
                            Err(err) => {
                                log::warn!("Card lookup failed: {:?}", err);
                            }
                        }
                    }
                }
            }

            if let Some(route_id) = route_id {
                if last_sync.elapsed() >= Duration::from_secs(refresh_secs) {
                    if sync_config(&state, route_id) {
                        last_sync = Instant::now();
                    }
                }
            }

            match upload_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(record) => {
                    buffer.push(record);
                    last_upload = Instant::now();
                    if buffer.len() >= settings.batch_size {
                        if let Err(err) = flush_batch(&state, &mut buffer) {
                            log::warn!("Upload batch failed: {:?}", err);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !buffer.is_empty() && last_upload.elapsed() >= Duration::from_secs(5) {
                        if let Err(err) = flush_batch(&state, &mut buffer) {
                            log::warn!("Upload batch failed: {:?}", err);
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    })
}

fn flush_batch(state: &Arc<Mutex<GatewayState>>, buffer: &mut Vec<UploadRecord>) -> Result<(), NetError> {
    if buffer.is_empty() {
        return Ok(());
    }
    let payload = BatchUpload::new(buffer.clone()).to_json_string();
    let base_url = resolve_base_url(state);
    let url = format!("{}{}", base_url, BATCH_RECORDS_PATH);
    let content_length = payload.len().to_string();
    let headers = [
        ("content-type", "application/json"),
        ("content-length", content_length.as_str()),
    ];

    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let mut request = client.request(Method::Post, &url, &headers)?;
    request.write_all(payload.as_bytes())?;
    request.flush()?;
    log::info!("Uploading batch to {}", url);
    let response = request.submit()?;
    let status = response.status();
    log::info!("Upload response status {}", status);
    if !(200..300).contains(&status) {
        update_backend_status(state, false);
        return Err(NetError::HttpStatus(status));
    }
    buffer.clear();
    if let Ok(mut state) = state.lock() {
        state.tap_cache.clear();
    }
    update_backend_status(state, true);
    Ok(())
}

fn sync_config(state: &Arc<Mutex<GatewayState>>, route_id: u16) -> bool {
    let now = current_epoch();
    let mut ok = false;
    let base_url = resolve_base_url(state);

    match fetch_route_config(&base_url, route_id) {
        Ok(config) => {
            if let Ok(mut state) = state.lock() {
                state.update_route_config(config, now);
            }
            ok = true;
        }
        Err(err) => {
            log::warn!("Route config fetch failed: {:?}", err);
        }
    }

    match fetch_blacklist(&base_url) {
        Ok(cards) => {
            if let Ok(mut state) = state.lock() {
                state.update_blacklist(cards, now);
            }
            ok = true;
        }
        Err(err) => {
            log::warn!("Blacklist fetch failed: {:?}", err);
        }
    }

    update_backend_status(state, ok);
    ok
}

fn fetch_route_config(base_url: &str, route_id: u16) -> Result<RouteConfig, NetError> {
    let url = format!("{}{}?route_id={}", base_url, CONFIG_PATH, route_id);
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<RouteConfigResponse> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(payload.message.unwrap_or_else(|| "request failed".to_string())));
    }
    let config = payload
        .data
        .ok_or_else(|| NetError::Api("empty config response".to_string()))?;
    Ok(config.into())
}

fn fetch_blacklist(base_url: &str) -> Result<Vec<String>, NetError> {
    let url = format!("{}{}?status=blocked", base_url, CARDS_PATH);
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<Vec<CardResponse>> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(payload.message.unwrap_or_else(|| "request failed".to_string())));
    }
    let cards = payload.data.unwrap_or_default();
    Ok(cards.into_iter().filter_map(|card| card.card_id).collect())
}

fn fetch_card_profile(base_url: &str, card_id: &str) -> Result<Option<CardProfile>, NetError> {
    let url = format!("{}{}?card_id={}", base_url, CARDS_PATH, card_id);
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);
    let headers = [("accept", "application/json")];
    let request = client.request(Method::Get, &url, &headers)?;
    let mut response = request.submit()?;
    let status = response.status();
    let body = read_response_body(&mut response)?;
    if !(200..300).contains(&status) {
        return Err(NetError::HttpStatus(status));
    }
    let payload: ApiResponse<Vec<CardResponse>> = serde_json::from_slice(&body)?;
    if !payload.success {
        return Err(NetError::Api(payload.message.unwrap_or_else(|| "request failed".to_string())));
    }
    let cards = payload.data.unwrap_or_default();
    Ok(cards.into_iter().next().map(|card| CardProfile {
        card_type: card.card_type,
        status: card.status,
        discount_rate: card.discount_rate,
        discount_amount: card.discount_amount,
    }))
}

fn read_response_body(
    response: &mut embedded_svc::http::client::Response<&mut EspHttpConnection>,
) -> Result<Vec<u8>, EspIOError> {
    let mut body = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let len = response.read(&mut buf)?;
        if len == 0 {
            break;
        }
        body.extend_from_slice(&buf[..len]);
    }
    Ok(body)
}

fn update_backend_status(state: &Arc<Mutex<GatewayState>>, reachable: bool) {
    if let Ok(mut state) = state.lock() {
        state.update_health(None, Some(reachable));
    }
}

fn resolve_base_url(state: &Arc<Mutex<GatewayState>>) -> String {
    if let Ok(state) = state.lock() {
        if !state.backend_base_url.is_empty() {
            return state.backend_base_url.clone();
        }
    }
    BACKEND_BASE_URL.to_string()
}

fn apply_card_profile(state: &Arc<Mutex<GatewayState>>, card_id: &str, profile: CardProfile) {
    let Some(tone) = tone_from_profile(&profile) else {
        return;
    };
    if let Ok(mut state) = state.lock() {
        let now_ms = current_epoch_millis();
        state.update_card_cache(
            card_id.to_string(),
            profile.card_type.clone(),
            profile.status.clone(),
            profile.discount_rate,
            profile.discount_amount,
            now_ms,
        );
        if state.last_card_id == card_id {
            if tone == PassengerTone::Error {
                state.last_passenger_tone = tone;
                if let Some(status) = profile.status.as_deref() {
                    if status == "blocked" {
                        state.last_passenger_message = "卡已冻结".to_string();
                    } else if status == "lost" {
                        state.last_passenger_message = "卡已挂失".to_string();
                    }
                }
            } else {
                state.update_passenger_tone(tone);
                if let Some(card_type) = profile.card_type.as_deref() {
                    state.apply_card_discount_policy(
                        card_type,
                        profile.discount_rate,
                        profile.discount_amount,
                    );
                }
            }
        }
    }
}

fn current_epoch() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    }
}

fn current_epoch_millis() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => 0,
    }
}

#[derive(Deserialize)]
struct RouteConfigResponse {
    route_id: u16,
    route_name: String,
    #[serde(default)]
    fare_type: Option<String>,
    #[serde(default)]
    tap_mode: Option<String>,
    max_fare: Option<f32>,
    #[serde(default)]
    stations: Vec<StationResponse>,
    #[serde(default)]
    fares: Vec<FareRuleResponse>,
}

#[derive(Deserialize)]
struct StationResponse {
    #[serde(default)]
    id: Option<u16>,
    name: String,
    sequence: u16,
    zone_id: Option<u16>,
    #[serde(default)]
    is_transfer: Option<bool>,
}

#[derive(Deserialize)]
struct FareRuleResponse {
    #[serde(default)]
    base_price: Option<f32>,
    #[serde(default)]
    fare_type: Option<String>,
    #[serde(default)]
    segment_count: Option<u16>,
    #[serde(default)]
    extra_price: Option<f32>,
    #[serde(default)]
    start_station: Option<u16>,
    #[serde(default)]
    end_station: Option<u16>,
}

#[derive(Deserialize)]
struct CardResponse {
    #[serde(default)]
    card_id: Option<String>,
    #[serde(default)]
    card_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    discount_rate: Option<f32>,
    #[serde(default)]
    discount_amount: Option<f32>,
}

struct CardProfile {
    card_type: Option<String>,
    status: Option<String>,
    discount_rate: Option<f32>,
    discount_amount: Option<f32>,
}

fn tone_from_profile(profile: &CardProfile) -> Option<PassengerTone> {
    if let Some(status) = profile.status.as_deref() {
        if status == "blocked" || status == "lost" {
            return Some(PassengerTone::Error);
        }
    }
    match profile.card_type.as_deref() {
        Some("student") => Some(PassengerTone::Student),
        Some("elder") => Some(PassengerTone::Elder),
        Some("disabled") => Some(PassengerTone::Disabled),
        Some("normal") => Some(PassengerTone::Normal),
        _ => None,
    }
}

impl From<RouteConfigResponse> for RouteConfig {
    fn from(value: RouteConfigResponse) -> Self {
        let fare_type = match value.fare_type.as_deref() {
            Some("segment") => FareType::Segment,
            Some("distance") => FareType::Distance,
            _ => FareType::Uniform,
        };
        let tap_mode = match value.tap_mode.as_deref() {
            Some("tap_in_out") => TapMode::TapInOut,
            _ => TapMode::SingleTap,
        };
        let fares = value
            .fares
            .into_iter()
            .map(|fare| FareRule {
                base_price: fare.base_price.unwrap_or(0.0),
                fare_type: fare.fare_type,
                segment_count: fare.segment_count,
                extra_price: fare.extra_price,
                start_station: fare.start_station,
                end_station: fare.end_station,
            })
            .collect();
        let stations = value
            .stations
            .into_iter()
            .map(|station| StationConfig {
                id: station.id.unwrap_or(0),
                name: station.name,
                sequence: station.sequence,
                zone_id: station.zone_id,
                is_transfer: station.is_transfer.unwrap_or(false),
            })
            .collect();
        RouteConfig {
            route_id: value.route_id,
            route_name: value.route_name,
            fare_type,
            tap_mode,
            max_fare: value.max_fare,
            stations,
            fares,
        }
    }
}
