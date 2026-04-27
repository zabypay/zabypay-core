use crate::shared::utils::payment_uri::QrMetadata;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use validator::Validate;


#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreatePaymentRequest {
    #[validate(length(min = 1, message = "Amount is required"))]
    pub amount: String,

    #[validate(length(
        min = 2,
        max = 10,
        message = "Currency must be between 2 and 10 characters"
    ))]
    pub currency: String,

    #[validate(length(
        min = 1,
        max = 100,
        message = "External ID must be between 1 and 100 characters"
    ))]
    pub external_id: String,

    #[serde(default = "default_expires_in_seconds")]
    pub expires_in_seconds: u32,

    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_expires_in_seconds() -> u32 {
    3600
}

#[derive(Debug, Clone, Serialize)]
pub struct PaymentResponse {
    pub id: String,

    pub merchant_id: String,

    pub external_id: String,

    pub amount: String,

    pub currency: String,

    pub wallet_address: String,

    pub status: String,

    pub metadata: HashMap<String, serde_json::Value>,

    pub payment_url: String,

    pub expires_at: DateTime<Utc>,

    pub paid_at: Option<DateTime<Utc>>,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,

    pub qr_data: QrMetadata,

    pub usd_price: Option<Decimal>,

    pub usd_value: Option<Decimal>,

    pub price_snapshot_ts: Option<DateTime<Utc>>,

    pub is_simulated_price: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaymentStatus {
    Pending,
    Paid,
    Expired,
    Failed,
}

impl PaymentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PaymentStatus::Pending => "pending",
            PaymentStatus::Paid => "paid",
            PaymentStatus::Expired => "expired",
            PaymentStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pending" => Some(PaymentStatus::Pending),
            "paid" => Some(PaymentStatus::Paid),
            "expired" => Some(PaymentStatus::Expired),
            "failed" => Some(PaymentStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaymentListQuery {
    pub page: Option<u64>,

    pub limit: Option<u64>,

    pub status: Option<String>,

    pub currency: Option<String>,

    pub from_date: Option<DateTime<Utc>>,

    pub to_date: Option<DateTime<Utc>>,

    pub environment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookPayload {
    pub event: String,

    pub data: PaymentResponse,

    pub timestamp: DateTime<Utc>,

    pub signature: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdatePaymentStatusRequest {
    #[validate(length(min = 1, message = "Status is required"))]
    pub status: String,

    pub transaction_hash: Option<String>,

    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ConfirmPaymentRequest {
    #[validate(length(min = 1, message = "Transaction hash is required"))]
    pub transaction_hash: String,
    #[serde(default)]
    pub confirmations: u32,
    pub amount_received: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaymentListResponse {

    pub payments: Vec<PaymentResponse>,
    pub pagination: PaginationInfo,

    pub total_volume_usd: Option<Decimal>,

    pub volume_breakdown: Option<VolumeBreakdown>,
}
#[derive(Debug, Serialize)]
pub struct VolumeBreakdown {
    pub pending_usd: Decimal,
    pub paid_usd: Decimal,
    
    pub expired_usd: Decimal,

    pub failed_usd: Decimal,
}


#[derive(Debug, Serialize)]
pub struct PaginationInfo {
    pub page: u64,
    pub limit: u64,
    pub total: u64,
    pub total_pages: u64,
}
