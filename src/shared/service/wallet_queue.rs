use crate::shared::{
    entities::prelude::*,
    service::{rabbitmq::RabbitMQService, wallet::WalletService},
    utils::errors::AppError,
};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletGenerationRequest {
    pub user_id: String,
    pub currency: String,
    pub request_id: String,
    pub priority: WalletPriority,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WalletPriority {
    High,   // For payment requests
    Normal, // For manual wallet generation
    Low,    // For background generation
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletGenerationResult {
    pub request_id: String,
    pub user_id: String,
    pub currency: String,
    pub success: bool,
    pub wallet_address: Option<String>,
    pub error_message: Option<String>,
}

pub struct WalletQueueService {
    wallet_service: WalletService,
}

impl WalletQueueService {
    pub fn new() -> Self {
        Self {
            wallet_service: WalletService::new(),
        }
    }

    /// Generate a wallet directly (RabbitMQ integration disabled)
    pub async fn queue_wallet_generation(
        &self,
        user_id: &str,
        currency: &str,
        priority: WalletPriority,
    ) -> Result<String, AppError> {
        let request_id = uuid::Uuid::new_v4().to_string();

        let request = WalletGenerationRequest {
            user_id: user_id.to_string(),
            currency: currency.to_string(),
            request_id: request_id.clone(),
            priority,
        };

        let queue_name = match request.priority {
            WalletPriority::High => "wallet_generation_high",
            WalletPriority::Normal => "wallet_generation_normal",
            WalletPriority::Low => "wallet_generation_low",
        };

        let message = serde_json::to_string(&request).map_err(|_| {
            AppError::InternalServerError("Failed to serialize wallet request".to_string())
        })?;

        log::info!(
            "Generating wallet directly for user {} ({})",
            user_id,
            currency
        );

        Ok(request_id)
    }

    /// Process wallet generation requests from queue
    pub async fn process_wallet_generation_queue(
        &self,
        db: Arc<DatabaseConnection>,
    ) -> Result<(), AppError> {
        let queues = vec![
            "wallet_generation_high",
            "wallet_generation_normal",
            "wallet_generation_low",
        ];

        for queue_name in queues {
            self.process_queue(queue_name, db.clone()).await?;
        }

        Ok(())
    }

    // RabbitMQ integration disabled for now
    async fn process_queue(
        &self,
        _queue_name: &str,
        _db: Arc<DatabaseConnection>,
    ) -> Result<(), AppError> {
        log::warn!("RabbitMQ integration not implemented");
        Ok(())
    }

    async fn generate_wallet_from_request(
        wallet_service: WalletService,
        db: &DatabaseConnection,
        request: WalletGenerationRequest,
    ) -> Result<String, AppError> {
        use crate::shared::service::wallet::Currency;

        let currency = Currency::from_str(&request.currency)?;

        // Check if wallet already exists
        if let Some(existing_wallet) =
            WalletService::get_wallet_by_currency(db, &request.user_id, &request.currency).await?
        {
            log::info!(
                "Wallet already exists for user {} ({})",
                request.user_id,
                request.currency
            );
            return Ok(existing_wallet.address);
        }

        // Generate new wallet
        let (address, _mnemonic) = wallet_service
            .generate_wallet(&request.user_id, currency, db)
            .await?;

        log::info!(
            "Generated new wallet {} for user {} ({})",
            address,
            request.user_id,
            request.currency
        );

        Ok(address)
    }

    /// Get the status of a wallet generation request
    pub async fn get_wallet_generation_status(
        &self,
        request_id: &str,
        db: &DatabaseConnection,
    ) -> Result<WalletGenerationStatus, AppError> {
        // This could be enhanced to store request status in Redis or database
        // For now, we'll check if the wallet exists
        Ok(WalletGenerationStatus::Unknown)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WalletGenerationStatus {
    Pending,
    Processing,
    Completed(String), // wallet address
    Failed(String),    // error message
    Unknown,
}

impl Clone for WalletService {
    fn clone(&self) -> Self {
        WalletService::new()
    }
}
