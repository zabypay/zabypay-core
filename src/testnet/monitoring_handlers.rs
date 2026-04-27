use actix_web::{delete, get, post, web, HttpRequest, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::shared::{utils::errors::AppError, AppState};

#[derive(Debug, Deserialize, Validate)]
pub struct StartMonitoringRequest {
    #[validate(length(min = 1, message = "Payment ID is required"))]
    pub payment_id: String,

    #[validate(length(min = 1, message = "Transaction hash is required"))]
    pub transaction_hash: String,

    pub network: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StopMonitoringRequest {
    pub transaction_hash: String,
}

#[derive(Debug, Serialize)]
pub struct TestnetMonitoringResponse {
    pub success: bool,
    pub message: String,
    pub environment: String,
    pub data: Option<serde_json::Value>,
}

/// Start the payment monitoring service for testnet
#[post("/start")]
pub async fn start_testnet_monitoring_service(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    // Extract test API key from header for authentication
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet monitoring endpoints".to_string(),
        ));
    }

    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.start_monitoring().await {
        Ok(_) => Ok(HttpResponse::Ok().json(TestnetMonitoringResponse {
            success: true,
            message: "Testnet payment monitoring service started successfully".to_string(),
            environment: "testnet".to_string(),
            data: None,
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(TestnetMonitoringResponse {
            success: false,
            message: format!("Failed to start testnet monitoring service: {}", e),
            environment: "testnet".to_string(),
            data: None,
        })),
    }
}

/// Stop the payment monitoring service for testnet
#[post("/stop")]
pub async fn stop_testnet_monitoring_service(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    // Extract test API key from header for authentication
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet monitoring endpoints".to_string(),
        ));
    }

    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.stop_monitoring().await {
        Ok(_) => Ok(HttpResponse::Ok().json(TestnetMonitoringResponse {
            success: true,
            message: "Testnet payment monitoring service stopped successfully".to_string(),
            environment: "testnet".to_string(),
            data: None,
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(TestnetMonitoringResponse {
            success: false,
            message: format!("Failed to stop testnet monitoring service: {}", e),
            environment: "testnet".to_string(),
            data: None,
        })),
    }
}

/// Get testnet monitoring service status
#[get("/status")]
pub async fn get_testnet_monitoring_status(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    // Extract test API key from header for authentication
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet monitoring endpoints".to_string(),
        ));
    }

    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.get_monitoring_status().await {
        Ok(mut status) => {
            // Filter to show only testnet-relevant information
            if let Some(status_obj) = status.as_object_mut() {
                status_obj.insert(
                    "environment".to_string(),
                    serde_json::Value::String("testnet".to_string()),
                );
                status_obj.insert("note".to_string(), serde_json::Value::String("This status shows the global monitoring service that handles both mainnet and testnet payments".to_string()));
            }

            Ok(HttpResponse::Ok().json(TestnetMonitoringResponse {
                success: true,
                message: "Testnet monitoring status retrieved successfully".to_string(),
                environment: "testnet".to_string(),
                data: Some(status),
            }))
        }
        Err(e) => Ok(
            HttpResponse::InternalServerError().json(TestnetMonitoringResponse {
                success: false,
                message: format!("Failed to get testnet monitoring status: {}", e),
                environment: "testnet".to_string(),
                data: None,
            }),
        ),
    }
}

