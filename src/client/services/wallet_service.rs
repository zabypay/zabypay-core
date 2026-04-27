use crate::client::models::wallet::{WalletGenerationRequest, WalletInfo, WalletResponse};
use crate::shared::{
    entities::{prelude::*, wallet},
    service::wallet::{Currency, WalletService as CoreWalletService},
    utils::errors::AppError,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

impl WalletService {
    fn currency_from_string(currency: &str) -> Result<Currency, AppError> {
        match currency.to_lowercase().as_str() {
            "eth" | "ethereum" => Ok(Currency::Ethereum),
            "btc" | "bitcoin" => Ok(Currency::Bitcoin),
            "usdt" | "tether" => Ok(Currency::USDT),
            _ => Err(AppError::BadRequest("Invalid currency".to_string())),
        }
    }

    pub async fn generate_wallet_for_user(
        db: &DatabaseConnection,
        user_id: &str,
        request: &WalletGenerationRequest,
    ) -> Result<WalletResponse, AppError> {
        // Validate currency
        let _currency = Self::currency_from_string(&request.currency)?;

        // Check if wallet already exists for this user and currency
        let existing_wallet = Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .filter(wallet::Column::Currency.eq(&request.currency))
            .one(db)
            .await?;

        if existing_wallet.is_some() {
            return Err(AppError::BadRequest(format!(
                "Wallet for {} already exists",
                request.currency
            )));
        }

        // Generate wallet using the wallet service
        let wallet_service = CoreWalletService::new();
        let currency = Currency::from_str(&request.currency)?;
        let (address, mnemonic) = wallet_service
            .generate_wallet(user_id, currency, db)
            .await?;

        Ok(WalletResponse {
            address,
            currency: request.currency.to_uppercase(),
            created_at: chrono::Utc::now()
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string(),
        })
    }

    pub async fn get_user_wallets(
        db: &DatabaseConnection,
        user_id: &str,
    ) -> Result<Vec<WalletInfo>, AppError> {
        let wallets = Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .all(db)
            .await?;

        let wallet_responses = wallets
            .into_iter()
            .map(|wallet| WalletInfo {
                id: wallet.id,
                currency: wallet.currency,
                public_key: wallet.public_key,
                created_at: wallet
                    .created_at
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string(),
            })
            .collect();

        Ok(wallet_responses)
    }
}

pub struct WalletService;
