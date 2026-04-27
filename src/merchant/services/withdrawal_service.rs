use chrono::Utc;
use rust_decimal::prelude::{FromStr, ToPrimitive};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    merchant::{
        models::withdrawal::{
            BalancesResponse, CreateWithdrawalRequest, FeeEstimateRequest, FeeEstimateResponse,
            MaxWithdrawableRequest, MaxWithdrawableResponse, NetworkBalance, PaginationInfo,
            WithdrawalListQuery, WithdrawalListResponse, WithdrawalResponse, WithdrawalSummary,
        },
        services::BalanceSyncService,
    },
    shared::{
        entities::{
            merchant, payment_request, prelude::*, wallet, wallet_balance, wallet_transaction,
            withdrawal_request,
        },
        models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
        service::{
            usd_conversion_service::UsdConversionService, wallet::WalletService,
            withdrawal_processor::WithdrawalProcessor,
        },
        utils::{constants, errors::AppError},
        AppState,
    },
};

pub struct WithdrawalService;

impl WithdrawalService {
    fn extract_currency_and_network(wallet_currency: &str, environment: &str) -> (String, String) {
        let currency_without_env = wallet_currency
            .strip_suffix(&format!("_{}", environment))
            .unwrap_or(wallet_currency);

        match currency_without_env.to_lowercase().as_str() {
            "usdt_bnb" => ("usdt_bnb".to_string(), "usdt_bep20".to_string()),
            "usdt" => ("usdt".to_string(), "usdt_erc20".to_string()),
            "eth" => ("eth".to_string(), "eth".to_string()),
            "btc" => ("btc".to_string(), "btc".to_string()),
            "sol" => ("sol".to_string(), "sol".to_string()),
            "bnb" => ("bnb".to_string(), "bnb".to_string()),
            _ => {
                let base = currency_without_env
                    .split('_')
                    .next()
                    .unwrap_or(currency_without_env);
                (base.to_string(), base.to_string())
            }
        }
    }

    pub async fn get_balances(
        app_state: &AppState,
        merchant_id: &str,
        environment: &str,
    ) -> Result<BalancesResponse, AppError> {
        log::info!(
            "Getting balances for merchant {} environment {}",
            merchant_id,
            environment
        );

        if let Err(e) = BalanceSyncService::sync_wallet_balances_from_payments_for_environment(
            app_state,
            merchant_id,
            environment,
        )
        .await
        {
            log::warn!(
                "Balance sync failed, continuing with existing data: {:?}",
                e
            );
        }

        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        let wallets = Wallet::find()
            .filter(wallet::Column::UserId.eq(&merchant.user_id))
            .filter(wallet::Column::Currency.contains(&format!("_{}", environment)))
            .all(&app_state.db)
            .await?;

        log::info!(
            "Found {} wallets for user {}",
            wallets.len(),
            merchant.user_id
        );

        let mut balances = Vec::new();
        let mut total_usd_value = Decimal::ZERO;
        let mut last_updated = Utc::now();

        for wallet in &wallets {
            log::info!(
                "Processing wallet {} for currency {}",
                wallet.id,
                wallet.currency
            );
            if let Some(balance) = wallet_balance::Entity::find()
                .filter(wallet_balance::Column::WalletId.eq(&wallet.id))
                .one(&app_state.db)
                .await?
            {
                log::info!(
                    "Found balance {} for wallet {}: available={}, total={}",
                    balance.id,
                    wallet.id,
                    balance.available_balance,
                    balance.total_balance
                );

                let (mut base_currency, mut network_name) =
                    Self::extract_currency_and_network(&wallet.currency, environment);

                if base_currency == "usdt" && network_name == "usdt_erc20" {
                    if let Ok(bnb_usdt_payments) = payment_request::Entity::find()
                        .filter(payment_request::Column::MerchantId.eq(merchant_id))
                        .filter(payment_request::Column::Currency.eq("USDT_BNB"))
                        .filter(payment_request::Column::Status.eq("paid"))
                        .filter(payment_request::Column::Environment.eq(environment))
                        .all(&app_state.db)
                        .await
                    {
                        if !bnb_usdt_payments.is_empty() {
                            log::info!("🔧 Balance API: Found {} BNB USDT payments, correcting balance to BEP-20 format", bnb_usdt_payments.len());
                            base_currency = "usdt_bnb".to_string();
                            network_name = "usdt_bep20".to_string();
                        }
                    }
                }

                let network_info =
                    Self::get_network_info(&base_currency, &network_name, environment);

                balances.push(NetworkBalance {
                    currency: base_currency.clone(),
                    network: network_name,
                    symbol: network_info.symbol,
                    available: balance.available_balance.to_string(),
                    pending: balance.pending_balance.to_string(),
                    total: balance.total_balance.to_string(),
                    decimals: network_info.decimals,
                    updated_at: balance.last_updated.into(),
                });

                if balance.last_updated < last_updated {
                    last_updated = balance.last_updated.into();
                }

                match crate::shared::service::PRICE_ORACLE
                    .get_usd_price(&base_currency)
                    .await
                {
                    Ok(usd_price) => {
                        let usd_decimal = Decimal::from_str(&usd_price.to_string())
                            .unwrap_or_else(|_| Self::get_approximate_usd_price(&base_currency));
                        let currency_usd_value = balance.total_balance * usd_decimal;
                        total_usd_value += currency_usd_value;
                        log::info!(
                            "USD calculation: {} {} × ${} = ${}",
                            balance.total_balance,
                            base_currency,
                            usd_decimal,
                            currency_usd_value
                        );
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to get price for {}: {:?}, using fallback",
                            base_currency,
                            e
                        );
                        let fallback_price = Self::get_approximate_usd_price(&base_currency);
                        let currency_usd_value = balance.total_balance * fallback_price;
                        total_usd_value += currency_usd_value;
                        log::info!(
                            "USD fallback calculation: {} {} × ${} = ${}",
                            balance.total_balance,
                            base_currency,
                            fallback_price,
                            currency_usd_value
                        );
                    }
                }
            } else {
                log::warn!(
                    "No balance found for wallet {} currency {}",
                    wallet.id,
                    wallet.currency
                );
            }
        }

        let response = BalancesResponse {
            environment: environment.to_string(),
            balances: balances.clone(),
            wallet_breakdown: vec![],
            total_usd_value: Some(total_usd_value.to_string()),
            total_wallets: balances.len() as u64,
            last_updated,
        };

        log::info!(
            "Returning {} balances for merchant {}",
            response.balances.len(),
            merchant_id
        );

