use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use validator::Validate;


#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyResponse {
    pub id: String,

    pub name: String,

    pub key_prefix: String,

    pub is_active: bool,

    pub last_used: Option<DateTime<Utc>>,

    pub created_at: DateTime<Utc>,
}


#[derive(Debug, Clone, Deserialize, Validate)]
pub struct UpdateApiKeyRequest {
    #[validate(length(
        min = 1,
        max = 100,
        message = "Name must be between 1 and 100 characters"
    ))]
    pub name: Option<String>,
}