/// Start monitoring a specific testnet payment
#[post("/payments/monitor")]
pub async fn start_monitoring_testnet_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<StartMonitoringRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    // Extract test API key from header for authentication
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet monitoring endpoints".to_string(),
        ));
    }

    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    // Force network to base_sepolia for testnet
    let network = Some("base_sepolia".to_string());

    match monitor_service
        .start_monitoring_payment(&body.payment_id, &body.transaction_hash, network)
        .await
    {
        Ok(_) => Ok(HttpResponse::Ok().json(TestnetMonitoringResponse {
            success: true,
            message: format!(
                "Started monitoring testnet payment {} with transaction {} on Base Sepolia",
                body.payment_id, body.transaction_hash
            ),
            environment: "testnet".to_string(),
            data: Some(serde_json::json!({
                "payment_id": body.payment_id,
                "transaction_hash": body.transaction_hash,
                "network": "base_sepolia",
                "chain_id": 84532
            })),
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(TestnetMonitoringResponse {
            success: false,
            message: format!("Failed to start monitoring testnet payment: {}", e),
            environment: "testnet".to_string(),
            data: None,
        })),
    }
}

/// Stop monitoring a specific testnet transaction
#[delete("/payments/monitor/{tx_hash}")]
pub async fn stop_monitoring_testnet_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let tx_hash = path.into_inner();

    // Extract test API key from header for authentication
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet monitoring endpoints".to_string(),
        ));
    }

    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.stop_monitoring_payment(&tx_hash).await {
        Ok(_) => Ok(HttpResponse::Ok().json(TestnetMonitoringResponse {
            success: true,
            message: format!("Stopped monitoring testnet transaction {}", tx_hash),
            environment: "testnet".to_string(),
            data: Some(serde_json::json!({
                "transaction_hash": tx_hash,
                "network": "base_sepolia"
            })),
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(TestnetMonitoringResponse {
            success: false,
            message: format!("Failed to stop monitoring testnet transaction: {}", e),
            environment: "testnet".to_string(),
            data: None,
        })),
    }
}

/// List testnet payments with status filter
#[get("/payments")]
pub async fn list_testnet_payments(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    // Extract test API key from header for authentication
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet monitoring endpoints".to_string(),
        ));
    }

    // Extract key prefix to find merchant
    let key_prefix = if api_key.len() >= 16 {
        &api_key[8..16] // Skip "test_ak_" and take next 8 chars
    } else {
        return Err(AppError::Unauthorized(
            "Invalid test API key format".to_string(),
        ));
    };

    // Find API key by prefix to get merchant
    use crate::shared::entities::{api_key, prelude::*};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(format!("test_{}", key_prefix)))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?
        .ok_or(AppError::Unauthorized("Invalid test API key".to_string()))?;

    // Get testnet payments for this merchant
    use crate::shared::entities::payment_request;
    use sea_orm::{PaginatorTrait, QueryOrder, QuerySelect};

    let page = query
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let limit = query
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(100);
    let offset = (page - 1) * limit;

    let mut payments_query = PaymentRequest::find()
        .filter(payment_request::Column::MerchantId.eq(&api_key_record.merchant_id))
        .filter(payment_request::Column::Environment.eq("testnet"));

    // Apply status filter if provided
    if let Some(status) = query.get("status").and_then(|v| v.as_str()) {
        payments_query = payments_query.filter(payment_request::Column::Status.eq(status));
    }

    // Apply currency filter if provided
    if let Some(currency) = query.get("currency").and_then(|v| v.as_str()) {
        payments_query =
            payments_query.filter(payment_request::Column::Currency.eq(currency.to_lowercase()));
    }

    // Get total count
    let total_count = payments_query.clone().count(&app_state.db).await?;

    // Get payments with pagination
    let payments = payments_query
        .order_by_desc(payment_request::Column::CreatedAt)
        .offset(offset)
        .limit(limit)
        .all(&app_state.db)
        .await?;

    let payment_responses: Vec<serde_json::Value> = payments
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "external_id": p.external_id.unwrap_or_default(),
                "amount": p.amount.to_string(),
                "currency": p.currency.to_uppercase(),
                "wallet_address": p.wallet_address,
                "status": p.status,
                "environment": "testnet",
                "network": "base_sepolia",
                "chain_id": 84532,
                "paid_at": p.paid_at,
                "created_at": p.created_at,
                "updated_at": p.updated_at
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(TestnetMonitoringResponse {
        success: true,
        message: "Testnet payments retrieved successfully".to_string(),
        environment: "testnet".to_string(),
        data: Some(serde_json::json!({
            "payments": payment_responses,
            "pagination": {
                "page": page,
                "limit": limit,
                "total": total_count,
                "total_pages": (total_count + limit - 1) / limit
            },
            "network": "base_sepolia",
            "chain_id": 84532
        })),
    }))
}
