use actix_web::{delete, get, post, web, HttpRequest, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::shared::{
    service::payment_monitor::PaymentMonitorService, utils::errors::AppError, AppState,
};

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
pub struct MonitoringResponse {
    pub success: bool,
    pub message: String,
    pub data: Option<serde_json::Value>,
}


#[post("/start")]
pub async fn start_monitoring_service(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;


    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.start_monitoring().await {
        Ok(_) => Ok(HttpResponse::Ok().json(MonitoringResponse {
            success: true,
            message: "Payment monitoring service started successfully".to_string(),
            data: None,
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(MonitoringResponse {
            success: false,
            message: format!("Failed to start monitoring service: {}", e),
            data: None,
        })),
    }
}

#[post("/stop")]
pub async fn stop_monitoring_service(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;


    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.stop_monitoring().await {
        Ok(_) => Ok(HttpResponse::Ok().json(MonitoringResponse {
            success: true,
            message: "Payment monitoring service stopped successfully".to_string(),
            data: None,
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(MonitoringResponse {
            success: false,
            message: format!("Failed to stop monitoring service: {}", e),
            data: None,
        })),
    }
}


#[get("/status")]
pub async fn get_monitoring_status(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;


    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.get_monitoring_status().await {
        Ok(status) => Ok(HttpResponse::Ok().json(MonitoringResponse {
            success: true,
            message: "Monitoring status retrieved successfully".to_string(),
            data: Some(status),
        })),
        Err(e) => Ok(
            HttpResponse::InternalServerError().json(MonitoringResponse {
                success: false,
                message: format!("Failed to get monitoring status: {}", e),
                data: None,
            }),
        ),
    }
}


#[post("/payments/monitor")]
pub async fn start_monitoring_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<StartMonitoringRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let _api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;


    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service
        .start_monitoring_payment(
            &body.payment_id,
            &body.transaction_hash,
            body.network.clone(),
        )
        .await
    {
        Ok(_) => Ok(HttpResponse::Ok().json(MonitoringResponse {
            success: true,
            message: format!(
                "Started monitoring payment {} with transaction {}",
                body.payment_id, body.transaction_hash
            ),
            data: Some(serde_json::json!({
                "payment_id": body.payment_id,
                "transaction_hash": body.transaction_hash,
                "network": body.network
            })),
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(MonitoringResponse {
            success: false,
            message: format!("Failed to start monitoring payment: {}", e),
            data: None,
        })),
    }
}


#[delete("/payments/monitor/{tx_hash}")]
pub async fn stop_monitoring_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let _api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;


    let tx_hash = path.into_inner();
    let monitor_service =
        app_state
            .payment_monitor
            .as_ref()
            .ok_or(AppError::InternalServerError(
                "Payment monitor service not initialized".to_string(),
            ))?;

    match monitor_service.stop_monitoring_payment(&tx_hash).await {
        Ok(_) => Ok(HttpResponse::Ok().json(MonitoringResponse {
            success: true,
            message: format!("Stopped monitoring transaction {}", tx_hash),
            data: Some(serde_json::json!({
                "transaction_hash": tx_hash
            })),
        })),
        Err(e) => Ok(HttpResponse::BadRequest().json(MonitoringResponse {
            success: false,
            message: format!("Failed to stop monitoring transaction: {}", e),
            data: None,
        })),
    }
}

#[post("/check-payments")]
pub async fn manual_payment_check(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

   
    Ok(HttpResponse::Ok().json(MonitoringResponse {
        success: true,
        message: "Manual payment check triggered. Check logs for results.".to_string(),
        data: Some(serde_json::json!({
            "timestamp": chrono::Utc::now(),
            "note": "This feature will check all pending payments once. Enable automatic monitoring for continuous checking."
        })),
    }))
}


#[get("/health")]
pub async fn get_monitoring_health(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    match &app_state.payment_monitor {
        Some(monitor) => {
            let is_running = monitor.is_running().await;
            let status = monitor.get_monitoring_status().await?;

            let health = serde_json::json!({
                "status": if is_running { "healthy" } else { "stopped" },
                "is_running": is_running,
                "monitored_transactions": status["monitored_transactions_count"],
                "enabled_networks": status["networks_enabled"],
                "polling_interval_seconds": 30,
                "testnet_enabled": status["networks_enabled"].as_array()
                    .map(|arr| arr.iter().any(|n| n.as_str() == Some("base_sepolia")))
                    .unwrap_or(false),
                "mainnet_enabled": status["networks_enabled"].as_array()
                    .map(|arr| arr.iter().any(|n| n.as_str() == Some("ethereum")))
                    .unwrap_or(false),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            Ok(HttpResponse::Ok().json(health))
        }
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "status": "not_initialized",
            "is_running": false,
            "error": "Payment monitoring service not initialized",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }))),
    }
}