        Ok(response)
    }

    pub async fn estimate_fee(
        app_state: &AppState,
        merchant_id: &str,
        request: &FeeEstimateRequest,
    ) -> Result<FeeEstimateResponse, AppError> {
        log::info!(
            "🧮 Estimating fee for {} {} withdrawal",
            request.amount,
            request.network
        );

        Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        let balances = Self::get_balances(app_state, merchant_id, &request.environment).await?;

        let network_balance = balances
            .balances
            .iter()
            .find(|b| {
                b.network.to_lowercase() == request.network.to_lowercase()
                    || b.currency.to_lowercase() == request.network.to_lowercase()
            })
            .ok_or_else(|| {
                AppError::ValidationError(format!(
                    "No balance found for network: {}",
                    request.network
                ))
            })?;

        let total_balance = network_balance
            .available
            .parse::<Decimal>()
            .map_err(|_| AppError::ValidationError("Invalid balance amount".to_string()))?;

        let amount = request
            .amount
            .parse::<Decimal>()
            .map_err(|_| AppError::ValidationError("Invalid amount".to_string()))?;

        let network_info = Self::get_network_info(
            &Self::network_to_currency(&request.network),
            &request.network,
            &request.environment,
        );

        let (estimated_fee, gas_price, gas_limit, fee_rate, confirmation_time) =
            Self::calculate_network_fee(&request.network, &request.environment, amount).await?;

        let percentage_buffer = total_balance * "0.001".parse::<Decimal>().unwrap();
        let max_percentage_buffer = total_balance * "0.1".parse::<Decimal>().unwrap();
        let minimum_buffer = "0.000001".parse::<Decimal>().unwrap();

        let safety_buffer = std::cmp::min(
            std::cmp::max(percentage_buffer, minimum_buffer),
            max_percentage_buffer,
        );

        let transferable = if total_balance > (estimated_fee + safety_buffer) {
            total_balance - estimated_fee - safety_buffer
        } else {
            Decimal::ZERO
        };

        let net_amount = if amount > estimated_fee {
            amount - estimated_fee
        } else {
            Decimal::ZERO
        };

        if amount > transferable && transferable > Decimal::ZERO {
            return Err(AppError::ValidationError(format!(
                "Insufficient transferable balance. Maximum transferable: {} {}, Requested: {} {}",
                transferable, network_info.symbol, amount, network_info.symbol
            )));
        }

        if total_balance <= estimated_fee {
            return Err(AppError::ValidationError(format!(
                "Balance too small to cover network fees. Balance: {} {}, Required fee: {} {}",
                total_balance, network_info.symbol, estimated_fee, network_info.symbol
            )));
        }

        log::info!(
            "💰 Fee estimate: balance={}, fee={}, transferable={}, net_amount={}",
            total_balance,
            estimated_fee,
            transferable,
            net_amount
        );

        Ok(FeeEstimateResponse {
            network: request.network.clone(),
            currency: network_info.symbol,
            balance: total_balance.to_string(),
            estimated_fee: estimated_fee.to_string(),
            transferable: transferable.to_string(),
            net_amount: net_amount.to_string(),
            decimals: network_info.decimals,
            gas_price,
            gas_limit,
            fee_rate,
            priority: "medium".to_string(),
            estimated_confirmation_time: confirmation_time,
            safety_buffer: Some(safety_buffer.to_string()),
        })
    }

    pub async fn get_max_withdrawable_amount(
        app_state: &AppState,
        merchant_id: &str,
        request: &MaxWithdrawableRequest,
    ) -> Result<MaxWithdrawableResponse, AppError> {
        log::info!(
            "🧮 Calculating max withdrawable amount for {} {} for merchant {}",
            request.network,
            request.environment,
            merchant_id
        );

        let balances = Self::get_balances(app_state, merchant_id, &request.environment).await?;

        log::info!(
            " Retrieved {} balances from get_balances()",
            balances.balances.len()
        );

        if balances.balances.is_empty() {
            return Err(AppError::ValidationError(format!(
                "No wallet balances found for environment: {}. To withdraw funds, you need to first receive payments on the {} network to create deposit wallets.",
                request.environment,
                request.network.to_uppercase()
            )));
        }

        log::info!(
            " Available balances for environment {}:",
            request.environment
        );
        for balance in &balances.balances {
            log::info!(
                "  - Network: '{}', Currency: '{}', Symbol: '{}', Available: '{}'",
                balance.network,
                balance.currency,
                balance.symbol,
                balance.available
            );
        }
        log::info!(" Looking for network: '{}'", request.network);

        let network_balance = balances.balances.iter()
            .find(|b| {
                let matches_network = b.network.to_lowercase() == request.network.to_lowercase();
                let matches_currency = b.currency.to_lowercase() == request.network.to_lowercase();
                let matches_symbol = b.symbol.to_lowercase() == request.network.to_uppercase();

                log::debug!(" Checking balance: network='{}' currency='{}' symbol='{}' against request='{}'",
                           b.network, b.currency, b.symbol, request.network);
                log::debug!("  matches_network={}, matches_currency={}, matches_symbol={}",
                           matches_network, matches_currency, matches_symbol);

                matches_network || matches_currency || matches_symbol
            })
            .ok_or_else(|| {
                let available_networks: Vec<String> = balances.balances.iter()
                    .map(|b| format!("'{}' (currency: '{}', symbol: '{}')", b.network, b.currency, b.symbol))
                    .collect();
                AppError::ValidationError(format!(
                    "No balance found for network: '{}'. Available networks: [{}]",
                    request.network,
                    available_networks.join(", ")
                ))
            })?;

        let total_balance = network_balance
            .available
            .parse::<Decimal>()
            .map_err(|_| AppError::ValidationError("Invalid balance amount".to_string()))?;

        if total_balance <= Decimal::ZERO {
            return Ok(MaxWithdrawableResponse {
                network: request.network.clone(),
                currency: network_balance.symbol.clone(),
                max_withdrawable: "0".to_string(),
                total_balance: network_balance.available.clone(),
                estimated_fee: "0".to_string(),
                gas_price: None,
                gas_limit: None,
                wallets_available: balances.total_wallets as u32,
            });
        }

        if request.network.to_lowercase() == "eth" {
            return Self::calculate_eth_max_withdrawable(
                app_state,
                merchant_id,
                request,
                network_balance,
                total_balance,
            )
            .await;
        }

        let (estimated_fee, gas_price, gas_limit, _, _) =
            Self::calculate_network_fee(&request.network, &request.environment, total_balance)
                .await?;

        let max_withdrawable = if total_balance > estimated_fee {
            total_balance - estimated_fee
        } else {
            Decimal::ZERO
        };

        log::info!(
            "💰 Max withdrawable calculation: total={}, fee={}, max={}",
            total_balance,
            estimated_fee,
            max_withdrawable
        );

        Ok(MaxWithdrawableResponse {
            network: request.network.clone(),
            currency: network_balance.symbol.clone(),
            max_withdrawable: max_withdrawable.to_string(),
            total_balance: network_balance.available.clone(),
            estimated_fee: estimated_fee.to_string(),
            gas_price,
            gas_limit,
            wallets_available: balances.total_wallets as u32,
        })
    }

    async fn calculate_eth_max_withdrawable(
        app_state: &AppState,
        merchant_id: &str,
        request: &MaxWithdrawableRequest,
        network_balance: &NetworkBalance,
        total_balance: Decimal,
    ) -> Result<MaxWithdrawableResponse, AppError> {
        use crate::merchant::services::evm_multi_wallet_service::EvmMultiWalletService;

        log::info!("⚡ Using ETH multi-wallet service for accurate gas estimation");

        let preview_result = EvmMultiWalletService::calculate_net_withdrawable_amount(
            &app_state.db,
            merchant_id,
            &request.network,
            &request.environment,
            &total_balance.to_string(),
        )
        .await;

        match preview_result {
            Ok(preview) => {
                let max_withdrawable = preview
                    .net_amount
                    .parse::<Decimal>()
                    .unwrap_or(Decimal::ZERO);
                log::info!(
                    "✅ ETH max withdrawable: {} ETH (after {} ETH gas fees)",
                    max_withdrawable,
                    preview.estimated_gas_fee
                );

                Ok(MaxWithdrawableResponse {
                    network: request.network.clone(),
                    currency: network_balance.symbol.clone(),
                    max_withdrawable: preview.net_amount,
                    total_balance: network_balance.available.clone(),
                    estimated_fee: preview.estimated_gas_fee,
                    gas_price: None,
                    gas_limit: None,
                    wallets_available: 1,
                })
            }
            Err(_) => {
                log::warn!(" ETH multi-wallet service failed, falling back to basic calculation");
                let fallback_fee = "0.001".parse::<Decimal>().unwrap_or(Decimal::ZERO);
                let max_withdrawable = if total_balance > fallback_fee {
                    total_balance - fallback_fee
                } else {
                    Decimal::ZERO
                };

                Ok(MaxWithdrawableResponse {
                    network: request.network.clone(),
                    currency: network_balance.symbol.clone(),
                    max_withdrawable: max_withdrawable.to_string(),
                    total_balance: network_balance.available.clone(),
                    estimated_fee: fallback_fee.to_string(),
                    gas_price: Some("20000000000".to_string()),
                    gas_limit: Some("21000".to_string()),
                    wallets_available: 1,
                })
            }
        }
    }

    pub async fn create_withdrawal(
        app_state: &AppState,
        merchant_id: &str,
        request: CreateWithdrawalRequest,
    ) -> Result<WithdrawalResponse, AppError> {
        log::info!(
            "WithdrawalService::create_withdrawal called for merchant {} with request: {:?}",
            merchant_id,
            request
        );

        if !*constants::ENABLE_USDT {
            let network_lower = request.network.to_lowercase();
            if network_lower.contains("usdt")
                || network_lower == "usdt_erc20"
                || network_lower == "usdt_bep20"
            {
                return Err(AppError::ValidationError(
                    "USDT withdrawals are temporarily disabled. Please use ETH, BNB, or SOL instead.".to_string()
                ));
            }
        }

        request.validate_amount_for_network().map_err(|e| {
            log::error!("Amount validation failed: {}", e);
            AppError::ValidationError(e)
        })?;

        log::info!("Amount validation passed");

        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await
            .map_err(|e| {
                log::error!("Database error finding merchant: {:?}", e);
                AppError::DatabaseError(format!("Failed to find merchant: {}", e))
            })?
            .ok_or_else(|| {
                log::error!("Merchant {} not found", merchant_id);
                AppError::NotFound("Merchant not found".to_string())
            })?;

        log::info!(
            "Merchant {} found, checking for duplicate idempotency key: {}",
            merchant_id,
            request.idempotency_key
        );

        log::info!(
            "Checking for duplicate idempotency key: {}",
            request.idempotency_key
        );
        match withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .filter(withdrawal_request::Column::IdempotencyKey.eq(&request.idempotency_key))
            .one(&app_state.db)
            .await
        {
            Ok(Some(existing)) => {
                log::info!(
                    "Found existing withdrawal {} for idempotency key {}, returning existing",
                    existing.id,
                    request.idempotency_key
                );
                return Ok(Self::withdrawal_to_response(&existing));
            }
            Ok(None) => {
                log::info!(
                    "No existing withdrawal for idempotency key {}",
                    request.idempotency_key
                );
            }
            Err(e) => {
                let error_str = e.to_string();
                log::error!("Database error checking idempotency key: {}", error_str);
                log::error!("SQL Error details: {:?}", e);

                if error_str.contains("relation") || error_str.contains("does not exist") {
                    log::error!("withdrawal_request table does not exist!");
                    return Err(AppError::DatabaseError(
                        "Withdrawal system database tables are not configured. Please run migrations.".to_string()
                    ));
                }

                return Err(AppError::DatabaseError(format!(
                    "Failed to check idempotency key: {}",
                    e
                )));
            }
        }

        log::info!("No duplicate found, proceeding with address validation");

        let currency = Self::network_to_currency(&request.network);
        log::info!(
            "Network {} mapped to currency {}",
            request.network,
            currency
        );

        let currency_enum =
            crate::shared::service::wallet::Currency::from_str(&currency).map_err(|e| {
                log::error!("Failed to parse currency {}: {:?}", currency, e);
                AppError::ValidationError(format!("Unsupported currency: {}", currency))
            })?;

        if !crate::shared::service::wallet::validate_wallet_address(
            &currency_enum,
            &request.to_address,
        )
        .map_err(|e| {
            log::error!("Address validation error: {:?}", e);
            AppError::ValidationError(format!("Address validation failed: {}", e))
        })? {
            log::error!("Invalid destination address: {}", request.to_address);
            return Err(AppError::ValidationError(
                "Invalid destination address".to_string(),
            ));
        }

        log::info!(
            "Address validation passed for {} address: {}",
            currency,
            request.to_address
        );

        let environment_currency = format!("{}_{}", currency, request.environment);
        log::info!("Looking for wallet with currency: {}", environment_currency);

        let network_environment_currency = format!("{}_{}", request.network, request.environment);
        log::info!(
            "Also checking for wallet with network currency: {}",
            network_environment_currency
        );

        let network_env_base = format!("{}_{}", request.network, request.environment);
        log::info!(
            "🎯 Looking for payment-funded wallets with pattern: {}_%",
            network_env_base
        );

        let payment_wallets = Wallet::find()
            .filter(wallet::Column::UserId.eq(&merchant.user_id))
            .filter(wallet::Column::Currency.like(&format!("{}_%", network_env_base)))
            .all(&app_state.db)
            .await
            .map_err(|e| {
                log::error!("Database error finding payment-funded wallets: {:?}", e);
                AppError::DatabaseError(format!("Failed to find payment-funded wallets: {}", e))
            })?;

        let wallet = if !payment_wallets.is_empty() {
            log::info!(
                "🎯 Found {} payment-funded wallets, using the first one",
                payment_wallets.len()
            );
            for wallet in &payment_wallets {
                log::info!(
                    "   💰 Payment-funded wallet: {} (currency: {})",
                    wallet.address,
                    wallet.currency
                );
            }
            payment_wallets[0].clone()
        } else {
            log::error!(
                " No payment-funded wallets found for pattern: {}_%",
                network_env_base
            );

            let all_wallets = Wallet::find()
                .filter(wallet::Column::UserId.eq(&merchant.user_id))
                .all(&app_state.db)
                .await
                .map_err(|e| {
                    AppError::DatabaseError(format!("Failed to query all wallets: {}", e))
                })?;

            log::info!(" Debug - All wallets for user {}:", merchant.user_id);
            for wallet in &all_wallets {
                log::info!(
                    "   📝 Wallet: {} (currency: {})",
                    wallet.address,
                    wallet.currency
                );
            }

            log::info!("No payment-funded wallets found, trying exact currency matches");

            match Wallet::find()
                .filter(wallet::Column::UserId.eq(&merchant.user_id))
                .filter(wallet::Column::Currency.eq(&environment_currency))
                .one(&app_state.db)
                .await
            {
                Ok(Some(wallet)) => wallet,
                Ok(None) => {
                    log::info!(
                        "No wallet found with currency {}, trying network currency {}",
                        environment_currency,
                        network_environment_currency
                    );

                    match Wallet::find()
                        .filter(wallet::Column::UserId.eq(&merchant.user_id))
                        .filter(wallet::Column::Currency.eq(&network_environment_currency))
                        .one(&app_state.db)
                        .await
                    {
                        Ok(Some(wallet)) => wallet,
                        Ok(None) => {
                            log::info!("No wallet found with either currency format, attempting balance sync");

                            if let Err(sync_error) = BalanceSyncService::sync_wallet_balances_from_payments_for_environment(app_state, merchant_id, &request.environment).await {
                                        log::warn!("Balance sync failed: {:?}", sync_error);
                                    }

                            if let Ok(Some(wallet)) = Wallet::find()
                                .filter(wallet::Column::UserId.eq(&merchant.user_id))
                                .filter(wallet::Column::Currency.eq(&network_environment_currency))
                                .one(&app_state.db)
                                .await
                            {
                                wallet
                            } else if let Ok(Some(wallet)) = Wallet::find()
                                .filter(wallet::Column::UserId.eq(&merchant.user_id))
                                .filter(wallet::Column::Currency.eq(&environment_currency))
                                .one(&app_state.db)
                                .await
                            {
                                wallet
                            } else {
                                return Err(AppError::ValidationError(format!(
                                            "No {} wallet found for {} environment. Please ensure you have received payments in this currency first.",
                                            request.network, request.environment
                                        )));
                            }
                        }
                        Err(e) => {
                            log::error!(
                                "Database error finding wallet with network currency: {:?}",
                                e
                            );
                            return Err(AppError::DatabaseError(format!(
                                "Failed to find wallet: {}",
                                e
                            )));
                        }
                    }
                }
                Err(e) => {
                    log::error!("Database error finding wallet: {:?}", e);
                    return Err(AppError::DatabaseError(format!(
                        "Failed to find wallet: {}",
                        e
                    )));
                }
            }
        };

        log::info!("Looking for balance for wallet {}", wallet.id);
        let mut balance = match wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.eq(&wallet.id))
            .one(&app_state.db)
            .await
        {
            Ok(Some(balance)) => {
                log::info!(
                    "Found balance for wallet {}: available={}, total={}, pending={}",
                    wallet.id,
                    balance.available_balance,
                    balance.total_balance,
                    balance.pending_balance
                );
                balance
            }
            Ok(None) => {
                log::error!("No balance record found for wallet {}", wallet.id);
                return Err(AppError::NotFound(format!(
                    "No balance found for {} wallet. Please ensure you have received payments first.",
                    request.network
                )));
            }
            Err(e) => {
                let error_str = e.to_string();
                log::error!("Database error finding wallet balance: {}", error_str);
                log::error!("SQL Error details: {:?}", e);

                if error_str.contains("relation") || error_str.contains("does not exist") {
                    log::error!("wallet_balance table does not exist!");
                    return Err(AppError::DatabaseError(
                        "Wallet balance system database tables are not configured. Please run migrations.".to_string()
                    ));
                }

                return Err(AppError::DatabaseError(format!(
                    "Failed to find wallet balance: {}",
                    e
                )));
            }
        };

        log::info!(
            "Found balance for wallet {}: available={}, total={}",
            wallet.id,
            balance.available_balance,
            balance.total_balance
        );

        let amount = request.amount.parse::<Decimal>().map_err(|e| {
            log::error!("Failed to parse amount {}: {:?}", request.amount, e);
            AppError::ValidationError("Invalid amount".to_string())
        })?;

        log::info!("Parsed withdrawal amount: {}", amount);

        let (final_amount, original_usd_amount) = if request.is_usd_withdrawal() {
            log::info!(
                "💱 Converting USD amount {} to crypto for {}",
                request.amount,
                request.network
            );

            let crypto_amount = UsdConversionService::convert_usd_to_crypto(
                &request.amount,
                &Self::network_to_currency(&request.network),
                &request.environment,
            )
            .await?;

            let crypto_decimal = crypto_amount.parse::<Decimal>().map_err(|_| {
                AppError::ValidationError("Invalid converted crypto amount".to_string())
            })?;

            log::info!(
                "💱 USD conversion: ${} → {} {}",
                request.amount,
                crypto_amount,
                request.network.to_uppercase()
            );
            (crypto_decimal, Some(request.amount.clone()))
        } else {
            (amount, None)
        };

        // Estimate fees
        log::info!(
            "Calculating network fees for {} on {} in {}",
            final_amount,
            request.network,
            request.environment
        );
        let (estimated_fee, _, _, _, _) =
            Self::calculate_network_fee(&request.network, &request.environment, final_amount)
                .await
                .map_err(|e| {
                    log::error!("Fee calculation failed: {:?}", e);
                    e
                })?;
        let net_amount = final_amount - estimated_fee;

        log::info!(
            "Estimated fee: {}, net amount: {}",
            estimated_fee,
            net_amount
        );

        let total_needed = if request.network == "sol" {
            final_amount + estimated_fee
        } else {
            final_amount + estimated_fee
        };

        log::info!(
            "Balance validation: available={}, amount={}, fee={}, total_needed={}",
            balance.available_balance,
            final_amount,
            estimated_fee,
            total_needed
        );

        let (final_amount, final_net_amount) = if request.network == "sol"
            && balance.available_balance < total_needed
        {
            let max_withdrawable = balance.available_balance - estimated_fee;
            let shortfall = total_needed - balance.available_balance;
            let shortfall_lamports = (shortfall * Decimal::new(1_000_000_000, 0))
                .to_u64()
                .unwrap_or(0);

            if shortfall_lamports <= 10_000 && max_withdrawable > Decimal::ZERO {
                log::info!("🔧 Auto-adjusting Solana withdrawal from {} to {} SOL (shortfall: {} lamports)",
                          amount, max_withdrawable, shortfall_lamports);
                log::info!(
                    "   Original request: {} SOL + {} SOL fee = {} SOL total",
                    amount,
                    estimated_fee,
                    total_needed
                );
                log::info!(
                    "   Adjusted request: {} SOL + {} SOL fee = {} SOL total",
                    max_withdrawable,
                    estimated_fee,
                    max_withdrawable + estimated_fee
                );
                (max_withdrawable, max_withdrawable - estimated_fee)
            } else {
                (amount, net_amount)
            }
        } else {
            (amount, net_amount)
        };

        let final_total_needed = if request.network == "sol" {
            final_amount + estimated_fee
        } else {
            final_amount + estimated_fee
        };

        if balance.available_balance < final_total_needed {
            let balance_lamports = (balance.available_balance * Decimal::new(1_000_000_000, 0))
                .to_u64()
                .unwrap_or(0);
            let needed_lamports = (total_needed * Decimal::new(1_000_000_000, 0))
                .to_u64()
                .unwrap_or(0);
            let fee_lamports = (estimated_fee * Decimal::new(1_000_000_000, 0))
                .to_u64()
                .unwrap_or(0);

            log::error!("Insufficient balance: available={} lamports, needed={} lamports (including {} fee)",
                balance_lamports, needed_lamports, fee_lamports);

            let error_message = if request.network == "sol" {
                let max_withdrawable_lamports = balance_lamports.saturating_sub(fee_lamports);
                let max_withdrawable_sol = max_withdrawable_lamports as f64 / 1_000_000_000.0;
                format!(
                    "Insufficient balance: Need {} lamports (including {} fee) but only have {} lamports available. Maximum withdrawable amount: {} lamports ({:.6} SOL)",
                    needed_lamports,
                    fee_lamports,
                    balance_lamports,
                    max_withdrawable_lamports,
                    max_withdrawable_sol
                )
            } else {
                format!(
                    "Insufficient balance: Need {} lamports (including {} fee) but only have {} lamports available",
                    needed_lamports,
                    fee_lamports,
                    balance_lamports
                )
            };

            return Err(AppError::ValidationError(error_message));
        }

        log::info!("Balance check passed, will update balance after withdrawal");

        let withdrawal_id = Uuid::new_v4().to_string();
        let required_confirmations = match request.environment.as_str() {
            "testnet" => match request.network.as_str() {
                "sol" => 5,
                _ => 1,
            },
            "mainnet" => match request.network.as_str() {
                "ethereum" => 12,
                "bitcoin" => 6,
                "polygon" => 20,
                "base" => 10,
                "sol" => 32,
                _ => 6,
            },
            _ => 6,
        };

        let withdrawal = withdrawal_request::ActiveModel {
            id: Set(withdrawal_id.clone()),
            merchant_id: Set(merchant_id.to_string()),
            wallet_id: Set(wallet.id.clone()),
            external_id: Set(request.external_id.unwrap_or_else(|| withdrawal_id.clone())),
            idempotency_key: Set(request.idempotency_key.clone()),
            environment: Set(request.environment.clone()),
            network: Set(request.network.clone()),
            currency: Set(currency.clone()),
            to_address: Set(request.to_address.clone()),
            amount: Set(final_amount),
            fee: Set(Some(estimated_fee)),
            net_amount: Set(final_net_amount),
            status: Set("pending".to_string()),
            tx_hash: Set(None),
            blockchain_confirmations: Set(None),
            required_confirmations: Set(required_confirmations),
            metadata: Set(None),
            error_message: Set(None),
            created_at: Set(Utc::now().into()),
            processed_at: Set(None),
            confirmed_at: Set(None),
            updated_at: Set(Utc::now().into()),
            broadcast_at: Set(None),
            failed_at: Set(None),
            failure_reason: Set(None),
            finality_required: Set(required_confirmations),
            finality_reached_at: Set(None),
            replaced_by_tx: Set(None),
            reorg_depth: Set(0),
            lifecycle_state: Set(WithdrawalLifecycleState::Quoted.to_string()),
            withdrawal_funds_state: Set(WithdrawalFundsState::Locked.to_string()),
        };

        log::info!(
            "Creating withdrawal request in database with id: {}",
            withdrawal_id
        );
        let created_withdrawal = match withdrawal.insert(&app_state.db).await {
            Ok(created) => created,
            Err(e) => {
                log::error!("Database error inserting withdrawal: {:?}", e);
                let error_str = e.to_string();
                if error_str.contains("relation")
                    || error_str.contains("table")
                    || error_str.contains("no such table")
                    || error_str.contains("column")
                {
                    return Err(AppError::ValidationError(
                        "Withdrawal system is not fully configured. Database tables are missing."
                            .to_string(),
                    ));
                }
                return Err(AppError::DatabaseError(format!(
                    "Failed to create withdrawal: {}",
                    e
                )));
            }
        };

        log::info!("Withdrawal created successfully, locking funds instead of deducting");

        let total_lock_amount = final_amount + estimated_fee;
        let new_available_balance = balance.available_balance - total_lock_amount;
        let new_locked_balance = balance.locked_balance + total_lock_amount;

        log::info!(
            "Locking funds: available {} -> {}, locked {} -> {} (locked {})",
            balance.available_balance,
            new_available_balance,
            balance.locked_balance,
            new_locked_balance,
            total_lock_amount
        );

        let balance_model = wallet_balance::ActiveModel {
            id: Set(balance.id.clone()),
            available_balance: Set(new_available_balance),
            pending_balance: Set(balance.pending_balance),
            locked_balance: Set(new_locked_balance),
            total_balance: Set(balance.total_balance),
            last_updated: Set(Utc::now().into()),
            ..Default::default()
        };
        balance_model.update(&app_state.db).await.map_err(|e| {
            log::error!("Database error updating balance: {:?}", e);
            AppError::DatabaseError(format!("Failed to update balance: {}", e))
        })?;

        let transaction = wallet_transaction::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            wallet_id: Set(wallet.id),
            transaction_type: Set("withdrawal_lock".to_string()),
            currency: Set(currency),
            amount: Set(final_amount),
            fee: Set(Some(estimated_fee)),
            net_amount: Set(Decimal::ZERO),
            balance_before: Set(balance.available_balance),
            balance_after: Set(new_available_balance),
            tx_hash: Set(None),
            from_address: Set(Some(wallet.address)),
            to_address: Set(Some(request.to_address)),
            status: Set("locked".to_string()),
            description: Set(Some(
                "Withdrawal funds locked - awaiting confirmation".to_string(),
            )),
            metadata: Set(Some(
                serde_json::json!({
                    "withdrawal_id": withdrawal_id,
                    "environment": request.environment,
                    "network": request.network,
                    "locked_amount": total_lock_amount,
                    "safe_withdrawal": true
                })
                .to_string(),
            )),
            related_entity_type: Set(Some("withdrawal_request".to_string())),
            related_entity_id: Set(Some(withdrawal_id.clone())),
            created_at: Set(Utc::now().into()),
            processed_at: Set(None),
        };

        log::info!("Creating wallet transaction record");
        match transaction.insert(&app_state.db).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Database error inserting wallet transaction: {:?}", e);
                let error_str = e.to_string();
                if error_str.contains("relation")
                    || error_str.contains("table")
                    || error_str.contains("no such table")
                    || error_str.contains("column")
                {
                    log::warn!("Wallet transaction table missing, skipping transaction record");
                } else {
                    return Err(AppError::DatabaseError(format!(
                        "Failed to create wallet transaction: {}",
                        e
                    )));
                }
            }
        }

        log::info!("Wallet transaction created successfully");

        log::info!(
            "Processing withdrawal {} synchronously and waiting for transaction hash...",
            created_withdrawal.id
        );

        let processing_timeout = tokio::time::Duration::from_secs(30);

        match tokio::time::timeout(
            processing_timeout,
            WithdrawalProcessor::process_withdrawal(app_state, &created_withdrawal.id),
        )
        .await
        {
            Ok(processing_result) => match processing_result {
                Ok(tx_hash) => {
                    log::info!(
                        "✅ Withdrawal {} processed successfully with tx_hash: {}",
                        created_withdrawal.id,
                        tx_hash
                    );

                    let updated_withdrawal = withdrawal_request::Entity::find()
                        .filter(withdrawal_request::Column::Id.eq(&created_withdrawal.id))
                        .one(&app_state.db)
                        .await
                        .map_err(|e| {
                            log::error!("Failed to fetch updated withdrawal record: {}", e);
                            AppError::DatabaseError(format!(
                                "Failed to fetch updated withdrawal: {}",
                                e
                            ))
                        })?
                        .ok_or_else(|| {
                            log::error!("Updated withdrawal record not found");
                            AppError::NotFound("Updated withdrawal record not found".to_string())
                        })?;

                    log::info!(
                        "Withdrawal processing completed successfully: {}",
                        updated_withdrawal.id
                    );
                    Ok(Self::withdrawal_to_response(&updated_withdrawal))
                }
                Err(e) => {
                    log::error!(
                        " Withdrawal {} processing failed: {:?}",
                        created_withdrawal.id,
                        e
                    );

                    let failed_withdrawal = withdrawal_request::Entity::find()
                        .filter(withdrawal_request::Column::Id.eq(&created_withdrawal.id))
                        .one(&app_state.db)
                        .await
                        .map_err(|db_e| {
                            log::error!("Failed to fetch failed withdrawal record: {}", db_e);
                            AppError::DatabaseError(format!(
                                "Failed to fetch failed withdrawal: {}",
                                db_e
                            ))
                        })?
                        .ok_or_else(|| {
                            log::error!("Failed withdrawal record not found");
                            AppError::NotFound("Failed withdrawal record not found".to_string())
                        })?;

                    log::info!(
                        "Returning failed withdrawal response: {}",
                        failed_withdrawal.id
                    );
                    Ok(Self::withdrawal_to_response(&failed_withdrawal))
                }
            },
            Err(_timeout_error) => {
                log::warn!(" Withdrawal {} processing timed out after 30 seconds, switching to async processing...", created_withdrawal.id);

                let withdrawal_id_for_spawn = created_withdrawal.id.clone();
                let app_state_clone = app_state.clone();
                tokio::spawn(async move {
                    log::info!(
                        "🔄 Processing timed-out withdrawal {} in background...",
                        withdrawal_id_for_spawn
                    );
                    match WithdrawalProcessor::process_withdrawal(
                        &app_state_clone,
                        &withdrawal_id_for_spawn,
                    )
                    .await
                    {
                        Ok(tx_hash) => {
                            log::info!(
                                "✅ Background withdrawal {} completed with tx_hash: {}",
                                withdrawal_id_for_spawn,
                                tx_hash
                            );
                        }
                        Err(e) => {
                            log::error!(
                                " Background withdrawal {} failed: {:?}",
                                withdrawal_id_for_spawn,
                                e
                            );
                        }
                    }
                });

                log::info!(
                    "Returning pending withdrawal response due to timeout: {}",
                    created_withdrawal.id
                );
                Ok(Self::withdrawal_to_response(&created_withdrawal))
            }
        }
    }

    pub async fn process_withdrawal(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<String, AppError> {
        WithdrawalProcessor::process_withdrawal(app_state, withdrawal_id).await
    }

    pub async fn monitor_withdrawal(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<(), AppError> {
        WithdrawalProcessor::monitor_transaction(app_state, withdrawal_id).await
    }

    pub async fn list_withdrawals(
        app_state: &AppState,
        merchant_id: &str,
        query: WithdrawalListQuery,
    ) -> Result<WithdrawalListResponse, AppError> {
        log::info!(
            "WithdrawalService::list_withdrawals called for merchant {} with query {:?}",
            merchant_id,
            query
        );

        let merchant_exists = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await
            .map_err(|e| {
                log::error!(
                    "Database error validating merchant {}: {:?}",
                    merchant_id,
                    e
                );
                AppError::DatabaseError(format!("Failed to validate merchant: {}", e))
            })?;

        if merchant_exists.is_none() {
            log::warn!("Merchant {} not found", merchant_id);
            return Ok(WithdrawalListResponse {
                withdrawals: vec![],
                pagination: PaginationInfo {
                    page: 1,
                    limit: 50,
                    total: 0,
                    total_pages: 0,
                    has_next: false,
                    has_prev: false,
                },
                summary: WithdrawalSummary {
                    total_count: 0,
                    pending_count: 0,
                    processing_count: 0,
                    confirmed_count: 0,
                    failed_count: 0,
                    total_amount_usd: Some("0.00".to_string()),
                    total_fees_usd: Some("0.00".to_string()),
                },
            });
        }

        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(20).min(100).max(1);
        let offset = (page - 1) * limit;

        let mut withdrawal_query = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .order_by_desc(withdrawal_request::Column::CreatedAt);

        if let Some(status) = &query.status {
            withdrawal_query =
                withdrawal_query.filter(withdrawal_request::Column::Status.eq(status));
        }

        if let Some(network) = &query.network {
            withdrawal_query =
                withdrawal_query.filter(withdrawal_request::Column::Network.eq(network));
        }

        if let Some(environment) = &query.environment {
            withdrawal_query =
                withdrawal_query.filter(withdrawal_request::Column::Environment.eq(environment));
        }

        if let Some(from_date) = query.from_date {
            withdrawal_query =
                withdrawal_query.filter(withdrawal_request::Column::CreatedAt.gte(from_date));
        }

        if let Some(to_date) = query.to_date {
            withdrawal_query =
                withdrawal_query.filter(withdrawal_request::Column::CreatedAt.lte(to_date));
        }

        log::info!("Counting withdrawals for merchant {}", merchant_id);
        let total = match withdrawal_query.clone().count(&app_state.db).await {
            Ok(count) => count,
            Err(e) => {
                log::error!(
                    "Database error counting withdrawals (table may not exist): {:?}",
                    e
                );
                if e.to_string().contains("relation")
                    || e.to_string().contains("table")
                    || e.to_string().contains("no such table")
                {
                    log::warn!("withdrawal_request table does not exist, returning empty results");
                    return Ok(WithdrawalListResponse {
                        withdrawals: vec![],
                        pagination: PaginationInfo {
                            page: 1,
                            limit: 50,
                            total: 0,
                            total_pages: 0,
                            has_next: false,
                            has_prev: false,
                        },
                        summary: WithdrawalSummary {
                            total_count: 0,
                            pending_count: 0,
                            processing_count: 0,
                            confirmed_count: 0,
                            failed_count: 0,
                            total_amount_usd: Some("0.00".to_string()),
                            total_fees_usd: Some("0.00".to_string()),
                        },
                    });
                }
                return Err(AppError::DatabaseError(format!(
                    "Failed to count withdrawals: {}",
                    e
                )));
            }
        };

        log::info!(
            "Found {} total withdrawals for merchant {}",
            total,
            merchant_id
        );

        let total_pages = if limit > 0 {
            (total + limit - 1) / limit
        } else {
            0
        };

        log::info!(
            "Fetching withdrawals with offset={}, limit={}",
            offset,
            limit
        );
        let withdrawals = match withdrawal_query
            .offset(offset)
            .limit(limit)
            .all(&app_state.db)
            .await
        {
            Ok(results) => results,
            Err(e) => {
                log::error!(
                    "Database error fetching withdrawals (table may not exist): {:?}",
                    e
                );
                if e.to_string().contains("relation")
                    || e.to_string().contains("table")
                    || e.to_string().contains("no such table")
                {
                    log::warn!("withdrawal_request table does not exist, returning empty results");
                    vec![]
                } else {
                    return Err(AppError::DatabaseError(format!(
                        "Failed to fetch withdrawals: {}",
                        e
                    )));
                }
            }
        };

        log::info!(
            "Converting {} withdrawals to response format",
            withdrawals.len()
        );
        let withdrawal_responses: Vec<WithdrawalResponse> = withdrawals
            .into_iter()
            .map(|w| {
                log::debug!("Converting withdrawal {} to response", w.id);
                Self::withdrawal_to_response(&w)
            })
            .collect();

        // Calculate summary
        log::info!(
            "Calculating withdrawal summary for merchant {}",
            merchant_id
        );
        let summary = match Self::calculate_withdrawal_summary(app_state, merchant_id).await {
            Ok(summary) => summary,
            Err(e) => {
                log::error!("Failed to calculate withdrawal summary: {:?}", e);
                if e.to_string().contains("relation")
                    || e.to_string().contains("table")
                    || e.to_string().contains("no such table")
                {
                    log::warn!("withdrawal_request table does not exist, using empty summary");
                    WithdrawalSummary {
                        total_count: 0,
                        pending_count: 0,
                        processing_count: 0,
                        confirmed_count: 0,
                        failed_count: 0,
                        total_amount_usd: Some("0.00".to_string()),
                        total_fees_usd: Some("0.00".to_string()),
                    }
                } else {
                    return Err(e);
                }
            }
        };

        Ok(WithdrawalListResponse {
            withdrawals: withdrawal_responses,
            pagination: PaginationInfo {
                page,
                limit,
                total,
                total_pages,
                has_next: page < total_pages,
                has_prev: page > 1,
            },
            summary,
        })
    }

    pub fn withdrawal_to_response(withdrawal: &withdrawal_request::Model) -> WithdrawalResponse {
        let mut tx_hashes = vec![];
        let mut single_tx_hash = None;
        if let Some(hash) = &withdrawal.tx_hash {
            if !hash.is_empty() && hash != "pending" && hash != "null" && hash != "undefined" {
                tx_hashes.push(hash.clone());
                single_tx_hash = Some(hash.clone());
            }
        }

        let display_network = Self::map_network_for_display(&withdrawal.network);
        let display_currency = Self::map_currency_for_display(&withdrawal.currency);

        let explorer_urls = if let Some(hash) = &withdrawal.tx_hash {
            if !hash.is_empty() && hash != "pending" && hash != "null" && hash != "undefined" {
                match withdrawal.network.to_lowercase().as_str() {
                    "sol" | "solana" => vec![format!(
                        "https://explorer.solana.com/tx/{}?cluster={}",
                        hash,
                        if withdrawal.environment == "mainnet" {
                            "mainnet-beta"
                        } else {
                            "testnet"
                        }
                    )],
                    "eth" | "ethereum" => vec![format!(
                        "https://{}etherscan.io/tx/{}",
                        if withdrawal.environment == "mainnet" {
                            ""
                        } else {
                            "sepolia."
                        },
                        hash
                    )],
                    "bnb" | "bsc" => vec![format!(
                        "https://{}bscscan.com/tx/{}",
                        if withdrawal.environment == "mainnet" {
                            ""
                        } else {
                            "testnet."
                        },
                        hash
                    )],
                    "btc" | "bitcoin" => vec![format!(
                        "https://{}blockstream.info/tx/{}",
                        if withdrawal.environment == "mainnet" {
                            ""
                        } else {
                            "blockstream.info/testnet/"
                        },
                        hash
                    )],
                    _ => {
                        if withdrawal.currency.to_lowercase().contains("bnb") {
                            vec![format!(
                                "https://{}bscscan.com/tx/{}",
                                if withdrawal.environment == "mainnet" {
                                    ""
                                } else {
                                    "testnet."
                                },
                                hash
                            )]
                        } else {
                            vec![format!("https://etherscan.io/tx/{}", hash)] // Default to Ethereum
                        }
                    }
                }
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let usd_equivalent = if withdrawal.amount.is_zero() {
            None
        } else {
            let price = Self::get_approximate_usd_price(&display_currency);
            if price.is_zero() {
                None
            } else {
                let usd_value = withdrawal.amount * price;
                Some(format!("{:.2}", usd_value))
            }
        };

        let net_amount =
            if !withdrawal.net_amount.is_zero() && withdrawal.net_amount >= Decimal::ZERO {
                withdrawal.net_amount.to_string()
            } else {
                let fee = withdrawal.fee.unwrap_or_default();
                let calculated_net = withdrawal.amount - fee;
                if calculated_net >= Decimal::ZERO {
                    calculated_net.to_string()
                } else {
                    "0.00".to_string()
                }
            };

        WithdrawalResponse {
            id: withdrawal.id.clone(),
            merchant_id: withdrawal.merchant_id.clone(),
            external_id: Some(withdrawal.external_id.clone()),
            idempotency_key: withdrawal.idempotency_key.clone(),
            environment: withdrawal.environment.clone(),
            network: display_network,
            currency: display_currency,
            to_address: withdrawal.to_address.clone(),
            amount: withdrawal.amount.to_string(),
            requested_amount: withdrawal.amount.to_string(),
            transferred_amount: withdrawal.amount.to_string(),
            amount_type: "crypto".to_string(),
            usd_equivalent,
            fee: withdrawal.fee.map(|f| {
                if f >= Decimal::ZERO {
                    f.to_string()
                } else {
                    "0.00000".to_string()
                }
            }),
            total_fee: withdrawal.fee.map(|f| {
                if f >= Decimal::ZERO {
                    f.to_string()
                } else {
                    "0.00000".to_string()
                }
            }),
            net_amount,
            status: withdrawal.status.clone(),
            wallets_used: vec![],
            tx_hash: single_tx_hash,
            tx_hashes,
            confirmations: withdrawal.blockchain_confirmations.unwrap_or(0) as i32,
            blockchain_confirmations: withdrawal.blockchain_confirmations,
            required_confirmations: withdrawal.required_confirmations,
            explorer_url: explorer_urls.first().cloned(),
            explorer_urls,
            created_at: withdrawal.created_at.into(),
            processed_at: withdrawal.processed_at.map(|dt| dt.into()),
            confirmed_at: withdrawal.confirmed_at.map(|dt| dt.into()),
            updated_at: withdrawal.updated_at.into(),
            broadcast_at: withdrawal.broadcast_at.map(|dt| dt.into()),
            failed_at: withdrawal.failed_at.map(|dt| dt.into()),
            failure_reason: withdrawal.failure_reason.clone(),
        }
    }

    async fn calculate_network_fee(
        network: &str,
        environment: &str,
        amount: Decimal,
    ) -> Result<
        (
            Decimal,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        AppError,
    > {
        let (fee, gas_price, gas_limit, fee_rate, time) = match network {
            "eth" | "usdt_erc20" => {
                let base_fee = if environment == "mainnet" {
                    "20000000000"
                } else {
                    "1000000000"
                };
                let gas = "21000";
                let fee_wei = Decimal::from_str_exact(base_fee).unwrap()
                    * Decimal::from_str_exact(gas).unwrap();
                let fee_eth = fee_wei / Decimal::from_str_exact("1000000000000000000").unwrap(); // Convert wei to ETH
                (
                    fee_eth,
                    Some(base_fee.to_string()),
                    Some(gas.to_string()),
                    None,
                    Some("2-5 minutes".to_string()),
                )
            }
            "bnb" | "usdt_bep20" => {
                let base_fee = "5000000000"; // 5 Gwei
                let gas = "21000";
                let fee_wei = Decimal::from_str_exact(base_fee).unwrap()
                    * Decimal::from_str_exact(gas).unwrap();
                let fee_bnb = fee_wei / Decimal::from_str_exact("1000000000000000000").unwrap();
                (
                    fee_bnb,
                    Some(base_fee.to_string()),
                    Some(gas.to_string()),
                    None,
                    Some("30-60 seconds".to_string()),
                )
            }
            "btc" => {
                let sat_per_byte = if environment == "mainnet" { 20 } else { 10 };
                let tx_size = 250;
                let fee_sats = sat_per_byte * tx_size;
                let fee_btc = Decimal::from_str_exact(&fee_sats.to_string()).unwrap()
                    / Decimal::from_str_exact("100000000").unwrap();
                (
                    fee_btc,
                    None,
                    None,
                    Some(sat_per_byte.to_string()),
                    Some("10-30 minutes".to_string()),
                )
            }
            "sol" => {
                let fee_lamports = 5000; // 0.000005 SOL
                let fee_sol = Decimal::from_str_exact(&fee_lamports.to_string()).unwrap()
                    / Decimal::from_str_exact("1000000000").unwrap();
                (fee_sol, None, None, None, Some("1-2 seconds".to_string()))
            }
            "base_eth" => {
                let base_fee = if environment == "mainnet" {
                    "5000000000"
                } else {
                    "2000000000"
                };
                let gas = "21000";
                let fee_wei = Decimal::from_str_exact(base_fee).unwrap()
                    * Decimal::from_str_exact(gas).unwrap();
                let fee_eth = fee_wei / Decimal::from_str_exact("1000000000000000000").unwrap();
                (
                    fee_eth,
                    Some(base_fee.to_string()),
                    Some(gas.to_string()),
                    None,
                    Some("1-2 seconds".to_string()),
                )
            }
            _ => {
                return Err(AppError::ValidationError(format!(
                    "Unsupported network: {}",
                    network
                )))
            }
        };

        Ok((fee, gas_price, gas_limit, fee_rate, time))
    }

    fn get_approximate_usd_price(currency: &str) -> Decimal {
        match currency {
            "ethereum" | "eth" => Decimal::new(320000, 2), // $3200.00
            "bitcoin" | "btc" => Decimal::new(9500000, 2), // $95000.00
            "solana" | "sol" => Decimal::new(20000, 2),    // $200.00
            "bnb" => Decimal::new(70000, 2),               // $700.00
            "usdt" | "usdt_bnb" => Decimal::ONE,           // $1.00
            _ => Decimal::ZERO,
        }
    }

    fn get_network_info(currency: &str, network: &str, _environment: &str) -> NetworkInfo {
        // Handle USDT with different networks
        if currency == "usdt" || currency == "usdt_bnb" {
            match network {
                "usdt_bep20" => {
                    return NetworkInfo {
                        network: "BNB Smart Chain (BEP-20)".to_string(),
                        symbol: "USDT".to_string(),
                        decimals: 18,
                    };
                }
                "usdt_erc20" => {
                    return NetworkInfo {
                        network: "Ethereum (ERC-20)".to_string(),
                        symbol: "USDT".to_string(),
                        decimals: 6,
                    };
                }
                _ => {}
            }
        }

        match currency {
            "bitcoin" => NetworkInfo {
                network: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                decimals: 8,
            },
            "ethereum" => NetworkInfo {
                network: "Ethereum".to_string(),
                symbol: "ETH".to_string(),
                decimals: 18,
            },
            "solana" => NetworkInfo {
                network: "Solana".to_string(),
                symbol: "SOL".to_string(),
                decimals: 9,
            },
            "bnb" => NetworkInfo {
                network: "BNB Chain".to_string(),
                symbol: "BNB".to_string(),
                decimals: 18,
            },
            "usdt" => NetworkInfo {
                network: "Ethereum (USDT)".to_string(),
                symbol: "USDT".to_string(),
                decimals: 6,
            },
            "usdt_bnb" => NetworkInfo {
                network: "BNB Chain (USDT)".to_string(),
                symbol: "USDT".to_string(),
                decimals: 18,
            },
            _ => NetworkInfo {
                network: currency.to_string(),
                symbol: currency.to_uppercase(),
                decimals: 18,
            },
        }
    }

    fn network_to_currency(network: &str) -> String {
        match network {
            "eth" => "ethereum".to_string(),
            "bnb" => "bnb".to_string(),
            "sol" => "solana".to_string(),
            "btc" => "bitcoin".to_string(),
            "usdt_erc20" => "usdt_erc20".to_string(),
            "usdt_bep20" => "usdt_bep20".to_string(),
            "base_eth" => "ethereum".to_string(),
            _ => network.to_string(),
        }
    }

    fn map_network_for_display(network: &str) -> String {
        match network.to_lowercase().as_str() {
            "sol" => "SOL".to_string(),
            "eth" => "ETH".to_string(),
            "btc" => "BTC".to_string(),
            "bnb" => "BNB".to_string(),
            "usdt_erc20" => "USDT (ERC20)".to_string(),
            "usdt_bep20" => "USDT (BEP20)".to_string(),
            "base_eth" => "Base ETH".to_string(),
            "multi" => "SOL".to_string(),
            _ => network.to_uppercase(),
        }
    }

    fn map_currency_for_display(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "solana" => "SOL".to_string(),
            "ethereum" => "ETH".to_string(),
            "bitcoin" => "BTC".to_string(),
            "bnb" => "BNB".to_string(),
            "usdt" => "USDT".to_string(),
            "usdt_bnb" => "USDT".to_string(),
            "multi" => "SOL".to_string(),
            _ => currency.to_uppercase(),
        }
    }

    async fn calculate_withdrawal_summary(
        app_state: &AppState,
        merchant_id: &str,
    ) -> Result<WithdrawalSummary, AppError> {
        log::info!(
            "Calculating withdrawal summary for merchant {}",
            merchant_id
        );
        let withdrawals = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .all(&app_state.db)
            .await
            .map_err(|e| {
                log::error!("Database error fetching withdrawals for summary: {:?}", e);
                AppError::DatabaseError(format!("Failed to fetch withdrawals for summary: {}", e))
            })?;

        let total_count = withdrawals.len() as u64;
        let pending_count = withdrawals.iter().filter(|w| w.status == "pending").count() as u64;
        let processing_count = withdrawals
            .iter()
            .filter(|w| w.status == "processing")
            .count() as u64;
        let confirmed_count = withdrawals
            .iter()
            .filter(|w| w.status == "confirmed")
            .count() as u64;
        let failed_count = withdrawals.iter().filter(|w| w.status == "failed").count() as u64;

        let total_amount_usd = Some("0.00".to_string());
        let total_fees_usd = Some("0.00".to_string());

        Ok(WithdrawalSummary {
            total_count,
            pending_count,
            processing_count,
            confirmed_count,
            failed_count,
            total_amount_usd,
            total_fees_usd,
        })
    }
}

struct NetworkInfo {
    network: String,
    symbol: String,
    decimals: u8,
}
