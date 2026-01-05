use crate::model::UploadRecord;

#[derive(Clone, Debug)]
pub struct BatchUpload {
    pub records: Vec<UploadRecord>,
}

impl BatchUpload {
    pub fn new(records: Vec<UploadRecord>) -> Self {
        Self { records }
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string(&self.records).unwrap_or_else(|_| "[]".to_string())
    }
}
