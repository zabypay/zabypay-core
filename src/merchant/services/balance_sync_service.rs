use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;
use uuid::Uuid;

use crate::shared::{
    entities::{merchant, payment_request, prelude::*, wallet, wallet_balance},
    service::wallet::{Currency, WalletService},
    utils::{constants, errors::AppError}, // TODO: Re-enable USDT support (ERC20 + BEP20) when stable
    AppState,
};

pub struct BalanceSyncService;

impl BalanceSyncService {
    /// Extract base currency from currency key, handling BNB BEP-20 USDT properly
    fn extract_base_currency_from_key(currency_key: &str, environment: &str) -> String {
        // Remove environment suffix (e.g., "usdt_bnb_mainnet" -> "usdt_bnb")
        let currency_without_env = currency_key
            .strip_suffix(&format!("_{}", environment))
            .unwrap_or(currency_key);

        // Handle specific currency mappings
        match currency_without_env {
            // TODO: Re-enable USDT support (ERC20 + BEP20) when stable
            // "usdt_bnb" => "usdt_bnb".to_string(), // BNB BEP-20 USDT should create a USDT_BNB wallet
            // "usdt" => "usdt".to_string(),         // ETH ERC-20 USDT should create a USDT wallet
            _ => {
                // For other currencies, extract base currency (first part before underscore)
                currency_without_env
                    .split('_')
                    .next()
                    .unwrap_or(currency_without_env)
                    .to_string()
            }
        }
    }
    /// Sync wallet balances from payment data for a specific environment
    pub async fn sync_wallet_balances_from_payments(
        app_state: &AppState,
        merchant_id: &str,
    ) -> Result<(), AppError> {
        // Sync for both environments
        Self::sync_wallet_balances_from_payments_for_environment(app_state, merchant_id, "mainnet")
            .await?;
        Self::sync_wallet_balances_from_payments_for_environment(app_state, merchant_id, "testnet")
            .await?;
        Ok(())
    }

    /// Sync wallet balances from payment data for a specific environment
    pub async fn sync_wallet_balances_from_payments_for_environment(
        app_state: &AppState,
        merchant_id: &str,
        environment: &str,
    ) -> Result<(), AppError> {
        let db = &app_state.db;

        // Get merchant to validate and get user_id
        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        // Get all paid payments for this merchant in the specified environment
        let paid_payments = payment_request::Entity::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Status.eq("paid"))
            .filter(payment_request::Column::Environment.eq(environment))
            .all(db)
            .await?;

        // Group payments by currency and sum amounts
        let mut currency_balances: HashMap<String, Decimal> = HashMap::new();

        log::info!(
            "Balance sync: Found {} paid payments for merchant {} in {} environment",
            paid_payments.len(),
            merchant_id,
            environment
        );

        for payment in &paid_payments {
            let amount = payment.amount;

            // Determine the correct currency for balance tracking
            // Check if this is a BNB BEP-20 USDT payment (currency='usdt' or 'USDT_BNB' on BNB network)
            let actual_currency = if payment.currency.to_lowercase() == "usdt"
                || payment.currency.to_uppercase() == "USDT_BNB"
                || payment.currency.to_lowercase().contains("usdt_bnb")
            {
                // If currency is already USDT_BNB, use it directly
                if payment.currency.to_uppercase() == "USDT_BNB"
                    || payment.currency.to_lowercase().contains("usdt_bnb")
                {
                    log::info!(
                        "Balance sync: Payment {} has USDT_BNB currency, treating as usdt_bnb",
                        payment.id
                    );
                    "usdt_bnb"
                } else {
                    // Check metadata for BNB network indicators (for legacy USDT payments)
                    if let Some(metadata) = &payment.metadata {
                        if let Ok(meta_json) = serde_json::from_str::<serde_json::Value>(metadata) {
                            // Check for BNB transaction hash or network indicators
                            if let Some(tx_hash) =
                                meta_json.get("transaction_hash").and_then(|v| v.as_str())
                            {
                                // BNB transactions are 66 characters long and start with 0x
                                if tx_hash.len() == 66 && tx_hash.starts_with("0x") {
                                    log::info!("Balance sync: Detected BNB transaction {} for payment {}, treating as usdt_bnb",
                                        tx_hash, payment.id);
                                    "usdt_bnb"
                                } else {
                                    payment.currency.as_str()
                                }
                            } else {
                                payment.currency.as_str()
                            }
                        } else {
                            payment.currency.as_str()
                        }
                    } else {
                        payment.currency.as_str()
                    }
                }
            } else {
                payment.currency.as_str()
            };

            // Create environment-specific currency identifier
            let currency_key = format!("{}_{}", actual_currency.to_lowercase(), environment);
            *currency_balances
                .entry(currency_key.clone())
                .or_insert(Decimal::ZERO) += amount;
            log::info!("Balance sync: Processing payment {} - {} {} in {} (status: {}) -> currency_key: {}",
                payment.id, amount, payment.currency, environment, payment.status, currency_key);
        }

