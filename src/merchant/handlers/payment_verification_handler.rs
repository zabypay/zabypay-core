use actix_web::{web, HttpResponse};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::shared::{
    entities::{payment_request, prelude::*},
    service::universal_address_monitor::UniversalAddressMonitor,
    utils::errors::AppError,
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct VerifyPaymentRequest {
    pub payment_id: Option<String>,
    pub address: Option<String>,
    pub currency: Option<String>,
    pub expected_amount: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct VerifyPaymentResponse {
    pub payment_found: bool,
    pub payment_id: Option<String>,
    pub address: String,
    pub currency: String,
    pub expected_amount: f64,
    pub detected_transaction: Option<DetectedTransactionInfo>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DetectedTransactionInfo {
    pub transaction_hash: String,
    pub amount_received: f64,
    pub currency: String,
    pub block_time: DateTime<Utc>,
    pub confirmations: u32,
    pub status: String,
    pub from_address: String,
    pub network: String,
}

pub async fn verify_payment(
    app_state: web::Data<AppState>,
    body: web::Json<VerifyPaymentRequest>,
) -> Result<HttpResponse, AppError> {
    let monitor = UniversalAddressMonitor::new();

    let (address, currency, expected_amount, payment_id) =
        if let Some(payment_id) = &body.payment_id {
            let payment = PaymentRequest::find()
                .filter(payment_request::Column::Id.eq(payment_id))
                .one(&app_state.db)
                .await?
                .ok_or(AppError::NotFound(format!(
                    "Payment {} not found",
                    payment_id
                )))?;

            (
                payment.wallet_address.clone(),
                payment.currency.clone(),
                payment.amount.to_string().parse::<f64>().unwrap_or(0.0),
                Some(payment.id.clone()),
            )
        } else {
            let address = body.address.clone().ok_or(AppError::ValidationError(
                "Address is required when payment_id is not provided".to_string(),
            ))?;
            let currency = body.currency.clone().ok_or(AppError::ValidationError(
                "Currency is required when payment_id is not provided".to_string(),
            ))?;
            let expected_amount = body.expected_amount.ok_or(AppError::ValidationError(
                "Expected amount is required when payment_id is not provided".to_string(),
            ))?;

            (address, currency, expected_amount, None)
        };

    log::info!(
        " Verifying payment for address: {} (currency: {}, amount: {})",
        address,
        currency,
        expected_amount
    );

    let since_time = Utc::now() - chrono::Duration::hours(24);

    match monitor
        .check_payment_received(&address, expected_amount, &currency, since_time)
        .await
    {
        Ok(Some(tx)) => {
            log::info!("✅ Payment detected! Transaction: {}", tx.hash);

            if let Some(payment_id) = &payment_id {
                if let Ok(payment) = PaymentRequest::find()
                    .filter(payment_request::Column::Id.eq(payment_id))
                    .one(&app_state.db)
                    .await?
                    .ok_or(AppError::NotFound("Payment not found".to_string()))
                {
                    let mut metadata = if let Some(metadata_str) = &payment.metadata {
                        serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
                    } else {
                        serde_json::json!({})
                    };

                    metadata["transaction_hash"] = serde_json::Value::String(tx.hash.clone());
                    metadata["detected_at"] = serde_json::Value::String(Utc::now().to_rfc3339());
                    metadata["amount_received"] = serde_json::Value::Number(
                        serde_json::Number::from_f64(tx.amount)
                            .unwrap_or(serde_json::Number::from(0)),
                    );
                    metadata["network"] = serde_json::Value::String(tx.network.clone());
                    metadata["manual_verification"] = serde_json::Value::Bool(true);

                    let mut active_payment: payment_request::ActiveModel = payment.into();
                    active_payment.metadata =
                        sea_orm::Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));
                    active_payment.update(&app_state.db).await?;

                    log::info!("📝 Updated payment {} with transaction details", payment_id);
                }
            }

            Ok(HttpResponse::Ok().json(VerifyPaymentResponse {
                payment_found: true,
                payment_id,
                address: address.clone(),
                currency: currency.to_uppercase(),
                expected_amount,
                detected_transaction: Some(DetectedTransactionInfo {
                    transaction_hash: tx.hash,
                    amount_received: tx.amount,
                    currency: tx.currency,
                    block_time: tx.block_time,
                    confirmations: tx.confirmations,
                    status: tx.status,
                    from_address: tx.from_address,
                    network: tx.network,
                }),
                message: format!("Payment detected with {} confirmations", tx.confirmations),
            }))
        }
        Ok(None) => {
            log::info!(" No payment found for address: {}", address);

            Ok(HttpResponse::Ok().json(VerifyPaymentResponse {
                payment_found: false,
                payment_id,
                address: address.clone(),
                currency: currency.to_uppercase(),
                expected_amount,
                detected_transaction: None,
                message: format!(
                    "No payment found for {} {} to address {} in the last 24 hours",
                    expected_amount,
                    currency.to_uppercase(),
                    address
                ),
            }))
        }
        Err(e) => {
            log::error!(" Error verifying payment: {}", e);

            Ok(HttpResponse::Ok().json(VerifyPaymentResponse {
                payment_found: false,
                payment_id,
                address: address.clone(),
                currency: currency.to_uppercase(),
                expected_amount,
                detected_transaction: None,
                message: format!("Error checking blockchain: {}", e),
            }))
        }
    }
}

pub async fn force_check_payment(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let payment_id = path.into_inner();

    verify_payment(
        app_state,
        web::Json(VerifyPaymentRequest {
            payment_id: Some(payment_id),
            address: None,
            currency: None,
            expected_amount: None,
        }),
    )
    .await
}

pub async fn get_explorer_link(
    query: web::Query<ExplorerLinkQuery>,
) -> Result<HttpResponse, AppError> {
    let explorer_url = match query.network.to_lowercase().as_str() {
        "bitcoin" | "btc" => format!("https://blockstream.info/tx/{}", query.tx_hash),
        "ethereum" | "eth" => format!("https://etherscan.io/tx/{}", query.tx_hash),
        "solana" | "sol" => format!("https://solscan.io/tx/{}", query.tx_hash),
        "bnb" | "binancecoin" => format!("https://bscscan.com/tx/{}", query.tx_hash),
        "polygon" | "matic" => format!("https://polygonscan.com/tx/{}", query.tx_hash),
        "avalanche" | "avax" => format!("https://snowtrace.io/tx/{}", query.tx_hash),
        _ => {
            return Err(AppError::ValidationError(format!(
                "Unknown network: {}",
                query.network
            )))
        }
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "explorer_url": explorer_url,
        "network": query.network,
        "tx_hash": query.tx_hash,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ExplorerLinkQuery {
    pub network: String,
    pub tx_hash: String,
}
