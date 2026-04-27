use chrono::Utc;
use log;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::Arc;

use crate::shared::{
    entities::{payment_request, prelude::*},
    service::universal_address_monitor::UniversalAddressMonitor,
    utils::errors::AppError,
    AppState,
};

/// Simple, reliable payment monitoring service
pub struct SimplePaymentMonitor;

impl SimplePaymentMonitor {
    /// Check all pending payments and update their status if they have been paid
    pub async fn check_all_pending_payments(app_state: &AppState) -> Result<u32, AppError> {
        log::info!(" [SIMPLE_MONITOR] Starting payment check cycle");

        // Get all pending payments
        let pending_payments = PaymentRequest::find()
            .filter(payment_request::Column::Status.eq("pending"))
            .all(&app_state.db)
            .await?;

        log::info!(
            " [SIMPLE_MONITOR] Found {} pending payments to check",
            pending_payments.len()
        );

        let mut confirmed_count = 0u32;
        let monitor = UniversalAddressMonitor::new();

        for payment in pending_payments {
            log::info!(
                "🔎 [SIMPLE_MONITOR] Checking payment {} - Currency: {}, Amount: {}, Address: {}",
                payment.id,
                payment.currency,
                payment.amount,
                payment.wallet_address
            );

            // Skip expired payments
            if payment.expires_at <= Utc::now() {
                log::info!(
                    " [SIMPLE_MONITOR] Payment {} expired, skipping",
                    payment.id
                );
                continue;
            }

            let expected_amount = payment.amount.to_string().parse::<f64>().unwrap_or(0.0);

            // Map currency to network
            let effective_currency = match payment.environment.as_str() {
                "testnet" | "devnet" => {
                    match payment.currency.to_lowercase().as_str() {
                        "eth" | "ethereum" => "base_sepolia",
                        "sol" | "solana" => "solana_testnet",
                        "usdt" => "usdt", // USDT works on mainnet even in testnet mode
                        "usdt_bnb" => "bnb", // BNB BEP-20 USDT on testnet (uses mainnet BNB network)
                        _ => &payment.currency,
                    }
                }
                _ => {
                    match payment.currency.to_lowercase().as_str() {
                        "btc" => "bitcoin",
                        "eth" => "ethereum",
                        "sol" => "solana",
                        "bnb" => "bnb",
                        "matic" => "polygon",
                        "avax" => "avalanche",
                        "usdt" => "usdt",    // Maps to Ethereum USDT by default
                        "usdt_bnb" => "bnb", // BNB BEP-20 USDT
                        _ => &payment.currency,
                    }
                }
            };

            log::info!(
                "🌐 [SIMPLE_MONITOR] Payment {} mapped to network: {} -> {}",
                payment.id,
                payment.currency,
                effective_currency
            );

            // Check for payment
            match monitor
                .check_payment_received(
                    &payment.wallet_address,
                    expected_amount,
                    effective_currency,
                    payment.created_at.into(),
                )
                .await
            {
                Ok(Some(detected_tx)) => {
                    log::info!(
                        "🎉 [SIMPLE_MONITOR] Payment {} DETECTED! Tx: {}, Amount: {}",
                        payment.id,
                        detected_tx.hash,
                        detected_tx.amount
                    );

                    // Update payment status
                    if let Err(e) =
                        Self::confirm_payment(app_state, &payment, &detected_tx.hash).await
                    {
                        log::error!(
                            " [SIMPLE_MONITOR] Failed to confirm payment {}: {}",
                            payment.id,
                            e
                        );
                    } else {
                        confirmed_count += 1;
                        log::info!(
                            "✅ [SIMPLE_MONITOR] Payment {} confirmed successfully!",
                            payment.id
                        );
                    }
                }
                Ok(None) => {
                    log::debug!("📭 [SIMPLE_MONITOR] No payment found for {}", payment.id);
                }
                Err(e) => {
                    log::error!(
                        " [SIMPLE_MONITOR] Error checking payment {}: {}",
                        payment.id,
                        e
                    );
                }
            }
        }

        log::info!(
            "✅ [SIMPLE_MONITOR] Payment check cycle complete. Paid: {} payments",
            confirmed_count
        );
        Ok(confirmed_count)
    }

    /// Confirm a payment and update its status in the database
    async fn confirm_payment(
        app_state: &AppState,
        payment: &payment_request::Model,
        tx_hash: &str,
    ) -> Result<(), AppError> {
        log::info!(
            "💾 [SIMPLE_MONITOR] Confirming payment {} with transaction {}",
            payment.id,
            tx_hash
        );

        let mut active_payment: payment_request::ActiveModel = payment.clone().into();
        active_payment.status = Set("paid".to_string());
        active_payment.paid_at = Set(Some(Utc::now().into()));
        active_payment.updated_at = Set(Some(Utc::now().into()));

        // Update metadata with transaction hash
        let mut metadata = if let Some(metadata_str) = &payment.metadata {
            serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        metadata["transaction_hash"] = serde_json::Value::String(tx_hash.to_string());
        metadata["confirmed_at"] = serde_json::Value::String(Utc::now().to_rfc3339());
        metadata["confirmations"] = serde_json::Value::Number(serde_json::Number::from(15));

        active_payment.metadata = Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

        // Update in database
        match active_payment.update(&app_state.db).await {
            Ok(updated_payment) => {
                log::info!(
                    "✅ [SIMPLE_MONITOR] Payment {} updated to 'paid' status",
                    payment.id
                );

                // Send webhook notification if webhook service is available
                if let Some(payment_monitor) = &app_state.payment_monitor {
                    // Try to trigger webhook (best effort - don't fail confirmation if webhook fails)
                    if let Err(e) =
                        Self::send_webhook_notification(app_state, &updated_payment).await
                    {
                        log::warn!(" [SIMPLE_MONITOR] Webhook failed for payment {} (payment still confirmed): {}", payment.id, e);
                    }
                }

                Ok(())
            }
            Err(e) => {
                log::error!(
                    " [SIMPLE_MONITOR] Failed to update payment {}: {}",
                    payment.id,
                    e
                );
                Err(AppError::DatabaseError(format!(
                    "Failed to confirm payment: {}",
                    e
                )))
            }
        }
    }

    /// Send webhook notification (best effort)
    async fn send_webhook_notification(
        app_state: &AppState,
        payment: &payment_request::Model,
    ) -> Result<(), AppError> {
        use crate::shared::service::webhook_service::WebhookService;

        let webhook_service = WebhookService::new(Arc::new(app_state.clone()));
        webhook_service
            .send_payment_webhook(payment, "payment.paid")
            .await?;
        log::info!(
            "📡 [SIMPLE_MONITOR] Webhook sent for payment {}",
            payment.id
        );
        Ok(())
    }

    /// Run a single monitoring cycle (useful for testing or manual triggers)
    pub async fn run_monitoring_cycle(app_state: &AppState) -> Result<String, AppError> {
        let start_time = Utc::now();
        let confirmed_count = Self::check_all_pending_payments(app_state).await?;
        let duration = Utc::now().signed_duration_since(start_time);

        let result = format!(
            "Monitoring cycle completed in {}ms. Paid {} payments.",
            duration.num_milliseconds(),
            confirmed_count
        );

        log::info!(" [SIMPLE_MONITOR] {}", result);
        Ok(result)
    }
}
