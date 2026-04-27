use crate::{
    merchant::models::merchant::MerchantEnvironmentType,
    shared::{
        entities::{prelude::*, wallet},
        service::{
            wallet::{Currency, WalletService as CoreWalletService},
            wallet_queue::{WalletPriority, WalletQueueService},
        },
        utils::{
            errors::AppError,
            network_config::{EnvironmentType, NetworkConfigManager},
        },
    },
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::sync::Arc;

pub struct WalletGenerationService;

impl WalletGenerationService {
    pub async fn ensure_wallet_for_payment(
        db: &DatabaseConnection,
        user_id: &str,
        currency_str: &str,
        environment: Option<MerchantEnvironmentType>,
    ) -> Result<String, AppError> {
        let env_type = environment.unwrap_or_default();

        let effective_currency = Self::get_effective_currency(currency_str, &env_type)?;
        let normalized_currency = effective_currency.to_lowercase();

        let currency_key = format!("{}_{}", normalized_currency, String::from(env_type.clone()));

        let existing_wallet = Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .filter(wallet::Column::Currency.eq(&currency_key))
            .one(db)
            .await?;

        if let Some(wallet) = existing_wallet {
            log::info!(
                "Using existing {:?} {} wallet for user {}: {}",
                env_type,
                normalized_currency,
                user_id,
                wallet.address
            );
            Ok(wallet.address)
        } else {
            Self::generate_new_wallet_sync(
                db,
                user_id,
                &currency_key,
                &effective_currency,
                env_type,
            )
            .await
        }
    }

    pub async fn queue_wallet_generation(
        wallet_queue: &WalletQueueService,
        user_id: &str,
        currency_str: &str,
        priority: WalletPriority,
    ) -> Result<String, AppError> {
        wallet_queue
            .queue_wallet_generation(user_id, currency_str, priority)
            .await
    }

    async fn generate_new_wallet_sync(
        db: &DatabaseConnection,
        user_id: &str,
        currency_key: &str,
        effective_currency: &str,
        env_type: MerchantEnvironmentType,
    ) -> Result<String, AppError> {
        let base_currency_str = match env_type {
            MerchantEnvironmentType::Testnet => match effective_currency {
                "base_sepolia" => "ethereum",
                other => other,
            },
            MerchantEnvironmentType::Mainnet => effective_currency,
        };

        let currency = Currency::from_str(base_currency_str)?;

        let wallet_service = CoreWalletService::new();
        let (address, _mnemonic) = wallet_service
            .generate_wallet_with_key(user_id, currency, currency_key, db)
            .await?;

        log::info!(
            "Auto-generated {:?} {} wallet for user {}: {}",
            env_type,
            effective_currency,
            user_id,
            address
        );

        Ok(address)
    }

    fn get_effective_currency(
        currency_str: &str,
        env_type: &MerchantEnvironmentType,
    ) -> Result<String, AppError> {
        match env_type {
            MerchantEnvironmentType::Testnet => match currency_str.to_lowercase().as_str() {
                "ethereum" | "eth" | "usdt" | "usdt_bnb" | "usdt_erc20" | "usdt_bep20" | "usdc"
                | "bnb" => Ok("base_sepolia".to_string()),
                "bitcoin" | "btc" => Ok("bitcoin".to_string()),
                other => Err(AppError::ValidationError(format!(
                    "Currency {} not supported on testnet",
                    other
                ))),
            },
            MerchantEnvironmentType::Mainnet => {
                if NetworkConfigManager::get_network_config_by_environment(
                    currency_str,
                    EnvironmentType::Mainnet,
                )
                .is_some()
                {
                    Ok(currency_str.to_string())
                } else {
                    Err(AppError::ValidationError(format!(
                        "Currency {} not supported on mainnet",
                        currency_str
                    )))
                }
            }
        }
    }

    pub fn is_currency_supported(
        currency: &str,
        environment: Option<MerchantEnvironmentType>,
    ) -> bool {
        let env_type = environment.unwrap_or_default();
        Self::get_effective_currency(currency, &env_type).is_ok()
    }

    pub fn get_supported_currencies(environment: Option<MerchantEnvironmentType>) -> Vec<String> {
        let env_type = environment.unwrap_or_default();
        match env_type {
            MerchantEnvironmentType::Testnet => {
                vec!["base_sepolia".to_string()]
            }
            MerchantEnvironmentType::Mainnet => {
                NetworkConfigManager::get_supported_currencies_by_environment(
                    EnvironmentType::Mainnet,
                )
            }
        }
    }

    pub async fn get_wallet_by_currency_and_environment(
        db: &DatabaseConnection,
        user_id: &str,
        currency_str: &str,
        environment: Option<MerchantEnvironmentType>,
    ) -> Result<Option<wallet::Model>, AppError> {
        let env_type = environment.unwrap_or_default();
        let effective_currency = Self::get_effective_currency(currency_str, &env_type)?;
        let currency_key = format!(
            "{}_{}",
            effective_currency.to_lowercase(),
            String::from(env_type)
        );

        Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .filter(wallet::Column::Currency.eq(currency_key))
            .one(db)
            .await
            .map_err(|e| AppError::InternalServerError(format!("Failed to fetch wallet: {}", e)))
    }
}
