use chrono::Utc;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    Set,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    merchant::models::withdrawal::{
        AggregatedBalance, MultiWalletBalance, MultiWalletBalanceResponse,
        MultiWalletWithdrawalRequest, MultiWalletWithdrawalResponse, SelectedWallet, WalletBalance,
        WalletSelection, WithdrawalPlan, WithdrawalPreview, WithdrawalTransaction,
    },
    shared::{
        entities::{
            merchant, payment_request, prelude::*, wallet, wallet_balance, wallet_transaction,
            withdrawal_request,
        },
        models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
        service::{
            price_oracle::PRICE_ORACLE, wallet::WalletService,
            withdrawal_processor::WithdrawalProcessor,
        },
        utils::errors::AppError,
        AppState,
    },
};

pub struct MultiWalletService;

impl MultiWalletService {
    pub async fn get_merchant_wallets(
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: &str,
    ) -> Result<Vec<WalletBalance>, AppError> {
        log::info!(
            " Fetching all wallets for merchant {} in {} environment",
            merchant_id,
            environment
        );

        let payments = PaymentRequest::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Environment.eq(environment.to_lowercase()))
            .filter(payment_request::Column::Status.eq("paid"))
            .all(db)
            .await?;

        let mut wallet_balances = Vec::new();
        let mut processed_addresses = std::collections::HashSet::new();

        for payment in payments {
            if processed_addresses.contains(&payment.wallet_address) {
                continue;
            }
            processed_addresses.insert(payment.wallet_address.clone());

            let wallet_record = Wallet::find()
                .filter(wallet::Column::Address.eq(&payment.wallet_address))
                .one(db)
                .await?;

            if let Some(wallet) = wallet_record {
                let balance = wallet_balance::Entity::find()
                    .filter(wallet_balance::Column::WalletId.eq(&wallet.id))
                    .one(db)
                    .await?;

                if let Some(balance_record) = balance {
                    let usd_value = Self::calculate_usd_value(
                        &balance_record.available_balance,
                        &wallet.currency,
                    )
                    .await;

                    wallet_balances.push(WalletBalance {
                        wallet_id: wallet.id.clone(),
                        wallet_address: wallet.address.clone(),
                        currency: wallet.currency.clone(),
                        network: Self::extract_network(&wallet.currency),
                        available_balance: balance_record.available_balance.to_string(),
                        usd_value,
                        payment_source_id: Some(payment.id.clone()),
                        created_at: wallet.created_at.into(),
                        last_transaction_at: Some(balance_record.last_updated.into()),
                    });
                } else {
                    wallet_balances.push(WalletBalance {
                        wallet_id: wallet.id.clone(),
                        wallet_address: wallet.address.clone(),
                        currency: wallet.currency.clone(),
                        network: Self::extract_network(&wallet.currency),
                        available_balance: "0".to_string(),
                        usd_value: None,
                        payment_source_id: Some(payment.id.clone()),
                        created_at: wallet.created_at.into(),
                        last_transaction_at: None,
                    });
                }
            }
        }

