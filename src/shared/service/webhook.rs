use crate::{
    merchant::models::payment::{PaymentResponse, WebhookPayload},
    shared::{
        entities::{merchant, payment_request},
        utils::payment_uri::PaymentUriGenerator,
        AppState,
    },
};
use actix_web::web;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sea_orm::EntityTrait;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub async fn send_payment_webhook(
    app_state: &AppState,
    payment: &payment_request::Model,
    event_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get merchant info for webhook URL and secret
    let merchant = merchant::Entity::find_by_id(&payment.merchant_id)
        .one(&app_state.db)
        .await?
        .ok_or("Merchant not found")?;

    if let Some(webhook_url) = &merchant.webhook_url {
        if let Some(webhook_secret) = &merchant.webhook_secret {
            // Generate QR metadata
            let qr_metadata = PaymentUriGenerator::get_qr_metadata(
                &payment.currency,
                &payment.wallet_address,
                &payment.amount.to_string(),
                &payment.id,
                &payment.external_id.as_ref().unwrap_or(&String::new()),
            );

            // Create payment response for webhook
            // Extract USD pricing from metadata
            use crate::shared::models::usd_pricing::PaymentMetadata;
            let extracted_usd_pricing =
                PaymentMetadata::extract_usd_pricing(payment.metadata.clone());

            let payment_response = PaymentResponse {
                id: payment.id.clone(),
                merchant_id: payment.merchant_id.clone(),
                external_id: payment.external_id.clone().unwrap_or_default(),
                amount: payment.amount.to_string(),
                currency: payment.currency.to_uppercase(),
                wallet_address: payment.wallet_address.clone(),
                status: payment.status.clone(),
                metadata: serde_json::from_str(&payment.metadata.clone().unwrap_or_default())
                    .unwrap_or_default(),
                payment_url: payment.payment_url.clone().unwrap_or_default(),
                expires_at: payment.expires_at.into(),
                paid_at: payment.paid_at.map(|dt| dt.into()),
                created_at: payment.created_at.into(),
                updated_at: payment
                    .updated_at
                    .map(|dt| dt.into())
                    .unwrap_or_else(|| Utc::now()),
                qr_data: qr_metadata,
                usd_price: extracted_usd_pricing.usd_price,
                usd_value: extracted_usd_pricing.usd_value,
                price_snapshot_ts: extracted_usd_pricing.price_snapshot_ts,
                is_simulated_price: extracted_usd_pricing.is_simulated,
            };

            // Create webhook payload
            let payload = WebhookPayload {
                event: event_type.to_string(),
                data: payment_response,
                timestamp: Utc::now(),
                signature: String::new(), // Will be filled below
            };

            // Serialize payload for signing
            let payload_json = serde_json::to_string(&payload)?;

            // Generate HMAC signature
            let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())?;
            mac.update(payload_json.as_bytes());
            let signature = hex::encode(mac.finalize().into_bytes());

            // Create final payload with signature
            let final_payload = WebhookPayload {
                signature: format!("sha256={}", signature),
                ..payload
            };

            // Send webhook (in production, use a proper HTTP client and queue)
            let client = reqwest::Client::new();
            let response = client
                .post(webhook_url)
                .header("Content-Type", "application/json")
                .header("X-Webhook-Signature", &final_payload.signature)
                .json(&final_payload)
                .send()
                .await?;

            if response.status().is_success() {
                println!("✅ Webhook sent successfully to {}", webhook_url);
            } else {
                println!(" Webhook failed: {} - {}", response.status(), webhook_url);
            }
        }
    }

    Ok(())
}
