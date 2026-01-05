#[derive(Clone, Debug)]
pub struct ApiConfig {
    pub base_url: String,
}

pub const CONFIG_PATH: &str = "/api/v1/bus/config";
pub const BATCH_RECORDS_PATH: &str = "/api/v1/bus/batchRecords";
pub const CARDS_PATH: &str = "/api/v1/cards";

impl ApiConfig {
    pub fn config_url(&self, route_id: u16) -> String {
        format!("{}{}?route_id={}", self.base_url, CONFIG_PATH, route_id)
    }

    pub fn batch_records_url(&self) -> String {
        format!("{}{}", self.base_url, BATCH_RECORDS_PATH)
    }
}
