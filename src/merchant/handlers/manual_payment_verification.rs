use actix_web::{web, HttpResponse, Result};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use serde::{Deserialize, Serialize};

use crate::shared::{
    entities::{payment_request, prelude::*},
    service::universal_address_monitor::UniversalAddressMonitor,
    utils::errors::AppError,
    AppState,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ManualVerificationRequest {
    pub payment_id: String,
    pub transaction_hash: Option<String>, 
    pub tx_hash: Option<String>,
    pub force_check: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ManualVerificationResponse {
    pub payment_id: String,
    pub status: String,
    pub message: String,
    pub transaction_found: bool,
    pub tx_hash: Option<String>,
    pub confirmations: Option<u32>,
    pub amount_matched: bool,
    pub current_payment_status: String,
    pub actions_taken: Vec<String>,
}


pub async fn verify_payment_manual(
    app_state: web::Data<AppState>,
    body: web::Json<ManualVerificationRequest>,
) -> Result<HttpResponse, AppError> {
    let mut response = ManualVerificationResponse {
        payment_id: body.payment_id.clone(),
        status: "checking".to_string(),
        message: "Starting manual verification".to_string(),
        transaction_found: false,
        tx_hash: None,
        confirmations: None,
        amount_matched: false,
        current_payment_status: "unknown".to_string(),
        actions_taken: vec![],
    };

    log::info!(
        " [MANUAL_VERIFY] Starting manual verification for payment: {}",
        body.payment_id
    );

    let payment = match PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&body.payment_id))
        .one(&app_state.db)
        .await?
    {
        Some(payment) => {
            response.current_payment_status = payment.status.clone();
            response
                .actions_taken
                .push("Found payment in database".to_string());
            log::info!(
                "✅ [MANUAL_VERIFY] Payment found: status={}, amount={} {}, address={}",
                payment.status,
                payment.amount,
                payment.currency,
                payment.wallet_address
            );
            payment
        }
        None => {
            response.status = "failed".to_string();
            response.message = format!("Payment {} not found in database", body.payment_id);
            log::error!(" [MANUAL_VERIFY] Payment {} not found", body.payment_id);
            return Ok(HttpResponse::NotFound().json(response));
        }
    };

    if payment.status.to_lowercase() == "paid" || payment.status.to_lowercase() == "confirmed" {
        response.status = "already_paid".to_string();
        response.message = "Payment is already confirmed".to_string();
        log::info!(
            "ℹ️ [MANUAL_VERIFY] Payment {} already has status: {}",
            body.payment_id,
            payment.status
        );
        return Ok(HttpResponse::Ok().json(response));
    }

    let monitor = UniversalAddressMonitor::new();
    let expected_amount = payment
        .amount
        .to_string()
        .parse::<f64>()
        .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

    response
        .actions_taken
        .push("Initialized address monitor".to_string());

    let tx_hash = body.transaction_hash.as_ref().or(body.tx_hash.as_ref());

    let detection_result = if let Some(hash) = tx_hash {
        log::info!(
            "🔎 [MANUAL_VERIFY] Verifying specific transaction {} for {} {}",
            hash,
            expected_amount,
            payment.currency
        );
        response
            .actions_taken
            .push(format!("Verifying specific transaction: {}", hash));

        monitor
            .verify_transaction_for_payment(
                hash,
                &payment.wallet_address,
                expected_amount,
                &payment.currency,
            )
            .await
    } else {
        log::info!(
            "🔎 [MANUAL_VERIFY] Checking for transactions to address {} for amount {} {}",
            payment.wallet_address,
            expected_amount,
            payment.currency
        );
        response
            .actions_taken
            .push("Scanning blockchain for matching transactions".to_string());

        monitor
            .check_payment_received(
                &payment.wallet_address,
                expected_amount,
                &payment.currency,
                payment.created_at.into(),
            )
            .await
    };

    match detection_result {
        Ok(Some(detected_tx)) => {
            response.transaction_found = true;
            response.tx_hash = Some(detected_tx.hash.clone());
            response.confirmations = Some(detected_tx.confirmations);
            response.amount_matched =
                (detected_tx.amount - expected_amount).abs() <= expected_amount * 0.01;

            response.actions_taken.push(format!(
                "Found transaction: {} with {} confirmations",
                detected_tx.hash, detected_tx.confirmations
            ));

            log::info!(
                "✅ [MANUAL_VERIFY] Transaction detected: hash={}, amount={}, confirmations={}",
                detected_tx.hash,
                detected_tx.amount,
                detected_tx.confirmations
            );

            let required_confirmations = match payment.environment.as_str() {
                "testnet" => 1,
                _ => match payment.currency.to_lowercase().as_str() {
                    "ethereum" | "eth" => 3,
                    "bnb" | "binancecoin" => 3,
                    "bitcoin" | "btc" => 6,
                    "solana" | "sol" => 1,
                    _ => 3,
                },
            };

            if detected_tx.confirmations >= required_confirmations {
                let txn = app_state.db.begin().await?;

                let current_metadata = payment
                    .metadata
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("{}");
                let mut metadata =
                    serde_json::from_str::<serde_json::Value>(current_metadata).unwrap_or_default();

                let mut active_payment: payment_request::ActiveModel = payment.into();
                active_payment.status = Set("paid".to_string());
                active_payment.paid_at = Set(Some(Utc::now().into()));
                active_payment.updated_at = Set(Some(Utc::now().into()));

                metadata["transaction_hash"] = serde_json::Value::String(detected_tx.hash.clone());
                metadata["confirmations"] =
                    serde_json::Value::Number(serde_json::Number::from(detected_tx.confirmations));
                metadata["required_confirmations"] =
                    serde_json::Value::Number(serde_json::Number::from(required_confirmations));
                metadata["detected_at"] = serde_json::Value::String(Utc::now().to_rfc3339());
                metadata["verified_manually"] = serde_json::Value::Bool(true);
                metadata["verification_method"] =
                    serde_json::Value::String("manual_endpoint".to_string());

                active_payment.metadata =
                    Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

                match active_payment.update(&txn).await {
                    Ok(_) => {
                        match txn.commit().await {
                            Ok(_) => {
                                response.status = "confirmed".to_string();
                                response.message = format!(
                                    "Payment confirmed! Transaction {} with {} confirmations (required: {})",
                                    detected_tx.hash, detected_tx.confirmations, required_confirmations
                                );
                                response.current_payment_status = "paid".to_string();
                                response
                                    .actions_taken
                                    .push("Updated payment status to 'paid'".to_string());

                                log::info!("🎉 [MANUAL_VERIFY] Payment {} confirmed via manual verification", body.payment_id);
                            }
                            Err(e) => {
                                response.status = "db_error".to_string();
                                response.message =
                                    format!("Failed to commit database transaction: {}", e);
                                log::error!(" [MANUAL_VERIFY] DB commit failed: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        response.status = "db_error".to_string();
                        response.message = format!("Failed to update payment: {}", e);
                        log::error!(" [MANUAL_VERIFY] DB update failed: {}", e);
                    }
                }
            } else {
                response.status = "insufficient_confirmations".to_string();
                response.message = format!(
                    "Transaction found but only has {} confirmations (required: {})",
                    detected_tx.confirmations, required_confirmations
                );
                response
                    .actions_taken
                    .push("Transaction needs more confirmations".to_string());

                log::info!(
                    "⏳ [MANUAL_VERIFY] Transaction found but needs more confirmations: {}/{}",
                    detected_tx.confirmations,
                    required_confirmations
                );
            }
        }
        Ok(None) => {
            response.status = "no_transaction".to_string();
            response.message = "No matching transaction found for this payment".to_string();
            response
                .actions_taken
                .push("Checked blockchain - no matching transaction".to_string());

            log::info!(
                " [MANUAL_VERIFY] No transaction found for payment {}",
                body.payment_id
            );
        }
        Err(e) => {
            response.status = "monitor_error".to_string();
            response.message = format!("Error checking blockchain: {}", e);
            response
                .actions_taken
                .push("Blockchain check failed".to_string());

            log::error!(
                " [MANUAL_VERIFY] Monitor error for payment {}: {}",
                body.payment_id,
                e
            );
        }
    }

    Ok(HttpResponse::Ok().json(response))
}

pub async fn get_payment_verification_status(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let payment_id = path.into_inner();

    log::info!(
        "📋 [VERIFY_STATUS] Checking status for payment: {}",
        payment_id
    );

    let payment = PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&payment_id))
        .one(&app_state.db)
        .await?;

    match payment {
        Some(payment) => {
            let response = serde_json::json!({
                "payment_id": payment.id,
                "status": payment.status,
                "amount": payment.amount.to_string(),
                "currency": payment.currency,
                "wallet_address": payment.wallet_address,
                "environment": payment.environment,
                "created_at": payment.created_at,
                "paid_at": payment.paid_at,
                "expires_at": payment.expires_at,
                "metadata": payment.metadata,
                "found": true
            });

            log::info!(
                "✅ [VERIFY_STATUS] Payment found: status={}",
                payment.status
            );
            Ok(HttpResponse::Ok().json(response))
        }
        None => {
            let response = serde_json::json!({
                "payment_id": payment_id,
                "found": false,
                "message": "Payment not found in database"
            });

            log::info!(" [VERIFY_STATUS] Payment {} not found", payment_id);
            Ok(HttpResponse::NotFound().json(response))
        }
    }
}
