use crate::model::UploadRecord;

/// 批量上报结构封装。
#[derive(Clone, Debug)]
pub struct BatchUpload {
    pub records: Vec<UploadRecord>,
}

impl BatchUpload {
    /// 构造批量记录。
    pub fn new(records: Vec<UploadRecord>) -> Self {
        Self { records }
    }

    /// 是否为空批次。
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// 序列化为 JSON 字符串。
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(&self.records).unwrap_or_else(|_| "[]".to_string())
    }
}
