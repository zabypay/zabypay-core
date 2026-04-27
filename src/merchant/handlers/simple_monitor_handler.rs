use crate::shared::{service::simple_payment_monitor::SimplePaymentMonitor, AppState};
use actix_web::{web, HttpResponse, Result};
use log;
use serde_json::json;


pub async fn check_payments_now(data: web::Data<AppState>) -> Result<HttpResponse> {
    log::info!(" Manual payment check triggered via API");

    match SimplePaymentMonitor::run_monitoring_cycle(&data).await {
        Ok(result) => {
            log::info!("✅ Manual payment check completed: {}", result);
            Ok(HttpResponse::Ok().json(json!({
                "status": "success",
                "message": result,
                "timestamp": chrono::Utc::now().to_rfc3339()
            })))
        }
        Err(e) => {
            log::error!(" Manual payment check failed: {}", e);
            Ok(HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "error": e.to_string(),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })))
        }
    }
}


pub async fn get_simple_monitoring_stats(data: web::Data<AppState>) -> Result<HttpResponse> {
    use crate::shared::entities::{payment_request, prelude::*};
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    let pending_count = PaymentRequest::find()
        .filter(payment_request::Column::Status.eq("pending"))
        .count(&data.db)
        .await
        .unwrap_or(0);

    let paid_count = PaymentRequest::find()
        .filter(payment_request::Column::Status.eq("paid"))
        .count(&data.db)
        .await
        .unwrap_or(0);

    let expired_count = PaymentRequest::find()
        .filter(payment_request::Column::Status.eq("expired"))
        .count(&data.db)
        .await
        .unwrap_or(0);

    Ok(HttpResponse::Ok().json(json!({
        "simple_monitor_stats": {
            "pending_payments": pending_count,
            "paid_payments": paid_count,
            "expired_payments": expired_count,
            "last_check": chrono::Utc::now().to_rfc3339(),
            "status": "operational"
        }
    })))
}