        // Get confirmed withdrawals to subtract from available balance
        use crate::shared::entities::withdrawal_request;
        let confirmed_withdrawals = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .filter(withdrawal_request::Column::Environment.eq(environment))
            .filter(withdrawal_request::Column::Status.eq("confirmed"))
            .all(db)
            .await?;

        log::info!(
            "Balance sync: Found {} confirmed withdrawals for merchant {} in {} environment",
            confirmed_withdrawals.len(),
            merchant_id,
            environment
        );

        // Group withdrawals by currency and subtract from balances
        for withdrawal in &confirmed_withdrawals {
            let amount = withdrawal.amount;

            // Normalize withdrawal currency to match payment currency logic
            let withdrawal_currency = withdrawal.currency.to_lowercase();
            let normalized_currency = match withdrawal_currency.as_str() {
                "sol" => "sol",
                "bnb" => "bnb",
                "eth" => "eth",
                "usdt" => "usdt",
                _ => &withdrawal_currency,
            };

            // Try multiple currency key formats to match existing balances
            let possible_currency_keys = vec![
                format!("{}_{}", normalized_currency, environment), // e.g., "bnb_mainnet"
                format!("{}_{}", withdrawal_currency, environment), // e.g., "bnb_mainnet" (original)
                normalized_currency.to_string(),                    // e.g., "bnb"
                withdrawal_currency.clone(),                        // e.g., "bnb" (original)
            ];

            // Find which currency key exists in the balances and subtract from it
            let mut withdrawal_applied = false;
            for currency_key in &possible_currency_keys {
                if currency_balances.contains_key(currency_key) {
                    let current_balance = currency_balances
                        .entry(currency_key.clone())
                        .or_insert(Decimal::ZERO);
                    *current_balance -= amount;

                    log::info!(
                        "Balance sync: Subtracting withdrawal {} - {} {} from {} balance (key: {})",
                        withdrawal.id,
                        amount,
                        withdrawal.currency,
                        environment,
                        currency_key
                    );
                    withdrawal_applied = true;
                    break;
                }
            }

            if !withdrawal_applied {
                // If no matching balance found, create a negative balance entry for this currency
                let currency_key = format!("{}_{}", normalized_currency, environment);
                currency_balances.insert(currency_key.clone(), -amount);
                log::warn!("Balance sync: No matching payment found for withdrawal {} ({}), creating negative balance entry with key: {}",
                    withdrawal.id, withdrawal.currency, currency_key);
                log::info!(
                    "Balance sync: Available payment currency keys: {:?}",
                    currency_balances.keys().collect::<Vec<_>>()
                );
            }
        }

        log::info!("Balance sync: Currency totals: {:?}", currency_balances);

        // Validation: Check for negative balances and log warnings
        for (currency_key, total_amount) in &currency_balances {
            if *total_amount < Decimal::ZERO {
                log::warn!(" Balance sync: Negative balance detected for {} ({}). This suggests more has been withdrawn than paid in.",
                    currency_key, total_amount);
            }
        }

