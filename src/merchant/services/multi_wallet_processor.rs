use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

use crate::{
    merchant::{
        models::withdrawal::{SelectedWallet, WalletUsage, WithdrawalPlan, WithdrawalResponse},
        services::MultiWalletService,
    },
    shared::{
        entities::{prelude::*, wallet, wallet_balance, wallet_transaction, withdrawal_request},
        models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
        service::{
            solana_mainnet_withdrawal::SolanaMainnetWithdrawal,
            // eth_withdrawal::EthereumWithdrawal,  // TODO: Implement
            // btc_withdrawal::BitcoinWithdrawal,   // TODO: Implement
        },
        utils::errors::AppError,
        AppState,
    },
};

/// Multi-wallet transaction processor for executing withdrawals across multiple wallets
pub struct MultiWalletProcessor {
    app_state: std::sync::Arc<AppState>,
}

impl MultiWalletProcessor {
    pub fn new(app_state: std::sync::Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Execute a multi-wallet withdrawal request
    pub async fn execute_withdrawal(
        &self,
        merchant_id: &str,
        withdrawal_request: &crate::merchant::models::withdrawal::CreateWithdrawalRequest,
    ) -> Result<WithdrawalResponse, AppError> {
        let db = &self.app_state.db;

        log::info!(
            " Starting multi-wallet withdrawal execution for merchant {}",
            merchant_id
        );
        log::info!(
            "📋 Request details: {} {} to {}",
            withdrawal_request.amount,
            withdrawal_request.network,
            withdrawal_request.to_address
        );

        // Check for existing withdrawal with same idempotency key
        let existing = WithdrawalRequest::find()
            .filter(
                withdrawal_request::Column::IdempotencyKey.eq(&withdrawal_request.idempotency_key),
            )
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .one(db)
            .await?;

        if let Some(existing_withdrawal) = existing {
            log::warn!(
                " Duplicate withdrawal request detected: {}",
                existing_withdrawal.id
            );
            return Err(AppError::ValidationError(
                "Withdrawal with this idempotency key already exists".to_string(),
            ));
        }

        // Parse requested amount
        let amount_type = withdrawal_request
            .amount_type
            .as_deref()
            .unwrap_or("crypto");
        let (target_amount, is_max_withdrawal) = if withdrawal_request.amount == "max" {
            (Decimal::ZERO, true) // Will be calculated later
        } else {
            (
                withdrawal_request
                    .amount
                    .parse::<Decimal>()
                    .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?,
                false,
            )
        };

        // Create withdrawal plan
        let plan = if is_max_withdrawal {
            self.create_max_withdrawal_plan(
                db,
                merchant_id,
                &withdrawal_request.environment,
                &withdrawal_request.network,
            )
            .await?
        } else {
            let strategy = withdrawal_request
                .wallet_selection_strategy
                .as_deref()
                .unwrap_or("optimal");
            MultiWalletService::select_wallets_for_withdrawal(
                db,
                merchant_id,
                &withdrawal_request.environment,
                &withdrawal_request.network,
                target_amount,
                strategy,
            )
            .await?
        };

        if !plan.can_fulfill {
            return Err(AppError::ValidationError(format!(
                "Insufficient balance. Requested: {}, Available: {}, Shortfall: {}",
                plan.requested_amount,
                plan.total_available,
                plan.shortfall_amount.unwrap_or_default()
            )));
        }

        log::info!(
            " Withdrawal plan created: {} wallets selected, {} total transfer",
            plan.selected_wallets.len(),
            plan.net_transfer_amount
        );

        // Create initial withdrawal request record
        let withdrawal_id = Uuid::new_v4().to_string();
        let required_confirmations = Self::get_required_confirmations(&withdrawal_request.network);

        let withdrawal_record = withdrawal_request::ActiveModel {
            id: Set(withdrawal_id.clone()),
            merchant_id: Set(merchant_id.to_string()),
            external_id: Set(withdrawal_request.external_id.clone().unwrap_or_default()),
            idempotency_key: Set(withdrawal_request.idempotency_key.clone()),
            environment: Set(withdrawal_request.environment.clone()),
            network: Set(withdrawal_request.network.clone()),
            currency: Set(Self::get_currency_for_network(&withdrawal_request.network)),
            to_address: Set(withdrawal_request.to_address.clone()),
            amount: Set(plan.requested_amount.parse().unwrap_or_default()),
            wallet_id: Set("multi-wallet".to_string()), // Multi-wallet withdrawal placeholder
            status: Set("processing".to_string()),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
            finality_required: Set(required_confirmations),
            finality_reached_at: Set(None),
            replaced_by_tx: Set(None),
            reorg_depth: Set(0),
            lifecycle_state: Set(WithdrawalLifecycleState::Quoted.to_string()),
            withdrawal_funds_state: Set(WithdrawalFundsState::Unlocked.to_string()),
            ..Default::default()
        };

        let saved_withdrawal = withdrawal_record.insert(db).await?;
        log::info!("💾 Created withdrawal record: {}", saved_withdrawal.id);

        // Execute transactions for each selected wallet
        let mut wallet_usages = Vec::new();
        let mut tx_hashes = Vec::new();
        let mut explorer_urls = Vec::new();
        let mut total_transferred = Decimal::ZERO;
        let mut total_fees = Decimal::ZERO;

        for selected_wallet in &plan.selected_wallets {
            log::info!(
                "🏦 Processing wallet {} ({}/{})",
                selected_wallet.wallet_address,
                selected_wallet.order,
                plan.selected_wallets.len()
            );

            match self
                .execute_single_wallet_transaction(
                    db,
                    &withdrawal_id,
                    selected_wallet,
                    &withdrawal_request.to_address,
                    &withdrawal_request.network,
                    &withdrawal_request.environment,
                )
                .await
            {
                Ok(wallet_usage) => {
                    if let Some(ref tx_hash) = wallet_usage.tx_hash {
                        tx_hashes.push(tx_hash.clone());
                        explorer_urls.push(Self::get_explorer_url(
                            tx_hash,
                            &withdrawal_request.network,
                            &withdrawal_request.environment,
                        ));
                    }

                    // Update totals
                    total_transferred += wallet_usage
                        .amount_used
                        .parse::<Decimal>()
                        .unwrap_or_default();
                    total_fees += wallet_usage.fee_paid.parse::<Decimal>().unwrap_or_default();

                    wallet_usages.push(wallet_usage);

                    log::info!(
                        "✅ Wallet {} processed successfully",
                        selected_wallet.wallet_address
                    );
                }
                Err(e) => {
                    log::error!(
                        " Failed to process wallet {}: {}",
                        selected_wallet.wallet_address,
                        e
                    );

                    // Create failed wallet usage record
                    wallet_usages.push(WalletUsage {
                        wallet_id: selected_wallet.wallet_id.clone(),
                        wallet_address: selected_wallet.wallet_address.clone(),
                        amount_used: "0".to_string(),
                        fee_paid: "0".to_string(),
                        tx_hash: None,
                        status: "failed".to_string(),
                        balance_before: selected_wallet.available_balance.clone(),
                        balance_after: selected_wallet.available_balance.clone(),
                    });

                    // For now, continue with other wallets even if one fails
                    // In production, you might want different strategies (fail-fast vs continue)
                }
            }
        }

        // Update withdrawal record with final status
        let final_status = if wallet_usages
            .iter()
            .any(|w| w.status == "confirmed" || w.status == "pending")
        {
            if wallet_usages.iter().all(|w| w.status == "confirmed") {
                "confirmed"
            } else {
                "processing"
            }
        } else {
            "failed"
        };

        let mut update_withdrawal: withdrawal_request::ActiveModel = saved_withdrawal.into();
        update_withdrawal.status = Set(final_status.to_string());
        update_withdrawal.tx_hash = Set(tx_hashes.first().cloned()); // Primary tx hash
        update_withdrawal.fee = Set(Some(total_fees));
        update_withdrawal.processed_at = Set(Some(Utc::now().into()));
        update_withdrawal.updated_at = Set(Utc::now().into());

        if final_status == "failed" {
            update_withdrawal.failed_at = Set(Some(Utc::now().into()));
            update_withdrawal.failure_reason =
                Set(Some("One or more wallet transactions failed".to_string()));
        }

        let final_withdrawal = update_withdrawal.update(db).await?;

        // Create response
        let response = WithdrawalResponse {
            id: final_withdrawal.id,
            merchant_id: final_withdrawal.merchant_id,
            external_id: Some(final_withdrawal.external_id),
            idempotency_key: final_withdrawal.idempotency_key,
            environment: final_withdrawal.environment,
            network: final_withdrawal.network,
            currency: final_withdrawal.currency,
            to_address: final_withdrawal.to_address,
            amount: final_withdrawal.amount.to_string(), // Backward compatibility
            requested_amount: final_withdrawal.amount.to_string(),
            transferred_amount: total_transferred.to_string(),
            amount_type: amount_type.to_string(),
            usd_equivalent: None,              // TODO: Calculate USD equivalent
            fee: Some(total_fees.to_string()), // Backward compatibility
            total_fee: Some(total_fees.to_string()),
            net_amount: (total_transferred - total_fees).to_string(),
            status: final_status.to_string(),
            wallets_used: wallet_usages,
            tx_hash: tx_hashes.first().cloned(), // Backward compatibility - first tx hash
            tx_hashes,
            confirmations: 0,               // Backward compatibility - default to 0
            blockchain_confirmations: None, // Will be updated by monitoring
            required_confirmations: Self::get_required_confirmations(&withdrawal_request.network),
            explorer_url: explorer_urls.first().cloned(), // Backward compatibility - first URL
            explorer_urls,
            created_at: final_withdrawal.created_at.into(),
            processed_at: final_withdrawal.processed_at.map(|dt| dt.into()),
            confirmed_at: final_withdrawal.confirmed_at.map(|dt| dt.into()),
            updated_at: final_withdrawal.updated_at.into(),
            broadcast_at: final_withdrawal.broadcast_at.map(|dt| dt.into()),
            failed_at: final_withdrawal.failed_at.map(|dt| dt.into()),
            failure_reason: final_withdrawal.failure_reason,
        };

        log::info!(
            "🎉 Multi-wallet withdrawal completed: {} transferred from {} wallets",
            total_transferred,
            response.wallets_used.len()
        );

        Ok(response)
    }

    /// Create withdrawal plan for maximum balance withdrawal
    async fn create_max_withdrawal_plan(
        &self,
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: &str,
        network: &str,
    ) -> Result<WithdrawalPlan, AppError> {
        let aggregated =
            MultiWalletService::get_aggregated_balances(db, merchant_id, environment).await?;

        // Find matching currency group
        let currency_group = aggregated
            .into_iter()
            .find(|group| MultiWalletService::currency_matches(network, &group.currency))
            .ok_or_else(|| {
                AppError::ValidationError(format!("No wallets found for network: {}", network))
            })?;

        let total_crypto: Decimal = currency_group
            .total_crypto_amount
            .parse()
            .unwrap_or_default();

        // Use largest_first strategy for max withdrawal to minimize transactions
        MultiWalletService::select_wallets_for_withdrawal(
            db,
            merchant_id,
            environment,
            network,
            total_crypto,
            "largest_first",
        )
        .await
    }

    /// Execute transaction for a single wallet
    async fn execute_single_wallet_transaction(
        &self,
        db: &DatabaseConnection,
        withdrawal_id: &str,
        selected_wallet: &SelectedWallet,
        to_address: &str,
        network: &str,
        environment: &str,
    ) -> Result<WalletUsage, AppError> {
        log::info!(
            "🔄 Executing transaction: {} {} from wallet {}",
            selected_wallet.amount_to_transfer,
            network,
            selected_wallet.wallet_address
        );

        // Get wallet record from database
        let wallet = Wallet::find()
            .filter(wallet::Column::Id.eq(&selected_wallet.wallet_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Wallet not found".to_string()))?;

        let amount_to_transfer: Decimal = selected_wallet
            .amount_to_transfer
            .parse()
            .unwrap_or_default();
        let balance_before: Decimal = selected_wallet
            .available_balance
            .parse()
            .unwrap_or_default();

        // Execute transaction based on network
        let (tx_hash, actual_fee) = match network.to_lowercase().as_str() {
            "sol" | "solana" => {
                self.execute_solana_transaction(
                    &wallet,
                    to_address,
                    amount_to_transfer,
                    environment,
                )
                .await?
            }
            "eth" | "ethereum" => {
                // TODO: Implement Ethereum withdrawal
                return Err(AppError::ValidationError(
                    "Ethereum withdrawals not yet implemented".to_string(),
                ));
            }
            "btc" | "bitcoin" => {
                // TODO: Implement Bitcoin withdrawal
                return Err(AppError::ValidationError(
                    "Bitcoin withdrawals not yet implemented".to_string(),
                ));
            }
            _ => {
                return Err(AppError::ValidationError(format!(
                    "Unsupported network: {}",
                    network
                )));
            }
        };

        // Update wallet balance
        let balance_after = balance_before - amount_to_transfer - actual_fee;
        self.update_wallet_balance(db, &selected_wallet.wallet_id, balance_after)
            .await?;

        // Create wallet transaction record
        self.create_wallet_transaction_record(
            db,
            &selected_wallet.wallet_id,
            withdrawal_id,
            amount_to_transfer,
            actual_fee,
            balance_before,
            balance_after,
            &tx_hash,
            &wallet.address,
            to_address,
        )
        .await?;

        Ok(WalletUsage {
            wallet_id: selected_wallet.wallet_id.clone(),
            wallet_address: selected_wallet.wallet_address.clone(),
            amount_used: amount_to_transfer.to_string(),
            fee_paid: actual_fee.to_string(),
            tx_hash: Some(tx_hash),
            status: "pending".to_string(), // Will be updated by monitoring
            balance_before: balance_before.to_string(),
            balance_after: balance_after.to_string(),
        })
    }

    /// Execute Solana transaction
    async fn execute_solana_transaction(
        &self,
        wallet: &wallet::Model,
        to_address: &str,
        amount: Decimal,
        environment: &str,
    ) -> Result<(String, Decimal), AppError> {
        let solana_service = SolanaMainnetWithdrawal::new(self.app_state.db.clone());

        let result = solana_service
            .execute_withdrawal(&wallet.id, to_address, amount, environment)
            .await?;

        let actual_fee = Decimal::try_from(result.fee_lamports as f64 / 1_000_000_000.0)
            .unwrap_or_else(|_| Decimal::try_from(0.000005).unwrap());

        Ok((result.transaction_hash, actual_fee))
    }

    /// Update wallet balance after transaction
    async fn update_wallet_balance(
        &self,
        db: &DatabaseConnection,
        wallet_id: &str,
        new_balance: Decimal,
    ) -> Result<(), AppError> {
        let balance_record = WalletBalance::find()
            .filter(wallet_balance::Column::WalletId.eq(wallet_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Wallet balance not found".to_string()))?;

        let mut active_balance: wallet_balance::ActiveModel = balance_record.into();
        active_balance.available_balance = Set(new_balance);
        active_balance.total_balance = Set(new_balance); // Assuming no pending balance for simplicity
        active_balance.last_updated = Set(Utc::now().into());

        active_balance.update(db).await?;
        Ok(())
    }

    /// Create wallet transaction record
    async fn create_wallet_transaction_record(
        &self,
        db: &DatabaseConnection,
        wallet_id: &str,
        withdrawal_id: &str,
        amount: Decimal,
        fee: Decimal,
        balance_before: Decimal,
        balance_after: Decimal,
        tx_hash: &str,
        from_address: &str,
        to_address: &str,
    ) -> Result<(), AppError> {
        let transaction = wallet_transaction::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            wallet_id: Set(wallet_id.to_string()),
            transaction_type: Set("withdrawal".to_string()),
            currency: Set("multi_wallet_withdrawal".to_string()),
            amount: Set(amount),
            fee: Set(Some(fee)),
            net_amount: Set(-(amount + fee)), // Negative for outgoing
            balance_before: Set(balance_before),
            balance_after: Set(balance_after),
            tx_hash: Set(Some(tx_hash.to_string())),
            from_address: Set(Some(from_address.to_string())),
            to_address: Set(Some(to_address.to_string())),
            status: Set("pending".to_string()),
            description: Set(Some("Multi-wallet withdrawal transaction".to_string())),
            related_entity_type: Set(Some("withdrawal_request".to_string())),
            related_entity_id: Set(Some(withdrawal_id.to_string())),
            created_at: Set(Utc::now().into()),
            processed_at: Set(Some(Utc::now().into())),
            ..Default::default()
        };

        transaction.insert(db).await?;
        Ok(())
    }

    /// Get currency identifier for a network
    fn get_currency_for_network(network: &str) -> String {
        match network.to_lowercase().as_str() {
            "sol" | "solana" => "solana".to_string(),
            "eth" | "ethereum" => "ethereum".to_string(),
            "bnb" | "binance" => "bnb".to_string(),
            "btc" | "bitcoin" => "bitcoin".to_string(),
            "usdt_erc20" => "usdt".to_string(),
            "usdt_bep20" => "usdt".to_string(),
            "usdt_spl" => "usdt".to_string(),
            _ => network.to_string(),
        }
    }

    /// Get required confirmations for a network
    fn get_required_confirmations(network: &str) -> i32 {
        match network.to_lowercase().as_str() {
            "sol" | "solana" => 32,
            "eth" | "ethereum" => 12,
            "bnb" | "binance" => 15,
            "btc" | "bitcoin" => 6,
            _ => 3,
        }
    }

    /// Get explorer URL for a transaction
    fn get_explorer_url(tx_hash: &str, network: &str, environment: &str) -> String {
        match (network.to_lowercase().as_str(), environment) {
            ("sol" | "solana", "mainnet") => format!("https://solscan.io/tx/{}", tx_hash),
            ("sol" | "solana", "testnet") => {
                format!("https://solscan.io/tx/{}?cluster=testnet", tx_hash)
            }
            ("eth" | "ethereum", "mainnet") => format!("https://etherscan.io/tx/{}", tx_hash),
            ("eth" | "ethereum", "testnet") => {
                format!("https://sepolia.etherscan.io/tx/{}", tx_hash)
            }
            ("bnb" | "binance", "mainnet") => format!("https://bscscan.com/tx/{}", tx_hash),
            ("btc" | "bitcoin", _) => {
                format!("https://blockchair.com/bitcoin/transaction/{}", tx_hash)
            }
            _ => format!("Transaction: {}", tx_hash),
        }
    }
}
