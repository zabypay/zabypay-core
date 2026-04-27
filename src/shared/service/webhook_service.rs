use actix_web::web;
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;
use tokio::time::sleep;

use crate::shared::{
    entities::{merchant, payment_request, prelude::*, webhook_log},
    utils::errors::AppError,
    AppState,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub event: String,
    pub data: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: String,
    pub merchant_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub webhook_url: String,
    pub status: WebhookStatus,
    pub attempts: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub response_status: Option<i32>,
    pub response_body: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WebhookStatus {
    Pending,
    Delivered,
    Failed,
    Retrying,
}

pub struct WebhookService {
    app_state: Arc<AppState>,
    client: reqwest::Client,
}

impl WebhookService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self {
            app_state,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Send webhook for payment event
    pub async fn send_payment_webhook(
        &self,
        payment: &payment_request::Model,
        event_type: &str,
    ) -> Result<(), AppError> {
        // Get merchant info
        let merchant = merchant::Entity::find_by_id(&payment.merchant_id)
            .one(&self.app_state.db)
            .await?
            .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

        if merchant.webhook_url.is_none() || merchant.webhook_secret.is_none() {
            log::debug!("Merchant {} has no webhook configured", merchant.id);
            return Ok(());
        }

        let webhook_url = merchant.webhook_url.unwrap();
        let webhook_secret = merchant.webhook_secret.unwrap();

        // Create webhook event data
        let event_data = self.create_payment_event_data(payment, event_type).await?;

        // Create webhook event
        let event = WebhookEvent {
            event: event_type.to_string(),
            data: event_data,
            timestamp: Utc::now(),
        };

        // Send webhook with payment context
        self.send_payment_specific_webhook(
            &merchant.id,
            &webhook_url,
            &webhook_secret,
            &event,
            &payment.id,
        )
        .await?;

        Ok(())
    }

    /// Create payment event data
    async fn create_payment_event_data(
        &self,
        payment: &payment_request::Model,
        event_type: &str,
    ) -> Result<serde_json::Value, AppError> {
        let mut data = serde_json::json!({
            "id": payment.id,
            "external_id": payment.external_id,
            "status": payment.status,
            "amount": payment.amount.to_string(),
            "currency": payment.currency.to_uppercase(),
            "wallet_address": payment.wallet_address,
            "created_at": payment.created_at,
            "expires_at": payment.expires_at,
        });

        // Add metadata if present
        if let Some(metadata_str) = &payment.metadata {
            if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(metadata_str) {
                data["metadata"] = metadata.clone();

                // Extract transaction details for confirmed payments
                if event_type == "payment.confirmed" {
                    if let Some(tx_hash) = metadata.get("transaction_hash") {
                        data["transaction_hash"] = tx_hash.clone();
                    }
                    if let Some(confirmations) = metadata.get("confirmations") {
                        data["confirmations"] = confirmations.clone();
                    }
                    if let Some(network) = metadata.get("network") {
                        data["network"] = network.clone();
                    }
                    if let Some(from_address) = metadata.get("from_address") {
                        data["from_address"] = from_address.clone();
                    }
                    if let Some(amount_received) = metadata.get("amount_received") {
                        data["amount_received"] = amount_received.clone();
                    }
                }
            }
        }

        // Add event-specific fields
        match event_type {
            "payment.confirmed" => {
                data["paid_at"] = serde_json::to_value(payment.paid_at)?;
            }
            "payment.expired" => {
                data["expired_at"] = serde_json::to_value(Utc::now())?;
            }
            "payment.failed" => {
                data["failed_at"] = serde_json::to_value(Utc::now())?;
            }
            _ => {}
        }

        Ok(data)
    }

    /// Send payment-specific webhook with retry logic
    async fn send_payment_specific_webhook(
        &self,
        merchant_id: &str,
        webhook_url: &str,
        webhook_secret: &str,
        event: &WebhookEvent,
        payment_request_id: &str,
    ) -> Result<(), AppError> {
        // Create webhook log entry with payment_request_id
        let webhook_id = format!("webhook_{}", uuid::Uuid::new_v4());
        let mut webhook_log = webhook_log::ActiveModel {
            id: Set(webhook_id.clone()),
            merchant_id: Set(merchant_id.to_string()),
            payment_request_id: Set(payment_request_id.to_string()), // Set payment ID
            event_type: Set(event.event.clone()),
            webhook_url: Set(webhook_url.to_string()),
            payload: Set(serde_json::to_string(&event)?),
            status: Set("pending".to_string()),
            attempts: Set(0),
            created_at: Set(Utc::now().into()),
            ..Default::default()
        };

        let mut webhook_entry = webhook_log.insert(&self.app_state.db).await?;

        // Attempt to send webhook with retries (same logic as general webhook)
        self.attempt_webhook_delivery(&mut webhook_entry, webhook_url, webhook_secret, event)
            .await?;

        Ok(())
    }

    /// Send webhook with retry logic
    pub async fn send_webhook(
        &self,
        merchant_id: &str,
        webhook_url: &str,
        webhook_secret: &str,
        event: &WebhookEvent,
    ) -> Result<(), AppError> {
        // Create webhook log entry
        let webhook_id = format!("webhook_{}", uuid::Uuid::new_v4());
        let mut webhook_log = webhook_log::ActiveModel {
            id: Set(webhook_id.clone()),
            merchant_id: Set(merchant_id.to_string()),
            payment_request_id: Set("".to_string()), // Default to empty for general webhooks
            event_type: Set(event.event.clone()),
            webhook_url: Set(webhook_url.to_string()),
            payload: Set(serde_json::to_string(&event)?),
            status: Set("pending".to_string()),
            attempts: Set(0),
            created_at: Set(Utc::now().into()),
            ..Default::default()
        };

        let mut webhook_entry = webhook_log.insert(&self.app_state.db).await?;

        // Attempt to send webhook with retries
        self.attempt_webhook_delivery(&mut webhook_entry, webhook_url, webhook_secret, event)
            .await?;

        Ok(())
    }

    /// Attempt webhook delivery with retries
    async fn attempt_webhook_delivery(
        &self,
        webhook_entry: &mut webhook_log::Model,
        webhook_url: &str,
        webhook_secret: &str,
        event: &WebhookEvent,
    ) -> Result<(), AppError> {
        let max_attempts = 5;
        let mut attempt = 0;

        while attempt < max_attempts {
            attempt += 1;

            // Update attempt count
            let mut active_webhook: webhook_log::ActiveModel = webhook_entry.clone().into();
            active_webhook.attempts = Set(attempt);
            active_webhook.last_attempt_at = Set(Some(Utc::now().into()));

            // Calculate signature
            let payload_json = serde_json::to_string(&event)?;
            let signature = self.calculate_signature(&payload_json, webhook_secret)?;

            // Send HTTP request
            log::info!(
                " Sending webhook attempt {}/{} to {}",
                attempt,
                max_attempts,
                webhook_url
            );

            match self
                .client
                .post(webhook_url)
                .header("Content-Type", "application/json")
                .header("X-Webhook-Signature", format!("sha256={}", signature))
                .body(payload_json.clone())
                .send()
                .await
            {
                Ok(response) => {
                    let status_code = response.status().as_u16() as i32;
                    let response_body = response.text().await.unwrap_or_default();

                    active_webhook.response_status = Set(Some(status_code));
                    active_webhook.response_body = Set(Some(response_body.clone()));

                    if status_code >= 200 && status_code < 300 {
                        // Success!
                        active_webhook.status = Set("delivered".to_string());
                        *webhook_entry = active_webhook.update(&self.app_state.db).await?;

                        log::info!("✅ Webhook delivered successfully to {}", webhook_url);
                        return Ok(());
                    } else {
                        // Non-2xx response
                        log::warn!(
                            " Webhook returned status {} for {}",
                            status_code,
                            webhook_url
                        );

                        if attempt < max_attempts {
                            active_webhook.status = Set("retrying".to_string());
                            active_webhook.next_retry_at =
                                Set(Some(self.calculate_next_retry(attempt).into()));
                        } else {
                            active_webhook.status = Set("failed".to_string());
                        }

                        *webhook_entry = active_webhook.update(&self.app_state.db).await?;
                    }
                }
                Err(e) => {
                    log::error!(" Webhook request failed: {}", e);
                    active_webhook.response_body = Set(Some(format!("Request error: {}", e)));

                    if attempt < max_attempts {
                        active_webhook.status = Set("retrying".to_string());
                        active_webhook.next_retry_at =
                            Set(Some(self.calculate_next_retry(attempt).into()));
                    } else {
                        active_webhook.status = Set("failed".to_string());
                    }

                    *webhook_entry = active_webhook.update(&self.app_state.db).await?;
                }
            }

            if attempt < max_attempts {
                // Wait before retry
                let wait_seconds = self.get_retry_delay_seconds(attempt as u64);
                log::info!("⏳ Waiting {} seconds before retry...", wait_seconds);
                sleep(tokio::time::Duration::from_secs(wait_seconds)).await;
            }
        }

        Err(AppError::ExternalServiceError(format!(
            "Webhook delivery failed after {} attempts",
            max_attempts
        )))
    }

    /// Calculate HMAC-SHA256 signature
    fn calculate_signature(&self, payload: &str, secret: &str) -> Result<String, AppError> {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| AppError::InternalServerError(format!("HMAC error: {}", e)))?;
        mac.update(payload.as_bytes());
        Ok(hex::encode(mac.finalize().into_bytes()))
    }

    /// Calculate next retry time based on attempt number
    fn calculate_next_retry(&self, attempt: i32) -> DateTime<Utc> {
        let delay_seconds = self.get_retry_delay_seconds(attempt as u64);
        Utc::now() + Duration::seconds(delay_seconds as i64)
    }

    /// Get retry delay in seconds (exponential backoff)
    fn get_retry_delay_seconds(&self, attempt: u64) -> u64 {
        match attempt {
            1 => 30,    // 30 seconds
            2 => 120,   // 2 minutes
            3 => 600,   // 10 minutes
            4 => 3600,  // 1 hour
            _ => 21600, // 6 hours
        }
    }

    /// Process pending webhook retries
    pub async fn process_pending_webhooks(&self) -> Result<(), AppError> {
        // Get webhooks that need retry
        let pending_webhooks = webhook_log::Entity::find()
            .filter(webhook_log::Column::Status.eq("retrying"))
            .filter(webhook_log::Column::NextRetryAt.lte(Utc::now()))
            .order_by_asc(webhook_log::Column::NextRetryAt)
            .all(&self.app_state.db)
            .await?;

        log::info!(
            "🔄 Processing {} pending webhook retries",
            pending_webhooks.len()
        );

        for webhook in pending_webhooks {
            // Get merchant for webhook secret
            let merchant = merchant::Entity::find_by_id(&webhook.merchant_id)
                .one(&self.app_state.db)
                .await?;

            if let Some(merchant) = merchant {
                if let Some(webhook_secret) = merchant.webhook_secret {
                    // Parse original event
                    if let Ok(event) = serde_json::from_str::<WebhookEvent>(&webhook.payload) {
                        // Retry sending
                        let _ = self
                            .send_webhook(
                                &webhook.merchant_id,
                                &webhook.webhook_url,
                                &webhook_secret,
                                &event,
                            )
                            .await;
                    }
                }
            }
        }

        Ok(())
    }

    /// Get webhook delivery status
    pub async fn get_webhook_status(
        &self,
        webhook_id: &str,
    ) -> Result<webhook_log::Model, AppError> {
        webhook_log::Entity::find_by_id(webhook_id)
            .one(&self.app_state.db)
            .await?
            .ok_or(AppError::NotFound("Webhook not found".to_string()))
    }

    /// List webhooks for a merchant
    pub async fn list_merchant_webhooks(
        &self,
        merchant_id: &str,
        limit: u64,
    ) -> Result<Vec<webhook_log::Model>, AppError> {
        webhook_log::Entity::find()
            .filter(webhook_log::Column::MerchantId.eq(merchant_id))
            .order_by_desc(webhook_log::Column::CreatedAt)
            .limit(limit)
            .all(&self.app_state.db)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    }

    /// Resend a webhook
    pub async fn resend_webhook(&self, webhook_id: &str) -> Result<(), AppError> {
        let webhook = self.get_webhook_status(webhook_id).await?;

        // Get merchant for webhook secret
        let merchant = merchant::Entity::find_by_id(&webhook.merchant_id)
            .one(&self.app_state.db)
            .await?
            .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

        let webhook_secret = merchant.webhook_secret.ok_or(AppError::ValidationError(
            "Merchant has no webhook secret".to_string(),
        ))?;

        // Parse original event
        let event = serde_json::from_str::<WebhookEvent>(&webhook.payload).map_err(|e| {
            AppError::InternalServerError(format!("Invalid webhook payload: {}", e))
        })?;

        // Send webhook
        self.send_webhook(
            &webhook.merchant_id,
            &webhook.webhook_url,
            &webhook_secret,
            &event,
        )
        .await
    }
}

/// Start webhook retry service
pub async fn start_webhook_retry_service(app_state: Arc<AppState>) {
    let webhook_service = WebhookService::new(app_state);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));

        loop {
            interval.tick().await;

            if let Err(e) = webhook_service.process_pending_webhooks().await {
                log::error!("Error processing webhook retries: {}", e);
            }
        }
    });

    log::info!(" Webhook retry service started");
}