        log::info!(
            " Found {} wallets with balances for merchant {}",
            wallet_balances.len(),
            merchant_id
        );
        Ok(wallet_balances)
    }

    pub async fn get_aggregated_balances(
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: &str,
    ) -> Result<Vec<AggregatedBalance>, AppError> {
        let wallets = Self::get_merchant_wallets(db, merchant_id, environment).await?;

        let mut currency_groups: HashMap<String, Vec<WalletBalance>> = HashMap::new();

        for wallet in wallets {
            let currency_key = format!("{}_{}", wallet.currency, wallet.network);
            currency_groups
                .entry(currency_key)
                .or_default()
                .push(wallet);
        }

        let mut aggregated = Vec::new();

        for (currency_key, wallets_in_currency) in currency_groups {
            let first_wallet = wallets_in_currency.first().unwrap();
            let currency = first_wallet.currency.clone();
            let network = first_wallet.network.clone();

            let mut total_crypto = Decimal::ZERO;
            let mut total_usd = Decimal::ZERO;

            for wallet in &wallets_in_currency {
                if let Ok(crypto_amount) = wallet.available_balance.parse::<Decimal>() {
                    total_crypto += crypto_amount;
                }

                if let Some(usd_str) = &wallet.usd_value {
                    if let Ok(usd_amount) = usd_str.parse::<Decimal>() {
                        total_usd += usd_amount;
                    }
                }
            }

            aggregated.push(AggregatedBalance {
                currency: currency.clone(),
                network: network.clone(),
                total_crypto_amount: total_crypto.to_string(),
                total_usd_value: total_usd.to_string(),
                wallet_count: wallets_in_currency.len() as u64,
                wallets: wallets_in_currency,
            });
        }

        Ok(aggregated)
    }

    pub async fn select_wallets_for_withdrawal(
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: &str,
        currency: &str,
        target_amount: Decimal,
        strategy: &str,
    ) -> Result<WithdrawalPlan, AppError> {
        log::info!(
            "🎯 Selecting wallets for withdrawal: {} {} using {} strategy",
            target_amount,
            currency,
            strategy
        );

        let wallets = Self::get_merchant_wallets(db, merchant_id, environment).await?;

        let mut matching_wallets: Vec<WalletBalance> = wallets
            .into_iter()
            .filter(|w| Self::currency_matches(currency, &w.currency))
            .collect();

        if matching_wallets.is_empty() {
            return Ok(WithdrawalPlan {
                withdrawal_id: uuid::Uuid::new_v4().to_string(),
                requested_amount: target_amount.to_string(),
                amount_type: "crypto".to_string(),
                total_available: "0".to_string(),
                selected_wallets: vec![],
                total_fees_estimated: "0".to_string(),
                net_transfer_amount: "0".to_string(),
                can_fulfill: false,
                shortfall_amount: Some(target_amount.to_string()),
            });
        }

        match strategy {
            "largest_first" => {
                matching_wallets.sort_by(|a, b| {
                    let a_balance: Decimal = a.available_balance.parse().unwrap_or_default();
                    let b_balance: Decimal = b.available_balance.parse().unwrap_or_default();
                    b_balance.cmp(&a_balance)
                });
            }
            "fifo" => {
                matching_wallets.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            }
            "optimal" | _ => {
                matching_wallets.sort_by(|a, b| {
                    let a_balance: Decimal = a.available_balance.parse().unwrap_or_default();
                    let b_balance: Decimal = b.available_balance.parse().unwrap_or_default();
                    b_balance.cmp(&a_balance)
                });
            }
        }

        let mut selected_wallets = Vec::new();
        let mut cumulative_amount = Decimal::ZERO;
        let mut total_fees = Decimal::ZERO;
        let mut total_available = Decimal::ZERO;

        for wallet in &matching_wallets {
            if let Ok(balance) = wallet.available_balance.parse::<Decimal>() {
                total_available += balance;
            }
        }

        for (index, wallet) in matching_wallets.iter().enumerate() {
            let wallet_balance: Decimal = wallet.available_balance.parse().unwrap_or_default();

            if wallet_balance <= Decimal::ZERO {
                continue;
            }

            let estimated_fee = Self::estimate_transaction_fee(currency, &wallet.network).await?;

            let remaining_needed = target_amount - cumulative_amount;
            let max_transferable = wallet_balance - estimated_fee;

            if max_transferable <= Decimal::ZERO {
                continue;
            }

            let amount_to_transfer = remaining_needed.min(max_transferable);

            selected_wallets.push(SelectedWallet {
                wallet_id: wallet.wallet_id.clone(),
                wallet_address: wallet.wallet_address.clone(),
                available_balance: wallet_balance.to_string(),
                amount_to_transfer: amount_to_transfer.to_string(),
                estimated_fee: estimated_fee.to_string(),
                net_amount: amount_to_transfer.to_string(),
                order: (index + 1) as u32,
            });

            cumulative_amount += amount_to_transfer;
            total_fees += estimated_fee;

            if cumulative_amount >= target_amount {
                break;
            }
        }

        let can_fulfill = cumulative_amount >= target_amount;
        let shortfall = if can_fulfill {
            None
        } else {
            Some((target_amount - cumulative_amount).to_string())
        };

        Ok(WithdrawalPlan {
            withdrawal_id: uuid::Uuid::new_v4().to_string(),
            requested_amount: target_amount.to_string(),
            amount_type: "crypto".to_string(),
            total_available: total_available.to_string(),
            selected_wallets,
            total_fees_estimated: total_fees.to_string(),
            net_transfer_amount: cumulative_amount.to_string(),
            can_fulfill,
            shortfall_amount: shortfall,
        })
    }

    async fn calculate_usd_value(amount: &Decimal, currency: &str) -> Option<String> {
        match Self::get_current_usd_price(currency).await {
            Ok(price) => {
                let usd_value = amount * price;
                Some(usd_value.to_string())
            }
            Err(e) => {
                log::warn!("Failed to get USD price for {}: {}", currency, e);
                None
            }
        }
    }

    async fn estimate_transaction_fee(currency: &str, network: &str) -> Result<Decimal, AppError> {
        let fee = match currency.to_lowercase().as_str() {
            "sol" | "solana" => Decimal::try_from(0.000005).unwrap(), // 5000 lamports
            "eth" | "ethereum" => Decimal::try_from(0.002).unwrap(),  // ~$5 at current gas prices
            "bnb" => Decimal::try_from(0.0005).unwrap(),              // BNB Smart Chain
            "btc" | "bitcoin" => Decimal::try_from(0.00001).unwrap(), // 1000 sats
            "usdt" => {
                match network.to_lowercase().as_str() {
                    "ethereum" => Decimal::try_from(0.003).unwrap(), // Higher for ERC-20
                    "bsc" | "bnb" => Decimal::try_from(0.001).unwrap(), // BEP-20
                    "solana" => Decimal::try_from(0.000005).unwrap(), // SPL token
                    _ => Decimal::try_from(0.001).unwrap(),
                }
            }
            _ => Decimal::try_from(0.001).unwrap(), // Default
        };

        log::debug!("Estimated fee for {} on {}: {}", currency, network, fee);
        Ok(fee)
    }

    fn extract_network(currency: &str) -> String {
        if currency.contains("_mainnet") {
            if currency.contains("sol") {
                "solana".to_string()
            } else if currency.contains("eth") {
                "ethereum".to_string()
            } else if currency.contains("bnb") {
                "binance".to_string()
            } else {
                "unknown".to_string()
            }
        } else if currency.contains("_testnet") {
            "testnet".to_string()
        } else {
            currency.split('_').next().unwrap_or("unknown").to_string()
        }
    }

    pub fn currency_matches(requested: &str, wallet_currency: &str) -> bool {
        let requested_lower = requested.to_lowercase();
        let wallet_lower = wallet_currency.to_lowercase();

        if wallet_lower.contains(&requested_lower) {
            return true;
        }

        match requested_lower.as_str() {
            "sol" | "solana" => wallet_lower.contains("sol"),
            "eth" | "ethereum" => wallet_lower.contains("eth") && !wallet_lower.contains("usdt"),
            "bnb" | "binance" => wallet_lower.contains("bnb"),
            "btc" | "bitcoin" => wallet_lower.contains("btc"),
            "usdt" => wallet_lower.contains("usdt"),
            "usdc" => wallet_lower.contains("usdc"),
            _ => false,
        }
    }

    pub async fn get_multi_wallet_balances(
        app_state: &AppState,
        merchant_id: &str,
        environment: &str,
    ) -> Result<MultiWalletBalanceResponse, AppError> {
        log::info!(
            "Getting multi-wallet balances for merchant {} environment {}",
            merchant_id,
            environment
        );

        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        let wallets =
            Self::get_merchant_wallets_with_usd(&app_state, &merchant.user_id, environment).await?;

        log::info!(
            "Found {} wallets for merchant {} in {} environment",
            wallets.len(),
            merchant_id,
            environment
        );

        let mut wallet_balances = Vec::new();
        let mut total_usd_value = Decimal::ZERO;
        let mut currency_totals = HashMap::new();

        for wallet_data in wallets {
            let base_currency = Self::extract_base_currency(&wallet_data.currency);
            let network_info = Self::get_network_info(&base_currency, environment);

            let wallet_balance = MultiWalletBalance {
                wallet_id: wallet_data.wallet_id.clone(),
                address: wallet_data.wallet_address.clone(),
                currency: base_currency.clone(),
                network: network_info.network.clone(),
                symbol: network_info.symbol.clone(),
                available: wallet_data.available_balance.clone(),
                pending: "0".to_string(),
                total: wallet_data.available_balance.clone(),
                usd_value: wallet_data.usd_value.clone().unwrap_or("0".to_string()),
                decimals: network_info.decimals,
                last_updated: wallet_data.last_transaction_at.unwrap_or(Utc::now()),
            };

            wallet_balances.push(wallet_balance);

            if let Ok(usd_val) = wallet_data
                .usd_value
                .clone()
                .unwrap_or("0".to_string())
                .parse::<Decimal>()
            {
                total_usd_value += usd_val;
            }

            let balance = wallet_data
                .available_balance
                .parse::<Decimal>()
                .unwrap_or_default();
            *currency_totals
                .entry(network_info.symbol)
                .or_insert(Decimal::ZERO) += balance;
        }

        Ok(MultiWalletBalanceResponse {
            environment: environment.to_string(),
            total_wallets: wallet_balances.len() as u64,
            total_usd_value: total_usd_value.to_string(),
            wallet_balances,
            currency_totals: currency_totals
                .into_iter()
                .map(|(currency, amount)| (currency, amount.to_string()))
                .collect(),
            last_updated: Utc::now(),
        })
    }

    pub async fn preview_withdrawal(
        app_state: &AppState,
        merchant_id: &str,
        request: &MultiWalletWithdrawalRequest,
    ) -> Result<WithdrawalPreview, AppError> {
        log::info!(
            "Previewing multi-wallet withdrawal for merchant {} amount {} {}",
            merchant_id,
            request.amount,
            request.amount_type
        );

        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        let wallets = Self::get_merchant_wallets_with_usd(
            &app_state,
            &merchant.user_id,
            &request.environment,
        )
        .await?;

        if wallets.is_empty() {
            return Err(AppError::ValidationError(format!(
                "No wallets found for {} environment",
                request.environment
            )));
        }

        let required_usd = match request.amount_type.as_str() {
            "usd" => request
                .amount
                .parse::<Decimal>()
                .map_err(|_| AppError::ValidationError("Invalid USD amount".to_string()))?,
            "crypto" => {
                let amount = request
                    .amount
                    .parse::<Decimal>()
                    .map_err(|_| AppError::ValidationError("Invalid crypto amount".to_string()))?;
                let currency = request.currency.as_ref().ok_or_else(|| {
                    AppError::ValidationError("Currency required for crypto amounts".to_string())
                })?;
                let usd_price = Self::get_current_usd_price(currency).await?;
                amount * usd_price
            }
            _ => {
                return Err(AppError::ValidationError(
                    "Amount type must be 'usd' or 'crypto'".to_string(),
                ))
            }
        };

        log::info!("Required amount: {} USD", required_usd);

        let total_available_usd: Decimal = wallets
            .iter()
            .map(|w| {
                w.usd_value
                    .clone()
                    .unwrap_or("0".to_string())
                    .parse::<Decimal>()
                    .unwrap_or_default()
            })
            .sum();

        log::info!(
            "Total available: {} USD across {} wallets",
            total_available_usd,
            wallets.len()
        );

        if required_usd > total_available_usd {
            return Err(AppError::ValidationError(format!(
                "Insufficient total balance: need ${} but only have ${} available",
                required_usd, total_available_usd
            )));
        }

        let strategy = request
            .wallet_selection_strategy
            .as_deref()
            .unwrap_or("optimal");
        let selected_wallets =
            Self::select_wallets_for_usd_withdrawal(&wallets, required_usd, strategy)?;

        let mut wallet_selections = Vec::new();
        let mut total_selected_usd = Decimal::ZERO;
        let mut estimated_total_fees_usd = Decimal::ZERO;

        for wallet_data in &selected_wallets {
            let base_currency = Self::extract_base_currency(&wallet_data.currency);
            let network_info = Self::get_network_info(&base_currency, &request.environment);

            let remaining_needed = required_usd - total_selected_usd;
            let wallet_usd_value = wallet_data
                .usd_value
                .clone()
                .unwrap_or("0".to_string())
                .parse::<Decimal>()
                .unwrap_or_default();
            let wallet_contribution_usd = remaining_needed.min(wallet_usd_value);

            let wallet_balance_crypto = wallet_data
                .available_balance
                .parse::<Decimal>()
                .unwrap_or_default();
            let wallet_contribution_crypto = if wallet_usd_value > Decimal::ZERO {
                wallet_balance_crypto * (wallet_contribution_usd / wallet_usd_value)
            } else {
                Decimal::ZERO
            };

            let estimated_fee_crypto = Self::estimate_transaction_fee_enhanced(
                &base_currency,
                &request.environment,
                wallet_contribution_crypto,
            )
            .await;
            let usd_price = Self::get_current_usd_price(&base_currency)
                .await
                .unwrap_or(Decimal::ONE);
            let estimated_fee_usd = estimated_fee_crypto * usd_price;
            estimated_total_fees_usd += estimated_fee_usd;

            wallet_selections.push(WalletSelection {
                wallet_id: wallet_data.wallet_id.clone(),
                address: wallet_data.wallet_address.clone(),
                currency: base_currency.clone(),
                symbol: network_info.symbol.clone(),
                amount_crypto: wallet_contribution_crypto.to_string(),
                amount_usd: wallet_contribution_usd.to_string(),
                estimated_fee: estimated_fee_crypto.to_string(),
                network: network_info.network.clone(),
            });

            total_selected_usd += wallet_contribution_usd;

            if total_selected_usd >= required_usd {
                break;
            }
        }

        Ok(WithdrawalPreview {
            requested_amount: request.amount.clone(),
            requested_amount_type: request.amount_type.clone(),
            requested_usd_equivalent: required_usd.to_string(),
            total_available_usd: total_available_usd.to_string(),
            selected_wallets: wallet_selections.len() as u64,
            total_selected_usd: total_selected_usd.to_string(),
            estimated_total_fees_usd: estimated_total_fees_usd.to_string(),
            net_amount_usd: (total_selected_usd - estimated_total_fees_usd).to_string(),
            wallet_selections,
            can_fulfill: total_selected_usd >= required_usd,
            environment: request.environment.clone(),
        })
    }

    pub async fn execute_multi_wallet_withdrawal(
        app_state: &AppState,
        merchant_id: &str,
        request: MultiWalletWithdrawalRequest,
    ) -> Result<MultiWalletWithdrawalResponse, AppError> {
        log::info!(
            "Executing multi-wallet withdrawal for merchant {} amount {} {}",
            merchant_id,
            request.amount,
            request.amount_type
        );

        if let Some(existing) =
            Self::check_idempotency(&app_state, merchant_id, &request.idempotency_key).await?
        {
            log::info!(
                "Returning existing withdrawal for idempotency key: {}",
                request.idempotency_key
            );
            return Ok(existing);
        }

        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(&app_state.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        let preview = Self::preview_withdrawal(app_state, merchant_id, &request).await?;

        if !preview.can_fulfill {
            return Err(AppError::ValidationError(
                "Cannot fulfill withdrawal with available balances".to_string(),
            ));
        }

        let primary_wallet_id = preview
            .wallet_selections
            .first()
            .map(|ws| ws.wallet_id.clone())
            .ok_or_else(|| {
                AppError::ValidationError("No wallets selected for withdrawal".to_string())
            })?;

        let master_withdrawal_id = Uuid::new_v4().to_string();
        let master_withdrawal = withdrawal_request::ActiveModel {
            id: Set(master_withdrawal_id.clone()),
            merchant_id: Set(merchant_id.to_string()),
            wallet_id: Set(primary_wallet_id.clone()),
            external_id: Set(request.external_id.unwrap_or_else(|| master_withdrawal_id.clone())),
            idempotency_key: Set(request.idempotency_key.clone()),
            environment: Set(request.environment.clone()),
            network: Set("multi".to_string()),
            currency: Set("multi".to_string()),
            to_address: Set(request.to_address.clone()),
            amount: Set(request.amount.parse::<Decimal>().unwrap_or_default()),
            fee: Set(Some(preview.estimated_total_fees_usd.parse::<Decimal>().unwrap_or_default())),
            net_amount: Set(preview.net_amount_usd.parse::<Decimal>().unwrap_or_default()),
            status: Set("processing".to_string()),
            tx_hash: Set(None),
            blockchain_confirmations: Set(None),
            required_confirmations: Set(1),
            metadata: Set(Some(serde_json::json!({
                "type": "multi_wallet_withdrawal",
                "wallets_count": preview.wallet_selections.len(),
                "amount_type": request.amount_type,
                "wallets_used": preview.wallet_selections.iter().map(|ws| ws.wallet_id.clone()).collect::<Vec<_>>(),
                "primary_wallet_id": primary_wallet_id.clone(),
                "note": "Using first wallet ID for foreign key constraint compliance"
            }).to_string())),
            error_message: Set(None),
            created_at: Set(Utc::now().into()),
            processed_at: Set(None),
            confirmed_at: Set(None),
            updated_at: Set(Utc::now().into()),
            broadcast_at: Set(None),
            failed_at: Set(None),
            failure_reason: Set(None),
            finality_required: Set(1),
            finality_reached_at: Set(None),
            replaced_by_tx: Set(None),
            reorg_depth: Set(0),
            lifecycle_state: Set(WithdrawalLifecycleState::Quoted.to_string()),
            withdrawal_funds_state: Set(WithdrawalFundsState::Unlocked.to_string()),
        };

        let saved_master = master_withdrawal.insert(&app_state.db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to create master withdrawal: {}", e))
        })?;

        let mut transactions = Vec::new();
        let mut successful_count = 0;
        let mut failed_count = 0;
        let mut total_transferred_usd = Decimal::ZERO;

        for wallet_selection in &preview.wallet_selections {
            log::info!(
                "Processing withdrawal from wallet {} ({} {})",
                wallet_selection.address,
                wallet_selection.amount_crypto,
                wallet_selection.symbol
            );

            match Self::execute_single_wallet_withdrawal(
                app_state,
                &wallet_selection,
                &request.to_address,
                &master_withdrawal_id,
            )
            .await
            {
                Ok(tx) => {
                    log::info!(
                        "✅ Successful withdrawal from wallet {}: tx_hash = {:?}",
                        wallet_selection.address,
                        tx.tx_hash
                    );
                    let tx_usd = tx.usd_equivalent.parse::<Decimal>().unwrap_or_default();
                    total_transferred_usd += tx_usd;
                    successful_count += 1;
                    transactions.push(tx);
                }
                Err(e) => {
                    log::error!(
                        " Failed withdrawal from wallet {}: {}",
                        wallet_selection.address,
                        e
                    );
                    failed_count += 1;

                    let failed_tx = WithdrawalTransaction {
                        wallet_id: wallet_selection.wallet_id.clone(),
                        wallet_address: wallet_selection.address.clone(),
                        currency: wallet_selection.currency.clone(),
                        amount: wallet_selection.amount_crypto.clone(),
                        fee: wallet_selection.estimated_fee.clone(),
                        net_amount: "0".to_string(),
                        usd_equivalent: "0".to_string(),
                        tx_hash: None,
                        status: "failed".to_string(),
                    };
                    transactions.push(failed_tx);
                }
            }
        }

        let final_status = if successful_count > 0 {
            if failed_count == 0 {
                "confirmed"
            } else {
                "partially_completed"
            }
        } else {
            "failed"
        };

        let mut master_update: withdrawal_request::ActiveModel = saved_master.into();
        master_update.status = Set(final_status.to_string());
        master_update.processed_at = Set(Some(Utc::now().into()));
        master_update.updated_at = Set(Utc::now().into());

        if final_status == "confirmed" || final_status == "partially_completed" {
            master_update.confirmed_at = Set(Some(Utc::now().into()));
            master_update.broadcast_at = Set(Some(Utc::now().into()));
        } else if final_status == "failed" {
            master_update.failed_at = Set(Some(Utc::now().into()));
            master_update.failure_reason = Set(Some("All wallet withdrawals failed".to_string()));
        }

        let final_master = master_update.update(&app_state.db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to update master withdrawal: {}", e))
        })?;

        let tx_hashes: Vec<String> = transactions
            .iter()
            .filter_map(|tx| tx.tx_hash.clone())
            .collect();

        Ok(MultiWalletWithdrawalResponse {
            id: master_withdrawal_id,
            merchant_id: merchant_id.to_string(),
            external_id: Some(final_master.external_id),
            idempotency_key: request.idempotency_key,
            environment: request.environment,
            to_address: request.to_address,
            requested_amount: request.amount,
            requested_amount_type: request.amount_type,
            transferred_amount_usd: total_transferred_usd.to_string(),
            total_fees_usd: preview.estimated_total_fees_usd.clone(),
            net_amount_usd: (total_transferred_usd
                - preview
                    .estimated_total_fees_usd
                    .parse::<Decimal>()
                    .unwrap_or_default())
            .to_string(),
            status: final_status.to_string(),
            wallets_used: successful_count,
            wallets_failed: failed_count,
            tx_hashes,
            transactions,
            created_at: final_master.created_at.into(),
            processed_at: final_master.processed_at.map(|dt| dt.into()),
            confirmed_at: final_master.confirmed_at.map(|dt| dt.into()),
            updated_at: final_master.updated_at.into(),
        })
    }

    async fn get_merchant_wallets_with_usd(
        app_state: &AppState,
        user_id: &str,
        environment: &str,
    ) -> Result<Vec<WalletBalance>, AppError> {
        let wallets = Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .filter(wallet::Column::Currency.contains(&format!("_{}", environment)))
            .all(&app_state.db)
            .await?;

        let mut wallet_data = Vec::new();

        for wallet in wallets {
            if let Some(balance) = wallet_balance::Entity::find()
                .filter(wallet_balance::Column::WalletId.eq(&wallet.id))
                .one(&app_state.db)
                .await?
            {
                if balance.available_balance > Decimal::ZERO {
                    let base_currency = Self::extract_base_currency(&wallet.currency);
                    let usd_value =
                        Self::calculate_usd_value(&balance.available_balance, &base_currency).await;

                    wallet_data.push(WalletBalance {
                        wallet_id: wallet.id,
                        wallet_address: wallet.address,
                        currency: wallet.currency.clone(),
                        network: Self::extract_network(&wallet.currency),
                        available_balance: balance.available_balance.to_string(),
                        usd_value,
                        payment_source_id: None,
                        created_at: wallet.created_at.into(),
                        last_transaction_at: Some(balance.last_updated.into()),
                    });
                }
            }
        }

        wallet_data.sort_by(|a, b| {
            let a_usd = a
                .usd_value
                .as_ref()
                .unwrap_or(&"0".to_string())
                .parse::<Decimal>()
                .unwrap_or_default();
            let b_usd = b
                .usd_value
                .as_ref()
                .unwrap_or(&"0".to_string())
                .parse::<Decimal>()
                .unwrap_or_default();
            b_usd.cmp(&a_usd)
        });

        Ok(wallet_data)
    }

    fn select_wallets_for_usd_withdrawal(
        wallets: &[WalletBalance],
        required_usd: Decimal,
        strategy: &str,
    ) -> Result<Vec<WalletBalance>, AppError> {
        let mut selected = Vec::new();
        let mut total_selected = Decimal::ZERO;

        let mut sorted_wallets = wallets.to_vec();

        match strategy {
            "largest_first" => {
                sorted_wallets.sort_by(|a, b| {
                    let a_usd = a
                        .usd_value
                        .as_ref()
                        .unwrap_or(&"0".to_string())
                        .parse::<Decimal>()
                        .unwrap_or_default();
                    let b_usd = b
                        .usd_value
                        .as_ref()
                        .unwrap_or(&"0".to_string())
                        .parse::<Decimal>()
                        .unwrap_or_default();
                    b_usd.cmp(&a_usd)
                });
            }
            "fifo" => {
                sorted_wallets.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            }
            "optimal" | _ => {}
        }

        for wallet_data in sorted_wallets {
            if total_selected >= required_usd {
                break;
            }

            let wallet_usd_value = wallet_data
                .usd_value
                .as_ref()
                .unwrap_or(&"0".to_string())
                .parse::<Decimal>()
                .unwrap_or_default();

            if wallet_usd_value > Decimal::ZERO {
                selected.push(wallet_data);
                total_selected += wallet_usd_value;
            }
        }

        if total_selected < required_usd {
            return Err(AppError::ValidationError(format!(
                "Insufficient funds: need ${} but only have ${} available",
                required_usd, total_selected
            )));
        }

        Ok(selected)
    }

    async fn execute_single_wallet_withdrawal(
        app_state: &AppState,
        wallet_selection: &WalletSelection,
        to_address: &str,
        master_withdrawal_id: &str,
    ) -> Result<WithdrawalTransaction, AppError> {
        log::info!(
            "Executing single wallet withdrawal from {} to {}",
            wallet_selection.address,
            to_address
        );

        let wallet = Wallet::find()
            .filter(wallet::Column::Id.eq(&wallet_selection.wallet_id))
            .one(&app_state.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Wallet not found".to_string()))?;

        let withdrawal_id = Uuid::new_v4().to_string();
        let amount = wallet_selection
            .amount_crypto
            .parse::<Decimal>()
            .map_err(|_| AppError::ValidationError("Invalid amount".to_string()))?;
        let fee = wallet_selection
            .estimated_fee
            .parse::<Decimal>()
            .map_err(|_| AppError::ValidationError("Invalid fee".to_string()))?;

        let withdrawal = withdrawal_request::ActiveModel {
            id: Set(withdrawal_id.clone()),
            merchant_id: Set("auto".to_string()),
            wallet_id: Set(wallet.id.clone()),
            external_id: Set(format!(
                "{}_{}",
                master_withdrawal_id, wallet_selection.wallet_id
            )),
            idempotency_key: Set(format!(
                "{}_{}",
                master_withdrawal_id, wallet_selection.wallet_id
            )),
            environment: Set(wallet_selection.network.clone()),
            network: Set(wallet_selection.currency.clone()),
            currency: Set(wallet_selection.currency.clone()),
            to_address: Set(to_address.to_string()),
            amount: Set(amount),
            fee: Set(Some(fee)),
            net_amount: Set(amount - fee),
            status: Set("pending".to_string()),
            tx_hash: Set(None),
            blockchain_confirmations: Set(None),
            required_confirmations: Set(1),
            metadata: Set(Some(
                serde_json::json!({
                    "master_withdrawal_id": master_withdrawal_id,
                    "wallet_address": wallet_selection.address
                })
                .to_string(),
            )),
            error_message: Set(None),
            created_at: Set(Utc::now().into()),
            processed_at: Set(None),
            confirmed_at: Set(None),
            updated_at: Set(Utc::now().into()),
            broadcast_at: Set(None),
            failed_at: Set(None),
            failure_reason: Set(None),
            finality_required: Set(1),
            finality_reached_at: Set(None),
            replaced_by_tx: Set(None),
            reorg_depth: Set(0),
            lifecycle_state: Set(WithdrawalLifecycleState::Quoted.to_string()),
            withdrawal_funds_state: Set(WithdrawalFundsState::Unlocked.to_string()),
        };

        let saved_withdrawal = withdrawal
            .insert(&app_state.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to create withdrawal: {}", e)))?;

        match WithdrawalProcessor::process_withdrawal(app_state, &withdrawal_id).await {
            Ok(tx_hash) => {
                log::info!("✅ Withdrawal processed successfully: {}", tx_hash);

                Ok(WithdrawalTransaction {
                    wallet_id: wallet_selection.wallet_id.clone(),
                    wallet_address: wallet_selection.address.clone(),
                    currency: wallet_selection.currency.clone(),
                    amount: wallet_selection.amount_crypto.clone(),
                    fee: wallet_selection.estimated_fee.clone(),
                    net_amount: (amount - fee).to_string(),
                    usd_equivalent: wallet_selection.amount_usd.clone(),
                    tx_hash: Some(tx_hash),
                    status: "confirmed".to_string(),
                })
            }
            Err(e) => {
                log::error!(" Withdrawal processing failed: {}", e);

                let mut withdrawal_update: withdrawal_request::ActiveModel =
                    saved_withdrawal.into();
                withdrawal_update.status = Set("failed".to_string());
                withdrawal_update.failure_reason = Set(Some(e.to_string()));
                withdrawal_update.failed_at = Set(Some(Utc::now().into()));
                withdrawal_update.updated_at = Set(Utc::now().into());

                let _ = withdrawal_update.update(&app_state.db).await;

                Err(e)
            }
        }
    }

    async fn check_idempotency(
        app_state: &AppState,
        merchant_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<MultiWalletWithdrawalResponse>, AppError> {
        if let Some(existing) = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .filter(withdrawal_request::Column::IdempotencyKey.eq(idempotency_key))
            .one(&app_state.db)
            .await?
        {
            return Ok(Some(MultiWalletWithdrawalResponse {
                id: existing.id,
                merchant_id: existing.merchant_id,
                external_id: Some(existing.external_id),
                idempotency_key: existing.idempotency_key,
                environment: existing.environment,
                to_address: existing.to_address,
                requested_amount: existing.amount.to_string(),
                requested_amount_type: "usd".to_string(),
                transferred_amount_usd: existing.net_amount.to_string(),
                total_fees_usd: existing.fee.unwrap_or_default().to_string(),
                net_amount_usd: existing.net_amount.to_string(),
                status: existing.status,
                wallets_used: 1,
                wallets_failed: 0,
                tx_hashes: existing.tx_hash.into_iter().collect(),
                transactions: vec![],
                created_at: existing.created_at.into(),
                processed_at: existing.processed_at.map(|dt| dt.into()),
                confirmed_at: existing.confirmed_at.map(|dt| dt.into()),
                updated_at: existing.updated_at.into(),
            }));
        }

        Ok(None)
    }

    async fn estimate_transaction_fee_enhanced(
        currency: &str,
        environment: &str,
        amount: Decimal,
    ) -> Decimal {
        match currency.to_lowercase().as_str() {
            "ethereum" | "eth" => {
                let base_fee = if environment == "mainnet" {
                    "20000000000"
                } else {
                    "10000000000"
                }; // Gwei
                let gas = "21000";
                let fee_wei = Decimal::from_str(base_fee).unwrap_or_default()
                    * Decimal::from_str(gas).unwrap_or_default();
                fee_wei / Decimal::from_str("1000000000000000000").unwrap_or(Decimal::ONE)
            }
            "bitcoin" | "btc" => {
                let sat_per_byte = if environment == "mainnet" { 20 } else { 10 };
                let tx_size = 250; // bytes
                let fee_sats = sat_per_byte * tx_size;
                Decimal::from(fee_sats) / Decimal::from(100_000_000)
            }
            "solana" | "sol" => {
                Decimal::from(5000) / Decimal::from(1_000_000_000) // 0.000005 SOL
            }
            "bnb" => {
                let base_fee = "5000000000"; // 5 Gwei
                let gas = "21000";
                let fee_wei = Decimal::from_str(base_fee).unwrap_or_default()
                    * Decimal::from_str(gas).unwrap_or_default();
                fee_wei / Decimal::from_str("1000000000000000000").unwrap_or(Decimal::ONE)
            }
            _ => Decimal::from_str("0.001").unwrap_or_default(),
        }
    }

    async fn get_current_usd_price(currency: &str) -> Result<Decimal, AppError> {
        PRICE_ORACLE.get_usd_price(currency).await
    }

    fn extract_base_currency(currency: &str) -> String {
        currency.split('_').next().unwrap_or(currency).to_string()
    }

    fn get_network_info(currency: &str, environment: &str) -> NetworkInfo {
        match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => NetworkInfo {
                network: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                decimals: 8,
            },
            "ethereum" | "eth" => NetworkInfo {
                network: "Ethereum".to_string(),
                symbol: "ETH".to_string(),
                decimals: 18,
            },
            "solana" | "sol" => NetworkInfo {
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
                network: match environment {
                    "mainnet" => "Ethereum (USDT)".to_string(),
                    _ => "Testnet (USDT)".to_string(),
                },
                symbol: "USDT".to_string(),
                decimals: 6,
            },
            _ => NetworkInfo {
                network: currency.to_string(),
                symbol: currency.to_uppercase(),
                decimals: 18,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct NetworkInfo {
    network: String,
    symbol: String,
    decimals: u8,
}