        // For each currency, ensure wallet and balance exist
        for (currency_key, total_amount) in currency_balances {
            log::info!(
                "Balance sync: Processing currency {} with total amount {}",
                currency_key,
                total_amount
            );

            // Find or create wallet for this currency+environment combination
            // First try exact match, then look for payment-specific wallets
            let wallet = match wallet::Entity::find()
                .filter(wallet::Column::UserId.eq(&merchant.user_id))
                .filter(wallet::Column::Currency.eq(&currency_key))
                .one(db)
                .await?
            {
                Some(wallet) => {
                    log::info!(
                        "Balance sync: Found existing wallet {} for currency {}",
                        wallet.id,
                        currency_key
                    );
                    wallet
                }
                None => {
                    // Look for payment-specific wallets (e.g., sol_mainnet_uuid)
                    log::info!(
                        "Balance sync: No exact match for {}, looking for payment-specific wallets",
                        currency_key
                    );
                    match wallet::Entity::find()
                        .filter(wallet::Column::UserId.eq(&merchant.user_id))
                        .filter(wallet::Column::Currency.like(&format!("{}_%", currency_key)))
                        .one(db)
                        .await?
                    {
                        Some(payment_wallet) => {
                            log::info!("Balance sync: Found payment-specific wallet {} for currency pattern {}", payment_wallet.id, currency_key);
                            payment_wallet
                        }
                        None => {
                            log::info!(
                                "Balance sync: Creating new wallet for user {} currency {}",
                                merchant.user_id,
                                currency_key
                            );

                            // Extract base currency from currency_key with proper BNB BEP-20 handling
                            let base_currency =
                                Self::extract_base_currency_from_key(&currency_key, environment);

                            // Convert to Currency enum and create proper wallet
                            match Currency::from_str(&base_currency) {
                                Ok(currency_enum) => {
                                    // Generate proper wallet with keys
                                    let wallet_service = WalletService::new();
                                    let (address, _mnemonic) = wallet_service
                                        .generate_wallet_with_key(
                                            &merchant.user_id,
                                            currency_enum,
                                            &currency_key,
                                            db,
                                        )
                                        .await?;

                                    // Fetch the created wallet
                                    let created_wallet = wallet::Entity::find()
                                        .filter(wallet::Column::UserId.eq(&merchant.user_id))
                                        .filter(wallet::Column::Currency.eq(&currency_key))
                                        .one(db)
                                        .await?
                                        .ok_or_else(|| {
                                            AppError::InternalServerError(
                                                "Failed to retrieve created wallet".to_string(),
                                            )
                                        })?;

                                    log::info!("Balance sync: Created proper wallet {} for currency {} with address {}", created_wallet.id, currency_key, address);
                                    created_wallet
                                }
                                Err(_) => {
                                    log::warn!("Unsupported currency for wallet generation: {}, creating placeholder", currency_key);
                                    // Fallback to placeholder for unsupported currencies
                                    let wallet = wallet::ActiveModel {
                                        id: Set(Uuid::new_v4().to_string()),
                                        user_id: Set(merchant.user_id.clone()),
                                        currency: Set(currency_key.clone()),
                                        address: Set(format!("generated_{}_address", currency_key)),
                                        public_key: Set("".to_string()),
                                        private_key: Set("".to_string()),
                                        mnemonic: Set("".to_string()),
                                        created_at: Set(Utc::now().into()),
                                    };
                                    let created_wallet = wallet.insert(db).await?;
                                    log::info!("Balance sync: Created placeholder wallet {} for currency {}", created_wallet.id, currency_key);
                                    created_wallet
                                }
                            }
                        }
                    }
                }
            };

            // Find or create wallet balance
            let existing_balance = wallet_balance::Entity::find()
                .filter(wallet_balance::Column::WalletId.eq(&wallet.id))
                .one(db)
                .await?;

            match existing_balance {
                Some(balance) => {
                    log::info!(
                        "Balance sync: Updating existing balance {} for wallet {}",
                        balance.id,
                        wallet.id
                    );

                    // Calculate available balance (payments - withdrawals), ensuring it's not negative
                    let available_balance = if total_amount < Decimal::ZERO {
                        Decimal::ZERO
                    } else {
                        total_amount
                    };

                    // Update existing balance
                    let mut balance_active: wallet_balance::ActiveModel = balance.into();
                    balance_active.available_balance = Set(available_balance);
                    balance_active.total_balance = Set(total_amount.max(Decimal::ZERO)); // Total payments received
                    balance_active.pending_balance = Set(Decimal::ZERO);
                    balance_active.last_updated = Set(Utc::now().into());
                    balance_active.update(db).await?;
                    log::info!(
                        "Balance sync: Updated available balance to {} (net: {})",
                        available_balance,
                        total_amount
                    );
                }
                None => {
                    log::info!(
                        "Balance sync: Creating new balance for wallet {}",
                        wallet.id
                    );

                    // Calculate available balance (payments - withdrawals), ensuring it's not negative
                    let available_balance = if total_amount < Decimal::ZERO {
                        Decimal::ZERO
                    } else {
                        total_amount
                    };

                    // Create new balance
                    let balance = wallet_balance::ActiveModel {
                        id: Set(Uuid::new_v4().to_string()),
                        wallet_id: Set(wallet.id.clone()),
                        currency: Set(currency_key.clone()),
                        available_balance: Set(available_balance),
                        pending_balance: Set(Decimal::ZERO),
                        locked_balance: Set(Decimal::ZERO),
                        total_balance: Set(total_amount.max(Decimal::ZERO)), // Total payments received
                        last_updated: Set(Utc::now().into()),
                        created_at: Set(Utc::now().into()),
                    };
                    let created_balance = balance.insert(db).await?;
                    log::info!(
                        "Balance sync: Created balance {} with available {} (net: {})",
                        created_balance.id,
                        available_balance,
                        total_amount
                    );
                }
            }
        }

        Ok(())
    }

    /// Map currency to network for withdrawal system
    fn currency_to_network(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "eth" => "ethereum".to_string(),
            "btc" => "bitcoin".to_string(),
            "sol" => "solana".to_string(),
            "bnb" => "binance".to_string(),
            _ => currency.to_string(),
        }
    }

    /// Get network string for withdrawal API
    pub fn currency_to_withdrawal_network(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "eth" => "eth".to_string(),
            "btc" => "btc".to_string(),
            "sol" => "sol".to_string(),
            "bnb" => "bnb".to_string(),
            "usdt" => "usdt_erc20".to_string(), // Default USDT to ERC-20
            "base_eth" => "base_eth".to_string(),
            _ => currency.to_lowercase(),
        }
    }

    /// Force a balance recalculation for a merchant (useful for debugging)
    pub async fn force_balance_recalculation(
        app_state: &AppState,
        merchant_id: &str,
    ) -> Result<(), AppError> {
        log::info!(
            "🔄 Forcing balance recalculation for merchant {}",
            merchant_id
        );

        // Sync for both environments
        Self::sync_wallet_balances_from_payments_for_environment(app_state, merchant_id, "mainnet")
            .await?;
        Self::sync_wallet_balances_from_payments_for_environment(app_state, merchant_id, "testnet")
            .await?;

        log::info!(
            "✅ Balance recalculation completed for merchant {}",
            merchant_id
        );
        Ok(())
    }
}
