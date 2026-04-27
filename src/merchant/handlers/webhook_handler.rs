use actix_web::{web, HttpResponse};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::shared::{
    entities::{merchant, prelude::*, webhook_log},
    service::webhook_service::{WebhookEvent, WebhookService},
    utils::errors::AppError,
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct ListWebhooksQuery {
    pub page: Option<u64>,
    pub limit: Option<u64>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WebhookLogResponse {
    pub id: String,
    pub merchant_id: String,
    pub event_type: String,
    pub webhook_url: String,
    pub status: String,
    pub attempts: i32,
    pub response_status: Option<i32>,
    pub response_body: Option<String>,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct WebhookListResponse {
    pub webhooks: Vec<WebhookLogResponse>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
}

#[derive(Debug, Deserialize)]
pub struct TestWebhookRequest {
    pub merchant_id: String,
    pub event_type: String,
    pub test_data: Option<serde_json::Value>,
}

pub async fn list_webhooks(
    app_state: web::Data<AppState>,
    merchant_id: web::Path<String>,
    query: web::Query<ListWebhooksQuery>,
) -> Result<HttpResponse, AppError> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let mut query_builder = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()));

    if let Some(status) = &query.status {
        query_builder = query_builder.filter(webhook_log::Column::Status.eq(status));
    }

    let total = query_builder.clone().count(&app_state.db).await?;

    let webhooks = query_builder
        .order_by_desc(webhook_log::Column::CreatedAt)
        .limit(limit)
        .offset(offset)
        .all(&app_state.db)
        .await?;

    let webhook_responses: Vec<WebhookLogResponse> = webhooks
        .into_iter()
        .map(|w| WebhookLogResponse {
            id: w.id,
            merchant_id: w.merchant_id,
            event_type: w.event_type,
            webhook_url: w.webhook_url,
            status: w.status,
            attempts: w.attempts,
            response_status: w.response_status,
            response_body: w.response_body,
            created_at: w.created_at.into(),
            delivered_at: w.delivered_at.map(|dt| dt.into()),
            last_attempt_at: w.last_attempt_at.map(|dt| dt.into()),
            next_retry_at: w.next_retry_at.map(|dt| dt.into()),
        })
        .collect();

    Ok(HttpResponse::Ok().json(WebhookListResponse {
        webhooks: webhook_responses,
        total,
        page,
        limit,
    }))
}

pub async fn get_webhook_details(
    app_state: web::Data<AppState>,
    webhook_id: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let webhook = webhook_log::Entity::find_by_id(webhook_id.as_str())
        .one(&app_state.db)
        .await?
        .ok_or(AppError::NotFound("Webhook not found".to_string()))?;

    let response = WebhookLogResponse {
        id: webhook.id,
        merchant_id: webhook.merchant_id,
        event_type: webhook.event_type,
        webhook_url: webhook.webhook_url,
        status: webhook.status,
        attempts: webhook.attempts,
        response_status: webhook.response_status,
        response_body: webhook.response_body,
        created_at: webhook.created_at.into(),
        delivered_at: webhook.delivered_at.map(|dt| dt.into()),
        last_attempt_at: webhook.last_attempt_at.map(|dt| dt.into()),
        next_retry_at: webhook.next_retry_at.map(|dt| dt.into()),
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn resend_webhook(
    app_state: web::Data<AppState>,
    webhook_id: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let webhook_service = WebhookService::new(Arc::new(app_state.as_ref().clone()));

    webhook_service.resend_webhook(&webhook_id).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": format!("Webhook {} queued for resend", webhook_id)
    })))
}

pub async fn test_webhook(
    app_state: web::Data<AppState>,
    body: web::Json<TestWebhookRequest>,
) -> Result<HttpResponse, AppError> {
    let merchant = merchant::Entity::find_by_id(&body.merchant_id)
        .one(&app_state.db)
        .await?
        .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

    if merchant.webhook_url.is_none() {
        return Err(AppError::ValidationError(
            "Merchant has no webhook URL configured".to_string(),
        ));
    }

    let test_data = body.test_data.clone().unwrap_or_else(|| {
        serde_json::json!({
            "id": format!("test_pay_{}", uuid::Uuid::new_v4()),
            "external_id": "test_order_123",
            "status": "confirmed",
            "amount": "0.001",
            "currency": "ETH",
            "wallet_address": "0x742d35Cc6634C0532925a3b844Bc9e7595f2bd7e",
            "transaction_hash": "0xtest123...",
            "paid_at": Utc::now(),
            "confirmations": 3,
            "network": "ethereum",
            "test": true,
            "note": "This is a test webhook"
        })
    });

    let webhook_service = WebhookService::new(Arc::new(app_state.as_ref().clone()));
    let event = WebhookEvent {
        event: body.event_type.clone(),
        data: test_data,
        timestamp: Utc::now(),
    };

    let webhook_url = merchant.webhook_url.clone().unwrap();
    let webhook_secret = merchant.webhook_secret.unwrap_or_default();

    let _ = webhook_service
        .send_webhook(&merchant.id, &webhook_url, &webhook_secret, &event)
        .await;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "Test webhook sent successfully",
        "event_type": body.event_type,
        "webhook_url": webhook_url
    })))
}

pub async fn get_webhook_stats(
    app_state: web::Data<AppState>,
    merchant_id: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let total_webhooks = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()))
        .count(&app_state.db)
        .await?;

    let delivered_count = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()))
        .filter(webhook_log::Column::Status.eq("delivered"))
        .count(&app_state.db)
        .await?;

    let failed_count = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()))
        .filter(webhook_log::Column::Status.eq("failed"))
        .count(&app_state.db)
        .await?;

    let pending_count = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()))
        .filter(webhook_log::Column::Status.eq("pending"))
        .count(&app_state.db)
        .await?;

    let retrying_count = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()))
        .filter(webhook_log::Column::Status.eq("retrying"))
        .count(&app_state.db)
        .await?;

    let recent_webhooks = webhook_log::Entity::find()
        .filter(webhook_log::Column::MerchantId.eq(merchant_id.as_str()))
        .order_by_desc(webhook_log::Column::CreatedAt)
        .limit(5)
        .all(&app_state.db)
        .await?;

    let recent_webhook_summaries: Vec<_> = recent_webhooks
        .into_iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "event_type": w.event_type,
                "status": w.status,
                "attempts": w.attempts,
                "created_at": w.created_at,
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "total_webhooks": total_webhooks,
        "delivered": delivered_count,
        "failed": failed_count,
        "pending": pending_count,
        "retrying": retrying_count,
        "success_rate": if total_webhooks > 0 {
            (delivered_count as f64 / total_webhooks as f64 * 100.0)
        } else {
            0.0
        },
        "recent_webhooks": recent_webhook_summaries,
    })))
}

use crate::shared::service::webhook_service;
