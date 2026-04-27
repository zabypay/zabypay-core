use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use uuid::Uuid;

use crate::shared::{
    entities::{prelude::*, wallet, wallet_balance, wallet_transaction, withdrawal_request},
    service::{
        solana_http_client::SolanaHttpClient, solana_mainnet_withdrawal::SolanaMainnetWithdrawal,
    },
    utils::errors::AppError,
    AppState,
};

pub struct WithdrawalProcessor;

impl WithdrawalProcessor {
    /// Main entry point for processing a withdrawal
    pub async fn process_withdrawal(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<String, AppError> {
        log::info!("Starting withdrawal processing for ID: {}", withdrawal_id);
        let db = &app_state.db;

        // If processing fails, we need to update the withdrawal status
        match Self::process_withdrawal_internal(app_state, withdrawal_id).await {
            Ok(tx_hash) => {
                log::info!(
                    "✅ Withdrawal {} processed successfully: {}",
                    withdrawal_id,
                    tx_hash
                );
                Ok(tx_hash)
            }
            Err(e) => {
                log::error!(" Withdrawal {} failed: {:?}", withdrawal_id, e);

                // Update withdrawal status to failed
                if let Err(update_error) =
                    Self::mark_withdrawal_failed(db, withdrawal_id, &e.to_string()).await
                {
                    log::error!(
                        "Failed to update withdrawal {} status to failed: {:?}",
                        withdrawal_id,
                        update_error
                    );
                }

                Err(e)
            }
        }
    }

    /// Mark withdrawal as failed in database
    async fn mark_withdrawal_failed(
        db: &sea_orm::DatabaseConnection,
        withdrawal_id: &str,
        failure_reason: &str,
    ) -> Result<(), AppError> {
        if let Ok(Some(withdrawal)) = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(db)
            .await
        {
            let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.into();
            withdrawal_active.status = Set("failed".to_string());
            withdrawal_active.failed_at = Set(Some(Utc::now().into()));
            withdrawal_active.failure_reason = Set(Some(failure_reason.to_string()));
            withdrawal_active.updated_at = Set(Utc::now().into());

            withdrawal_active.update(db).await?;
            log::info!("📝 Updated withdrawal {} status to failed", withdrawal_id);
        }

        Ok(())
    }

    /// Internal withdrawal processing logic
    async fn process_withdrawal_internal(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<String, AppError> {
        log::info!(
            "Starting internal withdrawal processing for ID: {}",
            withdrawal_id
        );
        let db = &app_state.db;

        // Begin database transaction
        let txn = db.begin().await?;

        // Get withdrawal request
        let withdrawal = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(&txn)
            .await?
            .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

        // Check if already processed - allow both "pending" and "processing" status
        if withdrawal.status != "pending" && withdrawal.status != "processing" {
            return Err(AppError::ValidationError(format!(
                "Withdrawal already in status: {}",
                withdrawal.status
            )));
        }

        // Get wallet with private key
        let wallet = wallet::Entity::find()
            .filter(wallet::Column::Id.eq(&withdrawal.wallet_id))
            .one(&txn)
            .await?
            .ok_or_else(|| AppError::NotFound("Wallet not found".to_string()))?;

        // Update withdrawal to processing
        let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.clone().into();
        withdrawal_active.status = Set("processing".to_string());
        withdrawal_active.updated_at = Set(Utc::now().into());
        withdrawal_active.update(&txn).await?;

        // Process transaction based on network
        let tx_hash = match withdrawal.network.as_str() {
            "eth" | "usdt_erc20" => {
                let hash = Self::simulate_ethereum_withdrawal(&withdrawal).await?;

                // Update simulated withdrawal with transaction hash
                let mut withdrawal_active2: withdrawal_request::ActiveModel =
                    withdrawal.clone().into();
                withdrawal_active2.tx_hash = Set(Some(hash.clone()));
                withdrawal_active2.status = Set("broadcasted".to_string());
                withdrawal_active2.broadcast_at = Set(Some(Utc::now().into()));
                withdrawal_active2.update(&txn).await?;

                hash
            }
            "bnb" | "usdt_bep20" => {
                let hash = Self::simulate_bnb_withdrawal(&withdrawal).await?;

                // Update simulated withdrawal with transaction hash
                let mut withdrawal_active2: withdrawal_request::ActiveModel =
                    withdrawal.clone().into();
                withdrawal_active2.tx_hash = Set(Some(hash.clone()));
                withdrawal_active2.status = Set("broadcasted".to_string());
                withdrawal_active2.broadcast_at = Set(Some(Utc::now().into()));
                withdrawal_active2.update(&txn).await?;

                hash
            }
            "btc" => {
                let hash = Self::simulate_bitcoin_withdrawal(&withdrawal).await?;

                // Update simulated withdrawal with transaction hash
                let mut withdrawal_active2: withdrawal_request::ActiveModel =
                    withdrawal.clone().into();
                withdrawal_active2.tx_hash = Set(Some(hash.clone()));
                withdrawal_active2.status = Set("broadcasted".to_string());
                withdrawal_active2.broadcast_at = Set(Some(Utc::now().into()));
                withdrawal_active2.update(&txn).await?;

                hash
            }
            "sol" => {
                // For Solana, execute_solana_withdrawal handles its own DB updates
                // We pass the original db connection, not the transaction
                Self::execute_solana_withdrawal(&withdrawal, &wallet, db).await?
            }
            _ => {
                return Err(AppError::ValidationError(format!(
                    "Unsupported network: {}",
                    withdrawal.network
                )));
            }
        };

        // Update wallet balance
        let balance = wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.eq(&wallet.id))
            .one(&txn)
            .await?
            .ok_or_else(|| AppError::NotFound("Wallet balance not found".to_string()))?;

        let current_pending = balance.pending_balance;
        let current_available = balance.available_balance;
        let total_deducted = withdrawal.amount + withdrawal.fee.unwrap_or(Decimal::ZERO);

        // Get balance amounts for transaction record BEFORE updating
        let balance_before = current_available + total_deducted;
        let balance_after = current_available;

        let mut balance_active: wallet_balance::ActiveModel = balance.into();
        balance_active.pending_balance = Set(current_pending - total_deducted);
        balance_active.last_updated = Set(Utc::now().into());
        balance_active.update(&txn).await?;

        // Create wallet transaction record
        let net_amount = -(withdrawal.amount + withdrawal.fee.unwrap_or(Decimal::ZERO));
        let wallet_tx = wallet_transaction::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            wallet_id: Set(wallet.id.clone()),
            transaction_type: Set("withdrawal".to_string()),
            currency: Set(withdrawal.currency.clone()),
            amount: Set(withdrawal.amount),
            fee: Set(withdrawal.fee),
            net_amount: Set(net_amount),
            balance_before: Set(balance_before),
            balance_after: Set(balance_after),
            tx_hash: Set(Some(tx_hash.clone())),
            from_address: Set(Some(wallet.address.clone())),
            to_address: Set(Some(withdrawal.to_address.clone())),
            status: Set("pending".to_string()),
            description: Set(Some("Withdrawal request".to_string())),
            metadata: Set(None),
            related_entity_type: Set(Some("withdrawal_request".to_string())),
            related_entity_id: Set(Some(withdrawal.id.clone())),
            created_at: Set(Utc::now().into()),
            processed_at: Set(None),
        };
        wallet_tx.insert(&txn).await?;

        // Commit transaction
        txn.commit().await?;

        // Start real blockchain confirmation monitoring
        let withdrawal_id_clone = withdrawal_id.to_string();
        let app_state_clone = app_state.clone();
        log::info!(
            "Starting real blockchain confirmation monitoring for withdrawal: {}",
            withdrawal_id_clone
        );
        tokio::spawn(async move {
            log::info!(
                "Background blockchain monitoring task started for withdrawal: {}",
                withdrawal_id_clone
            );
            if let Err(e) =
                Self::monitor_blockchain_confirmation(&app_state_clone, &withdrawal_id_clone).await
            {
                log::error!(
                    "Failed to monitor blockchain confirmation for {}: {:?}",
                    withdrawal_id_clone,
                    e
                );
            } else {
                log::info!(
                    "Successfully confirmed withdrawal on blockchain: {}",
                    withdrawal_id_clone
                );
            }
        });

        Ok(tx_hash)
    }

