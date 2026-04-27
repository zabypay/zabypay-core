use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status_code: u16,
    pub body: String,
}

impl ApiResponse {
    pub fn new(status_code: u16, body: String) -> Self {
        Self { status_code, body }
    }

    pub fn success<T: Serialize>(data: T, message: &str) -> serde_json::Value {
        serde_json::json!({
            "success": true,
            "status_code": 200,
            "message": message,
            "data": data
        })
    }

    pub fn error(status_code: u16, message: &str) -> serde_json::Value {
        serde_json::json!({
            "success": false,
            "status_code": status_code,
            "message": message,
            "data": null
        })
    }
}
