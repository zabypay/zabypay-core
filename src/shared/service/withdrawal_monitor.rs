use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::shared::{
    entities::{prelude::*, wallet_balance, wallet_transaction, withdrawal_request},
    models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
    utils::errors::AppError,
};

/// Withdrawal monitoring service for safe withdrawals
/// Monitors blockchain confirmations and manages fund state transitions
pub struct WithdrawalMonitor {
    pub db: DatabaseConnection,
    pub monitoring_interval: Duration,
}

#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub network: String,
    pub finality_confirmations: i32,
    pub rpc_url: String,
}

impl WithdrawalMonitor {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            monitoring_interval: Duration::from_secs(30), // Check every 30 seconds
        }
    }

    /// Get chain configuration from database or use defaults
    async fn get_chain_config(
        &self,
        network: &str,
        environment: &str,
    ) -> Result<ChainConfig, AppError> {
        // In a real implementation, this would fetch from the chain_config table
        // For now, use hardcoded values matching our migration
        let (finality_confirmations, rpc_url) = match (network, environment) {
            ("eth", "mainnet") => (
                12,
                "https://eth-mainnet.g.alchemy.com/v2/your-api-key".to_string(),
            ),
            ("eth", "testnet") => (
                6,
                "https://eth-sepolia.g.alchemy.com/v2/your-api-key".to_string(),
            ),
            ("bnb", "mainnet") => (15, "https://bsc-dataseed.binance.org".to_string()),
            ("bnb", "testnet") => (
                10,
                "https://data-seed-prebsc-1-s1.binance.org:8545".to_string(),
            ),
            ("sol", "mainnet") => (32, "https://api.mainnet-beta.solana.com".to_string()),
            ("sol", "testnet") => (10, "https://api.testnet.solana.com".to_string()),
            ("btc", "mainnet") => (6, "https://bitcoin-mainnet-rpc-url".to_string()),
            ("btc", "testnet") => (3, "https://bitcoin-testnet-rpc-url".to_string()),
            _ => (12, "https://default-rpc-url".to_string()),
        };

        Ok(ChainConfig {
            network: network.to_string(),
            finality_confirmations,
            rpc_url,
        })
    }

    /// Check blockchain confirmation for a specific transaction
    async fn check_blockchain_confirmations(
        &self,
        tx_hash: &str,
        network: &str,
        rpc_url: &str,
    ) -> Result<Option<i32>, AppError> {
        match network {
            "sol" => {
                // For Solana, we can use the existing confirmation logic
                // Solana transactions are typically confirmed quickly
                // For now, return a mock confirmation count
                log::info!("Checking Solana confirmations for tx: {}", tx_hash);
                Ok(Some(32)) // Mock: assume confirmed
            }
            "eth" | "bnb" => {
                // For Ethereum/BSC, we would check with the RPC
                log::info!(
                    "Checking {}/{} confirmations for tx: {}",
                    network.to_uppercase(),
                    rpc_url,
                    tx_hash
                );
                Ok(Some(15)) // Mock: assume confirmed
            }
            "btc" => {
                // For Bitcoin, we would use a Bitcoin RPC client
                log::info!("Checking Bitcoin confirmations for tx: {}", tx_hash);
                Ok(Some(6)) // Mock: assume confirmed
            }
            _ => {
                log::warn!("Unknown network: {}", network);
                Ok(None)
            }
        }
    }

    /// Process a single withdrawal for confirmation monitoring
    async fn process_withdrawal_confirmation(
        &self,
        withdrawal: &withdrawal_request::Model,
    ) -> Result<bool, AppError> {
        let tx_hash = match &withdrawal.tx_hash {
            Some(hash) => hash,
            None => {
                log::debug!("Withdrawal {} has no transaction hash yet", withdrawal.id);
                return Ok(false); // Not ready for monitoring
            }
        };

        log::info!(
            "Monitoring withdrawal {} with tx_hash: {}",
            withdrawal.id,
            tx_hash
        );

        // Get chain configuration
        let chain_config = self
            .get_chain_config(&withdrawal.network, &withdrawal.environment)
            .await?;

        // Check current blockchain confirmations
        let current_confirmations = match self
            .check_blockchain_confirmations(tx_hash, &withdrawal.network, &chain_config.rpc_url)
            .await?
        {
            Some(confirmations) => confirmations,
            None => {
                log::error!(
                    "Failed to get confirmations for withdrawal {}",
                    withdrawal.id
                );
                return Ok(false);
            }
        };

        log::info!(
            "Withdrawal {} has {} confirmations (required: {})",
            withdrawal.id,
            current_confirmations,
            chain_config.finality_confirmations
        );

        // Update blockchain confirmations count
        let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.clone().into();
        withdrawal_active.blockchain_confirmations = Set(Some(current_confirmations));
        withdrawal_active.updated_at = Set(Utc::now().into());

        // Check if finality is reached
        if current_confirmations >= chain_config.finality_confirmations {
            // Finality reached - transition to confirmed state
            log::info!(
                "🎉 Withdrawal {} reached finality! Deducting funds from balance",
                withdrawal.id
            );

            withdrawal_active.finality_reached_at = Set(Some(Utc::now().into()));
            withdrawal_active.lifecycle_state =
                Set(WithdrawalLifecycleState::Confirmed.to_string());
            withdrawal_active.withdrawal_funds_state =
                Set(WithdrawalFundsState::Confirmed.to_string());
            withdrawal_active.status = Set("confirmed".to_string());

            // Update withdrawal record
            withdrawal_active.update(&self.db).await.map_err(|e| {
                AppError::DatabaseError(format!("Failed to update withdrawal: {}", e))
            })?;

            // Now deduct funds from locked balance and total balance
            self.finalize_withdrawal_deduction(withdrawal).await?;

            return Ok(true); // Processing complete
        } else {
            // Still waiting for finality
            withdrawal_active.lifecycle_state =
                Set(WithdrawalLifecycleState::Processing.to_string());
            withdrawal_active.update(&self.db).await.map_err(|e| {
                AppError::DatabaseError(format!("Failed to update withdrawal: {}", e))
            })?;

            return Ok(false); // Continue monitoring
        }
    }

    /// Finalize withdrawal by deducting funds from locked and total balances
    async fn finalize_withdrawal_deduction(
        &self,
        withdrawal: &withdrawal_request::Model,
    ) -> Result<(), AppError> {
        log::info!(
            "Finalizing withdrawal deduction for withdrawal {}",
            withdrawal.id
        );

        // Get the wallet balance
        let wallet_balance = wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.eq(&withdrawal.wallet_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch wallet balance: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Wallet balance not found".to_string()))?;

        // Calculate total deduction (amount + fee)
        let fee = withdrawal.fee.unwrap_or(Decimal::ZERO);
        let total_deduction = withdrawal.amount + fee;

        // Deduct from locked balance and total balance
        let new_locked_balance = wallet_balance.locked_balance - total_deduction;
        let new_total_balance = wallet_balance.total_balance - total_deduction;

        log::info!(
            "Deducting {} from wallet {}: locked {} -> {}, total {} -> {}",
            total_deduction,
            withdrawal.wallet_id,
            wallet_balance.locked_balance,
            new_locked_balance,
            wallet_balance.total_balance,
            new_total_balance
        );

        // Store original balance before moving wallet_balance
        let original_total_balance = wallet_balance.total_balance;

        // Update wallet balance
        let mut balance_active: wallet_balance::ActiveModel = wallet_balance.into();
        balance_active.locked_balance = Set(new_locked_balance.max(Decimal::ZERO)); // Ensure non-negative
        balance_active.total_balance = Set(new_total_balance.max(Decimal::ZERO)); // Ensure non-negative
        balance_active.last_updated = Set(Utc::now().into());

        balance_active
            .update(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to update balance: {}", e)))?;

        // Create final wallet transaction record
        let final_transaction = wallet_transaction::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            wallet_id: Set(withdrawal.wallet_id.clone()),
            transaction_type: Set("withdrawal_confirmed".to_string()),
            currency: Set(withdrawal.currency.clone()),
            amount: Set(withdrawal.amount),
            fee: Set(withdrawal.fee),
            net_amount: Set(-withdrawal.net_amount), // Negative for debit
            balance_before: Set(original_total_balance),
            balance_after: Set(new_total_balance),
            tx_hash: Set(withdrawal.tx_hash.clone()),
            from_address: Set(None), // Will be populated from wallet
            to_address: Set(Some(withdrawal.to_address.clone())),
            status: Set("confirmed".to_string()),
            description: Set(Some("Withdrawal confirmed and deducted".to_string())),
            metadata: Set(Some(
                serde_json::json!({
                    "withdrawal_id": withdrawal.id,
                    "finality_reached": true,
                    "confirmations": withdrawal.blockchain_confirmations,
                    "safe_withdrawal": true
                })
                .to_string(),
            )),
            related_entity_type: Set(Some("withdrawal_request".to_string())),
            related_entity_id: Set(Some(withdrawal.id.clone())),
            created_at: Set(Utc::now().into()),
            processed_at: Set(Some(Utc::now().into())),
        };

        final_transaction.insert(&self.db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to create final transaction: {}", e))
        })?;

        log::info!("✅ Withdrawal {} finalized successfully", withdrawal.id);
        Ok(())
    }

    /// Monitor all pending withdrawals for confirmations
    pub async fn monitor_pending_withdrawals(&self) -> Result<(), AppError> {
        log::info!(" Starting withdrawal confirmation monitoring cycle");

        // Get all withdrawals that need monitoring
        let pending_withdrawals = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Status.is_in([
                "broadcasted",
                "pending",
                "processing",
            ]))
            .filter(
                withdrawal_request::Column::WithdrawalFundsState
                    .eq(WithdrawalFundsState::Locked.to_string()),
            )
            .filter(withdrawal_request::Column::TxHash.is_not_null())
            .all(&self.db)
            .await
            .map_err(|e| {
                AppError::DatabaseError(format!("Failed to fetch pending withdrawals: {}", e))
            })?;

        let total_count = pending_withdrawals.len();
        log::info!("Found {} withdrawals to monitor", total_count);

        let mut completed_count = 0;
        for withdrawal in pending_withdrawals {
            match self.process_withdrawal_confirmation(&withdrawal).await {
                Ok(true) => {
                    completed_count += 1;
                    log::info!(
                        "✅ Withdrawal {} completed confirmation process",
                        withdrawal.id
                    );
                }
                Ok(false) => {
                    log::debug!(
                        "⏳ Withdrawal {} still awaiting confirmations",
                        withdrawal.id
                    );
                }
                Err(e) => {
                    log::error!(" Error processing withdrawal {}: {:?}", withdrawal.id, e);
                }
            }
        }

        log::info!(
            " Monitoring cycle complete: {}/{} withdrawals finalized",
            completed_count,
            total_count
        );
        Ok(())
    }

    /// Start the monitoring loop (runs continuously)
    pub async fn start_monitoring(&self) -> Result<(), AppError> {
        log::info!(
            " Starting withdrawal monitoring service with interval: {:?}",
            self.monitoring_interval
        );

        loop {
            match self.monitor_pending_withdrawals().await {
                Ok(_) => {}
                Err(e) => {
                    log::error!("Error in monitoring cycle: {:?}", e);
                }
            }

            // Wait for next monitoring cycle
            sleep(self.monitoring_interval).await;
        }
    }

    /// Handle withdrawal failures (for future implementation)
    pub async fn handle_withdrawal_failure(
        &self,
        withdrawal_id: &str,
        reason: &str,
    ) -> Result<(), AppError> {
        log::warn!(
            "🚨 Handling withdrawal failure: {} - {}",
            withdrawal_id,
            reason
        );

        // Get withdrawal
        let withdrawal = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch withdrawal: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

        // Update withdrawal to failed state
        let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.clone().into();
        withdrawal_active.status = Set("failed".to_string());
        withdrawal_active.lifecycle_state = Set(WithdrawalLifecycleState::Failed.to_string());
        withdrawal_active.withdrawal_funds_state = Set(WithdrawalFundsState::Released.to_string());
        withdrawal_active.failed_at = Set(Some(Utc::now().into()));
        withdrawal_active.failure_reason = Set(Some(reason.to_string()));
        withdrawal_active.updated_at = Set(Utc::now().into());

        withdrawal_active.update(&self.db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to update failed withdrawal: {}", e))
        })?;

        // Release locked funds back to available balance
        self.release_locked_funds(&withdrawal).await?;

        log::info!(
            "✅ Withdrawal {} marked as failed and funds released",
            withdrawal_id
        );
        Ok(())
    }

    /// Release locked funds back to available balance
    async fn release_locked_funds(
        &self,
        withdrawal: &withdrawal_request::Model,
    ) -> Result<(), AppError> {
        log::info!(
            "🔄 Releasing locked funds for failed withdrawal {}",
            withdrawal.id
        );

        // Get wallet balance
        let wallet_balance = wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.eq(&withdrawal.wallet_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch wallet balance: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Wallet balance not found".to_string()))?;

        // Calculate total release amount (amount + fee)
        let fee = withdrawal.fee.unwrap_or(Decimal::ZERO);
        let total_release = withdrawal.amount + fee;

        // Store original values before moving wallet_balance
        let original_available_balance = wallet_balance.available_balance;
        let original_locked_balance = wallet_balance.locked_balance;

        // Move funds from locked back to available
        let new_locked_balance = original_locked_balance - total_release;
        let new_available_balance = original_available_balance + total_release;

        log::info!(
            "Releasing {} to wallet {}: locked {} -> {}, available {} -> {}",
            total_release,
            withdrawal.wallet_id,
            original_locked_balance,
            new_locked_balance,
            original_available_balance,
            new_available_balance
        );

        // Update wallet balance
        let mut balance_active: wallet_balance::ActiveModel = wallet_balance.into();
        balance_active.locked_balance = Set(new_locked_balance.max(Decimal::ZERO));
        balance_active.available_balance = Set(new_available_balance);
        balance_active.last_updated = Set(Utc::now().into());

        balance_active
            .update(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to update balance: {}", e)))?;

        // Create transaction record for fund release
        let release_transaction = wallet_transaction::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            wallet_id: Set(withdrawal.wallet_id.clone()),
            transaction_type: Set("withdrawal_refund".to_string()),
            currency: Set(withdrawal.currency.clone()),
            amount: Set(withdrawal.amount),
            fee: Set(withdrawal.fee),
            net_amount: Set(total_release), // Positive for credit
            balance_before: Set(original_available_balance),
            balance_after: Set(new_available_balance),
            tx_hash: Set(None),
            from_address: Set(None),
            to_address: Set(None),
            status: Set("completed".to_string()),
            description: Set(Some(
                "Withdrawal failed - funds released back to available balance".to_string(),
            )),
            metadata: Set(Some(
                serde_json::json!({
                    "withdrawal_id": withdrawal.id,
                    "failure_reason": withdrawal.failure_reason,
                    "released_amount": total_release,
                    "safe_withdrawal": true
                })
                .to_string(),
            )),
            related_entity_type: Set(Some("withdrawal_request".to_string())),
            related_entity_id: Set(Some(withdrawal.id.clone())),
            created_at: Set(Utc::now().into()),
            processed_at: Set(Some(Utc::now().into())),
        };

        release_transaction.insert(&self.db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to create release transaction: {}", e))
        })?;

        log::info!("✅ Locked funds released for withdrawal {}", withdrawal.id);
        Ok(())
    }
}