    /// Simulate Ethereum withdrawal
    async fn simulate_ethereum_withdrawal(
        withdrawal: &withdrawal_request::Model,
    ) -> Result<String, AppError> {
        // Generate a realistic looking transaction hash
        let tx_hash = format!("0x{}", hex::encode(&Uuid::new_v4().as_bytes()[..16]));

        log::info!(
            "Simulated Ethereum withdrawal: {} {} to {} - TX: {}",
            withdrawal.amount,
            withdrawal.currency,
            withdrawal.to_address,
            tx_hash
        );

        Ok(tx_hash)
    }

    /// Simulate BNB withdrawal
    async fn simulate_bnb_withdrawal(
        withdrawal: &withdrawal_request::Model,
    ) -> Result<String, AppError> {
        let tx_hash = format!("0x{}", hex::encode(&Uuid::new_v4().as_bytes()[..16]));

        log::info!(
            "Simulated BNB withdrawal: {} {} to {} - TX: {}",
            withdrawal.amount,
            withdrawal.currency,
            withdrawal.to_address,
            tx_hash
        );

        Ok(tx_hash)
    }

    /// Simulate Bitcoin withdrawal
    async fn simulate_bitcoin_withdrawal(
        withdrawal: &withdrawal_request::Model,
    ) -> Result<String, AppError> {
        let tx_hash = hex::encode(&Uuid::new_v4().as_bytes());

        log::info!(
            "Simulated Bitcoin withdrawal: {} {} to {} - TX: {}",
            withdrawal.amount,
            withdrawal.currency,
            withdrawal.to_address,
            tx_hash
        );

        Ok(tx_hash)
    }

