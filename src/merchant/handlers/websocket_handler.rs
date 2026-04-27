use crate::shared::AppState;
use actix_web::{web, HttpResponse, Result};

pub async fn websocket_payments(
    _req: actix_web::HttpRequest,
    _stream: web::Payload,
    _data: web::Data<AppState>,
) -> Result<HttpResponse> {
    Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "error": "WebSocket service temporarily disabled"
    })))
}

pub async fn websocket_stats(_data: web::Data<AppState>) -> Result<HttpResponse> {
    Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "error": "WebSocket service temporarily disabled"
    })))
}
