use crate::shared::{utils::errors::AppError, AppState};
use std::sync::Arc;
// Commented out until apalis dependency is added
// use apalis::prelude::*;
// use apalis_redis::{Config, ConnectionManager, RedisStorage};

#[derive(Clone)]
pub struct EmailWorker {
    app_state: Arc<AppState>,
}

impl EmailWorker {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    pub async fn start(&self) -> Result<(), AppError> {
        log::info!("Email worker starting...");

        // TODO: Implement email worker when apalis dependency is added
        /*
        let redis_url = &crate::shared::utils::constants::REDIS_URL;
        let storage = RedisStorage::new(
            ConnectionManager::new(Config::new(redis_url)).await
                .map_err(|e| AppError::InternalServerError(format!("Redis connection failed: {}", e)))?
        );

        let worker = WorkerBuilder::new("email-worker")
            .layer(
                TraceLayer::new()
                    .make_span_with(|_: &EmailJob| tracing::info_span!("email-job"))
                    .on_request(|_: &EmailJob, _span: &tracing::Span| {
                        tracing::info!("Processing email job");
                    })
                    .on_response(|_: &EmailJob, _latency: std::time::Duration, _span: &tracing::Span| {
                        tracing::info!("Email job completed");
                    })
            )
            .data(self.app_state.clone())
            .build(storage);

        worker.run().await;
        */

        log::info!("Email worker placeholder started (apalis not configured)");
        Ok(())
    }

    // Commented out until proper dependencies are available
    /*
    pub async fn setup_email_storage(&self) -> Result<RedisStorage, AppError> {
        let redis_url = &crate::shared::utils::constants::REDIS_URL;
        let email_storage = RedisStorage::new(
            ConnectionManager::new(Config::new(redis_url)).await
                .map_err(|e| AppError::InternalServerError(format!("Redis connection failed: {}", e)))?
        );
        Ok(email_storage)
    }
    */
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct EmailJob {
    pub to: String,
    pub subject: String,
    pub body: String,
    pub template: Option<String>,
}

impl EmailJob {
    pub fn new(to: String, subject: String, body: String) -> Self {
        Self {
            to,
            subject,
            body,
            template: None,
        }
    }

    pub fn with_template(to: String, subject: String, template: String) -> Self {
        Self {
            to,
            subject,
            body: String::new(),
            template: Some(template),
        }
    }
}