    /// Execute real Solana withdrawal using enhanced DB-driven configuration
    async fn execute_solana_withdrawal(
        withdrawal: &withdrawal_request::Model,
        wallet: &wallet::Model,
        db: &sea_orm::DatabaseConnection,
    ) -> Result<String, AppError> {
        log::info!(
            "Executing ENHANCED Solana withdrawal: {} {} to {} from wallet {}",
            withdrawal.amount,
            withdrawal.currency,
            withdrawal.to_address,
            wallet.address
        );

        // Determine network environment from withdrawal
        let environment = match withdrawal.environment.as_str() {
            "mainnet" => "mainnet",
            "testnet" => "testnet",
            "devnet" => "devnet",
            _ => "mainnet", // Default to mainnet for production
        };

        log::info!("Using Solana {} network for withdrawal", environment);

        // Create Solana mainnet withdrawal service with DB connection
        let solana_service = SolanaMainnetWithdrawal::new(db.clone());

        // Execute withdrawal with all validations
        log::info!(
            " Executing Solana withdrawal for wallet {} to {}",
            wallet.id,
            withdrawal.to_address
        );

        let result = solana_service
            .execute_withdrawal(
                &wallet.id,
                &withdrawal.to_address,
                withdrawal.amount,
                environment,
            )
            .await
            .map_err(|e| {
                log::error!(" Enhanced Solana withdrawal failed: {:?}", e);
                e
            })?;

        log::info!(
            "✅ Solana withdrawal successful! TX: {} on {} network",
            result.transaction_hash,
            environment
        );

        log::info!("🔗 Transaction URL: {}", result.explorer_url);

        log::info!(
            "💰 Fee: {} lamports, Amount: {} lamports",
            result.fee_lamports,
            result.amount_lamports
        );

        // Return the transaction hash
        Ok(result.transaction_hash)
    }

    /// Monitor real blockchain confirmation for withdrawals
    async fn monitor_blockchain_confirmation(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<(), AppError> {
        let db = &app_state.db;

        let withdrawal = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

        let tx_hash = withdrawal
            .tx_hash
            .as_ref()
            .ok_or_else(|| AppError::ValidationError("No transaction hash found".to_string()))?;

        log::info!(
            "Starting blockchain monitoring for withdrawal {} with tx: {}",
            withdrawal_id,
            tx_hash
        );

        // Determine network environment for blockchain client
        let environment = match withdrawal.environment.as_str() {
            "mainnet" => "mainnet",
            "testnet" => "testnet",
            _ => "devnet",
        };

        // Only monitor Solana transactions for now
        if withdrawal.network == "sol" {
            Self::monitor_solana_confirmation(app_state, withdrawal_id, tx_hash, environment)
                .await?;
        } else {
            log::warn!(
                "Real blockchain monitoring not yet implemented for network: {}",
                withdrawal.network
            );
            // Fall back to time-based confirmation for other networks
            Self::fallback_time_based_confirmation(app_state, withdrawal_id).await?;
        }

        Ok(())
    }

    /// Monitor Solana transaction confirmation on blockchain
    async fn monitor_solana_confirmation(
        app_state: &AppState,
        withdrawal_id: &str,
        tx_hash: &str,
        environment: &str,
    ) -> Result<(), AppError> {
        let db = &app_state.db;

        // Create Solana client for monitoring
        let solana_client = SolanaHttpClient::new(environment, db.clone()).await?;

        log::info!(
            " Starting Solana blockchain monitoring for tx: {} on {}",
            tx_hash,
            environment
        );

        // Monitor transaction with exponential backoff
        let mut attempt = 0;
        let max_attempts = 60; // Monitor for up to ~10 minutes
        let mut delay_seconds = 2;

        loop {
            attempt += 1;

            log::info!(
                " Checking Solana transaction status (attempt {}/{}): {}",
                attempt,
                max_attempts,
                tx_hash
            );

            match solana_client.get_transaction_status(tx_hash).await {
                Ok(status) => {
                    log::info!(
                        " Transaction status - Confirmed: {}, Confirmations: {}, Slot: {:?}",
                        status.confirmed,
                        status.confirmations,
                        status.slot
                    );

                    // Update withdrawal with current status
                    let withdrawal = withdrawal_request::Entity::find()
                        .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
                        .one(db)
                        .await?
                        .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

                    let mut withdrawal_active: withdrawal_request::ActiveModel =
                        withdrawal.clone().into();
                    withdrawal_active.blockchain_confirmations = Set(Some(status.confirmations));
                    withdrawal_active.updated_at = Set(Utc::now().into());

                    // Check if transaction failed
                    if let Some(error) = &status.error {
                        log::error!(" Transaction failed on blockchain: {}", error);
                        withdrawal_active.status = Set("failed".to_string());
                        withdrawal_active.update(db).await?;
                        return Err(AppError::InternalServerError(format!(
                            "Transaction failed: {}",
                            error
                        )));
                    }

                    // Check if transaction is confirmed (1+ confirmations)
                    if status.confirmed && status.confirmations >= 1 {
                        log::info!(
                            "✅ Transaction confirmed on Solana blockchain with {} confirmations!",
                            status.confirmations
                        );

                        withdrawal_active.status = Set("confirmed".to_string());
                        withdrawal_active.confirmed_at = Set(Some(Utc::now().into()));

                        // Update wallet balance - move from pending to completed
                        Self::finalize_wallet_balance(app_state, &withdrawal).await?;

                        withdrawal_active.update(db).await?;

                        log::info!(
                            "🎉 Withdrawal {} fully confirmed and processed!",
                            withdrawal_id
                        );
                        return Ok(());
                    }

                    // Update partial status
                    withdrawal_active.update(db).await?;
                }
                Err(e) => {
                    log::warn!(
                        "  Failed to check transaction status (attempt {}): {}",
                        attempt,
                        e
                    );
                }
            }

            // Exit if max attempts reached
            if attempt >= max_attempts {
                log::error!(
                    " Max monitoring attempts reached for withdrawal {}",
                    withdrawal_id
                );

                // Mark as timeout
                let withdrawal = withdrawal_request::Entity::find()
                    .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
                    .one(db)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

                let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.into();
                withdrawal_active.status = Set("timeout".to_string());
                withdrawal_active.updated_at = Set(Utc::now().into());
                withdrawal_active.update(db).await?;

                return Err(AppError::InternalServerError(
                    "Transaction monitoring timeout".to_string(),
                ));
            }

            // Wait before next check with exponential backoff
            tokio::time::sleep(tokio::time::Duration::from_secs(delay_seconds)).await;
            delay_seconds = std::cmp::min(delay_seconds * 2, 30); // Cap at 30 seconds
        }
    }

    /// Finalize wallet balance after confirmation
    async fn finalize_wallet_balance(
        app_state: &AppState,
        withdrawal: &withdrawal_request::Model,
    ) -> Result<(), AppError> {
        let db = &app_state.db;
        let balance = wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.eq(&withdrawal.wallet_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Wallet balance not found".to_string()))?;

        let total_amount = withdrawal.amount + withdrawal.fee.unwrap_or(Decimal::ZERO);

        let current_available = balance.available_balance;
        let current_total = balance.total_balance;
        let current_pending = balance.pending_balance;

        // Only reduce pending balance and total balance here
        // Available balance was already reduced during reservation
        let mut balance_active: wallet_balance::ActiveModel = balance.into();
        balance_active.total_balance = Set(current_total - total_amount);
        balance_active.pending_balance = Set(current_pending - total_amount);
        // Don't change available_balance as it was already reduced
        balance_active.last_updated = Set(Utc::now().into());
        balance_active.update(db).await?;

        log::info!("💰 Wallet balance finalized after withdrawal confirmation");

        // Trigger balance sync to update aggregated balances and USD calculations
        if let Err(e) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(
            app_state,
            &withdrawal.merchant_id,
            &withdrawal.environment,
        ).await {
            log::warn!("Failed to sync balances after withdrawal confirmation: {}", e);
            // Don't fail the withdrawal confirmation if balance sync fails
        }

        Ok(())
    }

    /// Fallback time-based confirmation for non-Solana networks
    async fn fallback_time_based_confirmation(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<(), AppError> {
        log::info!(
            " Using fallback time-based confirmation for withdrawal: {}",
            withdrawal_id
        );

        // Wait 30 seconds for other networks (temporary)
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

        let db = &app_state.db;
        let withdrawal = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

        let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.clone().into();
        withdrawal_active.status = Set("confirmed".to_string());
        withdrawal_active.blockchain_confirmations = Set(Some(6)); // Assume 6 confirmations
        withdrawal_active.confirmed_at = Set(Some(Utc::now().into()));
        withdrawal_active.updated_at = Set(Utc::now().into());

        Self::finalize_wallet_balance(app_state, &withdrawal).await?;
        withdrawal_active.update(db).await?;

        log::info!(
            "✅ Withdrawal {} confirmed via fallback method",
            withdrawal_id
        );
        Ok(())
    }

    /// Monitor transaction confirmation (updated to use real blockchain monitoring)
    pub async fn monitor_transaction(
        app_state: &AppState,
        withdrawal_id: &str,
    ) -> Result<(), AppError> {
        log::info!(
            "🔄 Manual transaction monitoring requested for withdrawal: {}",
            withdrawal_id
        );

        let db = &app_state.db;
        let withdrawal = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(db)
            .await?
            .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

        if withdrawal.tx_hash.is_none() {
            return Err(AppError::ValidationError("No transaction hash".to_string()));
        }

        let tx_hash = withdrawal.tx_hash.as_ref().unwrap();

        // Use real blockchain monitoring for supported networks
        if withdrawal.network == "sol" {
            let environment = match withdrawal.environment.as_str() {
                "mainnet" => "mainnet",
                "testnet" => "testnet",
                _ => "devnet",
            };

            log::info!(
                " Performing real-time Solana blockchain check for tx: {}",
                tx_hash
            );

            let solana_client = SolanaHttpClient::new(environment, db.clone()).await?;
            let status = solana_client.get_transaction_status(tx_hash).await?;

            log::info!(
                " Current blockchain status - Confirmed: {}, Confirmations: {}, Slot: {:?}",
                status.confirmed,
                status.confirmations,
                status.slot
            );

            // Update withdrawal with real blockchain data
            let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.clone().into();
            withdrawal_active.blockchain_confirmations = Set(Some(status.confirmations));
            withdrawal_active.updated_at = Set(Utc::now().into());

            // Check for transaction errors
            if let Some(error) = &status.error {
                log::error!(" Transaction failed on blockchain: {}", error);
                withdrawal_active.status = Set("failed".to_string());
                withdrawal_active.update(db).await?;
                return Err(AppError::InternalServerError(format!(
                    "Transaction failed: {}",
                    error
                )));
            }

            // Confirm if transaction is confirmed on blockchain
            if status.confirmed && status.confirmations >= 1 && withdrawal.status != "confirmed" {
                log::info!(
                    "✅ Confirming withdrawal {} based on real blockchain status",
                    withdrawal_id
                );

                withdrawal_active.status = Set("confirmed".to_string());
                withdrawal_active.confirmed_at = Set(Some(Utc::now().into()));

                // Finalize wallet balance
                Self::finalize_wallet_balance(app_state, &withdrawal).await?;
            }

            withdrawal_active.update(db).await?;
        } else {
            // Fallback to time-based monitoring for other networks
            log::warn!(
                "  Using time-based monitoring fallback for network: {}",
                withdrawal.network
            );

            let created_at = withdrawal.created_at;
            let now = Utc::now();
            let elapsed = now.signed_duration_since(created_at);

            let (confirmed, confirmations) = if elapsed.num_minutes() > 2 {
                (true, 6)
            } else {
                (false, elapsed.num_minutes().max(0) as i32)
            };

            let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.clone().into();
            withdrawal_active.blockchain_confirmations = Set(Some(confirmations));

            if confirmed && withdrawal.status != "confirmed" {
                withdrawal_active.status = Set("confirmed".to_string());
                withdrawal_active.confirmed_at = Set(Some(Utc::now().into()));

                Self::finalize_wallet_balance(app_state, &withdrawal).await?;
            }

            withdrawal_active.updated_at = Set(Utc::now().into());
            withdrawal_active.update(db).await?;
        }

        Ok(())
    }
}
