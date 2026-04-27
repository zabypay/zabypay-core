use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::shared::{
    entities::{merchant, payment_request, prelude::*},
    service::{
        blockchain_monitor::{
            BitcoinNetwork, BlockchainMonitor, EthereumNetwork, MonitoringConfig, NetworkProvider,
            SolanaNetwork, TransactionStatus,
        },
        // websocket_service::WebSocketPaymentService, // Temporarily disabled
        universal_address_monitor::UniversalAddressMonitor,
        webhook_service::WebhookService,
    },
    utils::errors::AppError,
    AppState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMonitorConfig {
    pub polling_interval_seconds: u64,
    pub max_monitoring_hours: u64,
    pub required_confirmations: u32,
    pub networks: Vec<NetworkConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub name: String,
    pub rpc_url: String,
    pub api_key: Option<String>,
    pub required_confirmations: u32,
    pub enabled: bool,
}

impl Default for PaymentMonitorConfig {
    fn default() -> Self {
        Self {
            polling_interval_seconds: 15, // Faster polling for USDT token detection
            max_monitoring_hours: 24,
            required_confirmations: 3,
            networks: vec![
                // Ethereum Mainnet - Primary for ETH and ERC-20 tokens (USDT, USDC)
                NetworkConfig {
                    name: "ethereum".to_string(),
                    rpc_url: "https://mainnet.infura.io/v3/eef650a32682456db1cb76fa3f4e1206"
                        .to_string(),
                    api_key: Some("eef650a32682456db1cb76fa3f4e1206".to_string()),
                    required_confirmations: 3,
                    enabled: true,
                },
                // Base Sepolia Testnet - For ETH testnet payments
                NetworkConfig {
                    name: "base_sepolia".to_string(),
                    rpc_url: "https://sepolia.base.org".to_string(), // Official Base Sepolia RPC endpoint
                    api_key: None,
                    required_confirmations: 1, // Testnet can use lower confirmations
                    enabled: true,
                },
                // Bitcoin Mainnet
                NetworkConfig {
                    name: "bitcoin".to_string(),
                    rpc_url: "https://blockstream.info/api".to_string(),
                    api_key: None,
                    required_confirmations: 6,
                    enabled: true,
                },
                // Solana Mainnet
                NetworkConfig {
                    name: "solana".to_string(),
                    rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
                    api_key: None,
                    required_confirmations: 1,
                    enabled: true,
                },
                // BNB Smart Chain Mainnet
                NetworkConfig {
                    name: "bnb".to_string(),
                    rpc_url: "https://bsc-dataseed.binance.org/".to_string(),
                    api_key: None,
                    required_confirmations: 3,
                    enabled: true,
                },
                // Polygon Mainnet (for MATIC)
                NetworkConfig {
                    name: "polygon".to_string(),
                    rpc_url: "https://polygon-rpc.com/".to_string(),
                    api_key: None,
                    required_confirmations: 3,
                    enabled: true,
                },
                // Avalanche Mainnet (for AVAX)
                NetworkConfig {
                    name: "avalanche".to_string(),
                    rpc_url: "https://api.avax.network/ext/bc/C/rpc".to_string(),
                    api_key: None,
                    required_confirmations: 3,
                    enabled: true,
                },
            ],
        }
    }
}

pub struct PaymentMonitorService {
    blockchain_monitor: Arc<RwLock<BlockchainMonitor>>,
    config: PaymentMonitorConfig,
    app_state: Arc<AppState>,
    is_running: Arc<RwLock<bool>>,
}

impl PaymentMonitorService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        let config = PaymentMonitorConfig::default();
        let mut blockchain_monitor = BlockchainMonitor::new();

        // Initialize blockchain networks
        for network_config in &config.networks {
            if network_config.enabled {
                let monitoring_config = MonitoringConfig {
                    network: network_config.name.clone(),
                    required_confirmations: network_config.required_confirmations,
                    polling_interval_seconds: config.polling_interval_seconds,
                    rpc_url: network_config.rpc_url.clone(),
                    api_key: network_config.api_key.clone(),
                };

                match network_config.name.as_str() {
                    "ethereum" | "bnb" | "base_sepolia" => {
                        blockchain_monitor.add_network(
                            NetworkProvider::Ethereum(EthereumNetwork::new(
                                network_config.rpc_url.clone(),
                            )),
                            monitoring_config,
                        );
                    }
                    "bitcoin" => {
                        blockchain_monitor.add_network(
                            NetworkProvider::Bitcoin(BitcoinNetwork::new(
                                network_config.rpc_url.clone(),
                            )),
                            monitoring_config,
                        );
                    }
                    "solana" => {
                        blockchain_monitor.add_network(
                            NetworkProvider::Solana(SolanaNetwork::new(
                                network_config.rpc_url.clone(),
                            )),
                            monitoring_config,
                        );
                    }
                    _ => {
                        log::warn!("Unknown network: {}", network_config.name);
                    }
                }
            }
        }

        Self {
            blockchain_monitor: Arc::new(RwLock::new(blockchain_monitor)),
            config,
            app_state,
            is_running: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn start_monitoring(&self) -> Result<(), AppError> {
        let mut is_running = self.is_running.write().await;
        if *is_running {
            return Err(AppError::ValidationError(
                "Payment monitoring is already running".to_string(),
            ));
        }
        *is_running = true;
        drop(is_running);

        log::info!(" Starting payment monitoring service...");
        log::info!(" Configuration:");
        log::info!(
            "   Polling interval: {} seconds",
            self.config.polling_interval_seconds
        );
        log::info!(
            "   Max monitoring: {} hours",
            self.config.max_monitoring_hours
        );
        log::info!(
            "   Required confirmations: {}",
            self.config.required_confirmations
        );

        let app_state = self.app_state.clone();
        let blockchain_monitor = self.blockchain_monitor.clone();
        let config = self.config.clone();
        let is_running = self.is_running.clone();

        let monitor_handle = tokio::spawn(async move {
            log::info!("🔄 Background monitoring task started");
            Self::monitoring_loop(app_state, blockchain_monitor, config, is_running).await;
            log::info!("📴 Background monitoring task finished");
        });

        // Give the monitoring task a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        log::info!(
            "✅ Payment monitoring service started with background task: {:?}",
            monitor_handle.is_finished()
        );

        Ok(())
    }

    pub async fn stop_monitoring(&self) -> Result<(), AppError> {
        let mut is_running = self.is_running.write().await;
        if !*is_running {
            return Err(AppError::ValidationError(
                "Payment monitoring is not running".to_string(),
            ));
        }
        *is_running = false;
        log::info!("🛑 Payment monitoring service stopped");
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    async fn monitoring_loop(
        app_state: Arc<AppState>,
        blockchain_monitor: Arc<RwLock<BlockchainMonitor>>,
        config: PaymentMonitorConfig,
        is_running: Arc<RwLock<bool>>,
    ) {
        log::info!(
            " [MONITOR_LOOP] Payment monitoring loop started with interval: {}s",
            config.polling_interval_seconds
        );

        let mut interval =
            tokio::time::interval(Duration::from_secs(config.polling_interval_seconds));
        // Skip the first immediate tick to align with interval
        interval.tick().await;

        let mut cycle_count = 0u64;

        loop {
            cycle_count += 1;
            log::info!(
                " [MONITOR_LOOP] Cycle #{} - Waiting for next tick...",
                cycle_count
            );

            // Wait for the next tick
            interval.tick().await;

            log::info!(
                " [MONITOR_LOOP] Cycle #{} - Tick received, checking running flag...",
                cycle_count
            );

            // Check if we should stop
            let should_stop = {
                let running = is_running.read().await;
                let is_running_val = *running;
                log::info!(
                    "🏃 [MONITOR_LOOP] Cycle #{} - Running flag: {}",
                    cycle_count,
                    is_running_val
                );
                !is_running_val
            };

            if should_stop {
                log::info!(
                    "🛑 [MONITOR_LOOP] Monitoring loop stopped by flag at cycle #{}",
                    cycle_count
                );
                break;
            }

            log::info!(
                "🔄 [MONITOR_LOOP] Cycle #{} - Starting payment monitoring at {}",
                cycle_count,
                Utc::now().format("%H:%M:%S")
            );

            match Self::check_pending_payments(&app_state, &blockchain_monitor, &config).await {
                Ok(_) => {
                    log::info!("✅ [MONITOR_LOOP] Cycle #{} - Payment monitoring completed successfully at {}", cycle_count, Utc::now().format("%H:%M:%S"));
                }
                Err(e) => {
                    log::error!(
                        " [MONITOR_LOOP] Cycle #{} - Error checking pending payments: {}",
                        cycle_count,
                        e
                    );
                }
            }

            // Cleanup old monitored transactions
            match Self::cleanup_old_transactions(&blockchain_monitor, config.max_monitoring_hours)
                .await
            {
                Ok(_) => {
                    log::info!(
                        "🧹 [MONITOR_LOOP] Cycle #{} - Transaction cleanup completed",
                        cycle_count
                    );
                }
                Err(e) => {
                    log::error!(
                        " [MONITOR_LOOP] Cycle #{} - Error cleaning up old transactions: {}",
                        cycle_count,
                        e
                    );
                }
            }

            log::info!(
                "⏱️ [MONITOR_LOOP] Cycle #{} - Waiting {}s until next monitoring cycle...",
                cycle_count,
                config.polling_interval_seconds
            );
        }

        log::info!(
            "🛑 [MONITOR_LOOP] Payment monitoring loop has exited after {} cycles",
            cycle_count
        );
    }

    async fn check_pending_payments(
        app_state: &AppState,
        blockchain_monitor: &Arc<RwLock<BlockchainMonitor>>,
        config: &PaymentMonitorConfig,
    ) -> Result<(), AppError> {
        // Get all pending payments (both expired and non-expired)
        let pending_payments = PaymentRequest::find()
            .filter(payment_request::Column::Status.eq("pending"))
            .all(&app_state.db)
            .await?;

        log::info!(" Checking {} pending payments", pending_payments.len());

        for payment in pending_payments {
            log::info!("🔎 Processing payment {} - Status: {}, Environment: '{}', Currency: {}, Amount: {}, Address: {}", 
                payment.id, payment.status, payment.environment, payment.currency, payment.amount, payment.wallet_address);

            // Check if payment has expired
            if payment.expires_at <= Utc::now() {
                log::info!(
                    " Payment {} expired at {}",
                    payment.id,
                    payment.expires_at
                );
                if let Err(e) = Self::expire_payment(app_state, &payment).await {
                    log::error!(" Error expiring payment {}: {}", payment.id, e);
                }
                continue;
            }

            // Check if we already have a transaction hash
            if let Some(tx_hash) = Self::extract_transaction_hash(&payment).await {
                if let Err(e) = Self::process_payment_transaction(
                    app_state,
                    blockchain_monitor,
                    &payment,
                    &tx_hash,
                )
                .await
                {
                    log::error!(" Error processing payment {}: {}", payment.id, e);
                }
            } else {
                // No transaction hash yet - check for incoming transactions by address
                if let Err(e) =
                    Self::check_address_for_payment(app_state, blockchain_monitor, config, &payment)
                        .await
                {
                    log::error!(
                        " Error checking address for payment {}: {}",
                        payment.id,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    async fn extract_transaction_hash(payment: &payment_request::Model) -> Option<String> {
        // Try to extract transaction hash from metadata or a dedicated field
        if let Some(metadata_str) = &payment.metadata {
            if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(metadata_str) {
                if let Some(tx_hash) = metadata.get("transaction_hash") {
                    if let Some(hash_str) = tx_hash.as_str() {
                        return Some(hash_str.to_string());
                    }
                }
            }
        }
        None
    }

    async fn process_payment_transaction(
        app_state: &AppState,
        blockchain_monitor: &Arc<RwLock<BlockchainMonitor>>,
        payment: &payment_request::Model,
        tx_hash: &str,
    ) -> Result<(), AppError> {
        // Check if this is a balance-detected transaction (synthetic hash)
        if tx_hash.starts_with("balance_detected_") {
            // Balance-detected transactions are already confirmed by the balance check
            log::info!(
                "🎉 Balance-detected payment {} is automatically confirmed",
                payment.id
            );
            Self::confirm_payment(app_state, payment, tx_hash).await?;
            return Ok(());
        }

        // For real transaction hashes, check if they already have sufficient confirmations in metadata
        if let Some(metadata_str) = &payment.metadata {
            if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(metadata_str) {
                if let Some(confirmations) = metadata.get("confirmations").and_then(|c| c.as_u64())
                {
                    // Get required confirmations based on environment and currency (consistent with detection logic)
                    let required_confirmations = match payment.environment.as_str() {
                        "testnet" => {
                            // Testnet: immediate confirmation to preserve working behavior
                            1
                        }
                        _ => {
                            // Mainnet: network-specific confirmation requirements
                            match payment.currency.to_lowercase().as_str() {
                                "ethereum" | "eth" => 12,
                                "bnb" | "binancecoin" => 15,
                                "bitcoin" | "btc" => 6,
                                "solana" | "sol" => 1,   // Keep Solana working
                                "usdt" | "tether" => 12, // USDT on Ethereum
                                "usdt_bnb" | "usdt_bep20" | "tether_bnb" => 15, // USDT on BSC
                                _ => 12, // Default mainnet requirement increased for tokens
                            }
                        }
                    };

                    if confirmations >= required_confirmations as u64 {
                        log::info!("🎉 Transaction {} already has {} confirmations (required: {}), confirming payment {}", 
                            tx_hash, confirmations, required_confirmations, payment.id);
                        Self::confirm_payment(app_state, payment, tx_hash).await?;
                        return Ok(());
                    } else {
                        log::debug!(
                            "⏳ Transaction {} has {} confirmations, need {} for payment {}",
                            tx_hash,
                            confirmations,
                            required_confirmations,
                            payment.id
                        );
                    }
                }
            }
        }

        let monitor = blockchain_monitor.read().await;

        // Start monitoring if not already monitoring
        let network = Self::get_network_for_currency(&payment.currency);
        if let Err(_) = monitor.check_transaction_status(tx_hash).await {
            // Transaction not being monitored, start monitoring
            drop(monitor);
            let monitor = blockchain_monitor.write().await;
            if let Err(e) = monitor
                .start_monitoring(&payment.id, tx_hash, &network)
                .await
            {
                log::warn!(" Could not start blockchain monitoring for {}: {}. Relying on address monitoring.", tx_hash, e);
            } else {
                log::info!(
                    "🎯 Started monitoring payment {} with tx {}",
                    payment.id,
                    tx_hash
                );
            }
            drop(monitor);
        }

        let monitor = blockchain_monitor.read().await;
        match monitor.check_transaction_status(tx_hash).await {
            Ok(TransactionStatus::Confirmed) => {
                drop(monitor);
                Self::confirm_payment(app_state, payment, tx_hash).await?;

                // Stop monitoring this transaction
                let monitor = blockchain_monitor.write().await;
                let _ = monitor.stop_monitoring(tx_hash).await;
            }
            Ok(TransactionStatus::Failed) => {
                drop(monitor);
                Self::fail_payment(app_state, payment, tx_hash).await?;

                // Stop monitoring this transaction
                let monitor = blockchain_monitor.write().await;
                let _ = monitor.stop_monitoring(tx_hash).await;
            }
            Ok(TransactionStatus::Pending) => {
                log::info!("⏳ Payment {} still pending (tx: {})", payment.id, tx_hash);
            }
            Ok(TransactionStatus::Dropped) => {
                drop(monitor);
                Self::fail_payment(app_state, payment, tx_hash).await?;

                // Stop monitoring this transaction
                let monitor = blockchain_monitor.write().await;
                let _ = monitor.stop_monitoring(tx_hash).await;
            }
            Err(e) => {
                log::warn!(" Error checking transaction {} via blockchain monitor: {}. Transaction may still be valid via address monitoring.", tx_hash, e);
            }
        }

        Ok(())
    }

    async fn expire_payment(
        app_state: &AppState,
        payment: &payment_request::Model,
    ) -> Result<(), AppError> {
        log::info!(
            " Expiring payment {} (expired at {})",
            payment.id,
            payment.expires_at
        );

        // Update payment status to expired
        let mut active_payment: payment_request::ActiveModel = payment.clone().into();
        active_payment.status = Set("expired".to_string());
        active_payment.updated_at = Set(Some(Utc::now().into()));

        // Update metadata with expiration info
        let mut metadata = if let Some(metadata_str) = &payment.metadata {
            serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        metadata["expired_at"] = serde_json::Value::String(Utc::now().to_rfc3339());
        metadata["expiry_reason"] =
            serde_json::Value::String("Payment timeout - no transaction received".to_string());

        active_payment.metadata = Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

        let updated_payment = active_payment.update(&app_state.db).await?;

        // Send webhook notification using new webhook service
        let webhook_service = WebhookService::new(Arc::new(app_state.clone()));
        if let Err(e) = webhook_service
            .send_payment_webhook(&updated_payment, "payment.expired")
            .await
        {
            log::error!(
                " Failed to send webhook for expired payment {}: {}",
                payment.id,
                e
            );
        } else {
            log::info!("📡 Webhook sent for expired payment {}", payment.id);
        }

        Ok(())
    }

    pub async fn confirm_payment(
        app_state: &AppState,
        payment: &payment_request::Model,
        tx_hash: &str,
    ) -> Result<(), AppError> {
        log::info!(
            "✅ Confirming payment {} with transaction {}",
            payment.id,
            tx_hash
        );

        // Use explicit database transaction to ensure atomicity
        let txn =
            app_state.db.begin().await.map_err(|e| {
                AppError::DatabaseError(format!("Failed to start transaction: {}", e))
            })?;

        // Fetch the latest payment from database to avoid stale data issues
        let current_payment = PaymentRequest::find_by_id(&payment.id)
            .one(&txn)
            .await?
            .ok_or(AppError::NotFound(
                "Payment not found during confirmation".to_string(),
            ))?;

        log::info!(
            "🔄 Current payment {} status in DB: {}",
            payment.id,
            current_payment.status
        );

        // Update payment status to paid
        let mut active_payment: payment_request::ActiveModel = current_payment.into();
        active_payment.status = Set("paid".to_string());
        active_payment.paid_at = Set(Some(Utc::now().into()));
        active_payment.updated_at = Set(Some(Utc::now().into()));

        // Update metadata with transaction hash
        let mut metadata = if let Some(metadata_str) = &payment.metadata {
            serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        metadata["transaction_hash"] = serde_json::Value::String(tx_hash.to_string());
        metadata["confirmed_at"] = serde_json::Value::String(Utc::now().to_rfc3339());

        active_payment.metadata = Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

        log::info!(
            "💾 Updating payment {} to 'paid' status in database transaction",
            payment.id
        );

        match active_payment.update(&txn).await {
            Ok(updated_payment) => {
                log::info!(
                    "✅ Payment {} updated in transaction, committing...",
                    payment.id
                );

                // Commit the transaction
                match txn.commit().await {
                    Ok(_) => {
                        log::info!(
                            "✅ Transaction committed! Payment {} status: {} at {}",
                            payment.id,
                            updated_payment.status,
                            updated_payment
                                .updated_at
                                .unwrap_or_else(|| Utc::now().into())
                        );

                        // Force connection pool to refresh and verify the update
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await; // Longer delay for DB consistency

                        // Verify using multiple attempts to ensure visibility
                        let mut verification_attempts = 3;
                        let mut verified = false;

                        while verification_attempts > 0 && !verified {
                            if let Ok(Some(verification_payment)) =
                                PaymentRequest::find_by_id(&payment.id)
                                    .one(&app_state.db)
                                    .await
                            {
                                if verification_payment.status == "paid" {
                                    log::info!(
                                        "✅ Verification successful: Payment {} status is now: {}",
                                        payment.id,
                                        verification_payment.status
                                    );
                                    verified = true;
                                } else {
                                    log::warn!("  Verification attempt {}: Payment {} status is still: {} (expected: paid)", 
                                        4 - verification_attempts, payment.id, verification_payment.status);
                                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                }
                            } else {
                                log::error!(
                                    " Failed to fetch payment {} for verification (attempt {})",
                                    payment.id,
                                    4 - verification_attempts
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            }
                            verification_attempts -= 1;
                        }

                        if !verified {
                            log::error!("🚨 CRITICAL: Payment {} update may not be visible to API endpoints after 3 verification attempts!", payment.id);
                            // Force a fresh connection query
                            if let Ok(fresh_connection) =
                                crate::shared::db::get_fresh_db_connection().await
                            {
                                if let Ok(Some(fresh_payment)) =
                                    PaymentRequest::find_by_id(&payment.id)
                                        .one(&fresh_connection)
                                        .await
                                {
                                    log::info!(
                                        "🔄 Fresh connection check: Payment {} status: {}",
                                        payment.id,
                                        fresh_payment.status
                                    );
                                    if fresh_payment.status == "paid" {
                                        log::info!("✅ Fresh connection confirmed payment {} is updated correctly", payment.id);
                                        verified = true;
                                    }
                                } else {
                                    log::error!(
                                        " Fresh connection query also failed for payment {}",
                                        payment.id
                                    );
                                }
                            }
                        }

                        // Send webhook notification using new webhook service
                        let webhook_service = WebhookService::new(Arc::new(app_state.clone()));
                        if let Err(e) = webhook_service
                            .send_payment_webhook(&updated_payment, "payment.confirmed")
                            .await
                        {
                            log::error!(
                                " Failed to send webhook for payment {}: {}",
                                payment.id,
                                e
                            );
                        } else {
                            log::info!("📡 Webhook sent for confirmed payment {}", payment.id);
                        }

                        // Send real-time WebSocket notification (temporarily disabled)
                        /*
                        if let Some(websocket_service) = &app_state.websocket_service {
                            websocket_service.notify_payment_update(&payment.id, "confirmed", Some(tx_hash)).await;
                            log::info!("📡 WebSocket notification sent for confirmed payment {}", payment.id);
                        } else {
                            log::warn!(" WebSocket service not available for payment {}", payment.id);
                        }
                        */

                        Ok(())
                    }
                    Err(e) => {
                        log::error!(
                            " Failed to commit transaction for payment {}: {}",
                            payment.id,
                            e
                        );
                        Err(AppError::DatabaseError(format!(
                            "Transaction commit failed: {}",
                            e
                        )))
                    }
                }
            }
            Err(e) => {
                log::error!(
                    " Failed to update payment {} in transaction: {}",
                    payment.id,
                    e
                );
                if let Err(rollback_err) = txn.rollback().await {
                    log::error!(" Failed to rollback transaction: {}", rollback_err);
                }
                Err(e.into())
            }
        }
    }

    async fn fail_payment(
        app_state: &AppState,
        payment: &payment_request::Model,
        tx_hash: &str,
    ) -> Result<(), AppError> {
        log::warn!(
            " Failing payment {} with transaction {}",
            payment.id,
            tx_hash
        );

        // Update payment status to failed
        let mut active_payment: payment_request::ActiveModel = payment.clone().into();
        active_payment.status = Set("failed".to_string());
        active_payment.updated_at = Set(Some(Utc::now().into()));

        // Update metadata with failure info
        let mut metadata = if let Some(metadata_str) = &payment.metadata {
            serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        metadata["transaction_hash"] = serde_json::Value::String(tx_hash.to_string());
        metadata["failed_at"] = serde_json::Value::String(Utc::now().to_rfc3339());

        active_payment.metadata = Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

        let updated_payment = active_payment.update(&app_state.db).await?;

        // Send webhook notification using new webhook service
        let webhook_service = WebhookService::new(Arc::new(app_state.clone()));
        if let Err(e) = webhook_service
            .send_payment_webhook(&updated_payment, "payment.failed")
            .await
        {
            log::error!(
                " Failed to send webhook for failed payment {}: {}",
                payment.id,
                e
            );
        } else {
            log::info!("📡 Webhook sent for failed payment {}", payment.id);
        }

        Ok(())
    }

    async fn cleanup_old_transactions(
        blockchain_monitor: &Arc<RwLock<BlockchainMonitor>>,
        max_hours: u64,
    ) -> Result<(), AppError> {
        let monitor = blockchain_monitor.read().await;
        let cleaned = monitor.cleanup_old_transactions(max_hours).await?;

        if cleaned > 0 {
            log::info!("🧹 Cleaned up {} old monitored transactions", cleaned);
        }

        Ok(())
    }

    fn get_network_for_currency(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            // Bitcoin
            "bitcoin" | "btc" => "bitcoin".to_string(),

            // Ethereum and ERC-20 tokens
            "ethereum" | "eth" => "ethereum".to_string(),
            "usdt" | "usdc" | "tether" | "usd-coin" => "ethereum".to_string(), // ERC-20 tokens on Ethereum

            // BNB Smart Chain and BEP-20 tokens
            "bnb" | "binance" | "binancecoin" => "bnb".to_string(),
            "usdt_bnb" | "usdt_bep20" | "tether_bnb" => "bnb".to_string(), // BEP-20 USDT on BNB Chain

            // Solana
            "solana" | "sol" => "solana".to_string(),

            // Polygon
            "matic" | "polygon" => "polygon".to_string(),

            // Avalanche
            "avax" | "avalanche" => "avalanche".to_string(),

            // Testnet networks
            "base_sepolia" => "base_sepolia".to_string(),

            // Handle environment-specific currencies (e.g., "base_sepolia_testnet")
            s if s.contains("_testnet") || s.contains("_mainnet") => {
                if s.contains("base_sepolia") {
                    "base_sepolia".to_string()
                } else if s.contains("bitcoin") {
                    "bitcoin".to_string()
                } else if s.contains("solana") {
                    "solana".to_string()
                } else if s.contains("bnb") {
                    "bnb".to_string()
                } else if s.contains("polygon") {
                    "polygon".to_string()
                } else if s.contains("avalanche") {
                    "avalanche".to_string()
                } else {
                    "ethereum".to_string() // Default EVM-compatible networks
                }
            }

            _ => "ethereum".to_string(), // Default to ethereum for unknown currencies
        }
    }

    async fn check_address_for_payment(
        app_state: &AppState,
        blockchain_monitor: &Arc<RwLock<BlockchainMonitor>>,
        _config: &PaymentMonitorConfig,
        payment: &payment_request::Model,
    ) -> Result<(), AppError> {
        let expected_amount = payment.amount.to_string().parse::<f64>().unwrap_or(0.0);

        log::info!(
            "🔎 [PAYMENT_MONITOR] Checking payment {}",
            serde_json::json!({
                "payment_id": payment.id,
                "network": payment.currency.to_uppercase(),
                "address": payment.wallet_address,
                "expected_amount": expected_amount,
                "status": payment.status,
                "environment": payment.environment,
                "created_at": payment.created_at.to_rfc3339(),
            })
        );

        // Use universal address monitor for all currency detection
        let monitor = UniversalAddressMonitor::new();

        // Determine effective currency based on payment environment
        let effective_currency = match payment.environment.as_str() {
            "testnet" | "devnet" => {
                log::info!(
                    "🧪 [{}] TESTNET environment, mapping {} to testnet networks",
                    payment.id,
                    payment.currency
                );
                match payment.currency.to_lowercase().as_str() {
                    "eth" | "ethereum" => "base_sepolia",
                    "sol" | "solana" => "solana_testnet",
                    _ => &payment.currency,
                }
            }
            _ => {
                log::info!("🌐 [{}] MAINNET environment", payment.id);
                match payment.currency.to_lowercase().as_str() {
                    "btc" => "bitcoin",
                    "eth" => "ethereum",
                    "sol" => "solana",
                    "bnb" => "bnb",
                    "matic" => "polygon",
                    "avax" => "avalanche",
                    _ => &payment.currency,
                }
            }
        };

        match monitor
            .check_payment_received(
                &payment.wallet_address,
                expected_amount,
                effective_currency,
                payment.created_at.into(),
            )
            .await
        {
            Ok(Some(tx)) => {
                log::info!("🎉 [{}] Transaction found! Hash: {}", payment.id, tx.hash);

                // Process detected payment (existing flow)
                Self::process_detected_native_payment(app_state, payment, tx).await?;

                return Ok(());
            }
            Ok(None) => {
                log::info!(
                    "⏳ [{}] No transaction found yet for {} {}",
                    payment.id,
                    expected_amount,
                    payment.currency.to_uppercase()
                );
            }
            Err(e) => {
                log::error!(" [{}] Transaction detection error: {}", payment.id, e);
            }
        }

        Ok(())
    }

    /// Process detected payment with proper environment-aware confirmation logic
    async fn process_detected_native_payment(
        app_state: &AppState,
        payment: &payment_request::Model,
        detected_tx: crate::shared::service::universal_address_monitor::DetectedTransaction,
    ) -> Result<(), AppError> {
        log::info!(
            "🎉 [{}] Processing detected payment: {} {} (Hash: {}, Confirmations: {})",
            payment.id,
            detected_tx.amount,
            payment.currency.to_uppercase(),
            detected_tx.hash,
            detected_tx.confirmations
        );

        // Get required confirmations based on environment and currency
        let required_confirmations = match payment.environment.as_str() {
            "testnet" => {
                // Testnet: immediate confirmation to preserve working behavior
                log::info!(
                    "🧪 [{}] TESTNET environment - immediate confirmation",
                    payment.id
                );
                1
            }
            _ => {
                // Mainnet: network-specific confirmation requirements
                match payment.currency.to_lowercase().as_str() {
                    "ethereum" | "eth" => 12,
                    "bnb" | "binancecoin" => 15,
                    "bitcoin" | "btc" => 6,
                    "solana" | "sol" => 1,   // Keep Solana working
                    "usdt" | "tether" => 12, // USDT on Ethereum
                    "usdt_bnb" | "usdt_bep20" | "tether_bnb" => 15, // USDT on BSC
                    _ => 12,                 // Default mainnet requirement for tokens
                }
            }
        };

        log::info!(
            " [{}] {} payment requires {} confirmations, transaction has {}",
            payment.id,
            payment.currency.to_uppercase(),
            required_confirmations,
            detected_tx.confirmations
        );

        // Create/update metadata
        let mut metadata = if let Some(metadata_str) = &payment.metadata {
            serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        metadata["transaction_hash"] = serde_json::Value::String(detected_tx.hash.clone());
        metadata["detected_at"] = serde_json::Value::String(Utc::now().to_rfc3339());
        metadata["amount_received"] = serde_json::Value::Number(
            serde_json::Number::from_f64(detected_tx.amount).unwrap_or(serde_json::Number::from(0)),
        );
        metadata["network"] = serde_json::Value::String(detected_tx.network.clone());
        metadata["confirmations"] =
            serde_json::Value::Number(serde_json::Number::from(detected_tx.confirmations));
        metadata["required_confirmations"] =
            serde_json::Value::Number(serde_json::Number::from(required_confirmations));
        metadata["from_address"] = serde_json::Value::String(detected_tx.from_address.clone());
        metadata["block_time"] = serde_json::Value::String(detected_tx.block_time.to_rfc3339());

        // Update payment metadata
        let txn = app_state.db.begin().await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to start payment transaction: {}", e))
        })?;

        let mut active_payment: payment_request::ActiveModel = payment.clone().into();
        active_payment.metadata = Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));
        active_payment.updated_at = Set(Some(Utc::now().into()));

        match active_payment.update(&txn).await {
            Ok(_) => match txn.commit().await {
                Ok(_) => {
                    log::info!("✅ [{}] Successfully updated payment metadata", payment.id);
                }
                Err(e) => {
                    log::error!(
                        " [{}] Failed to commit payment metadata: {}",
                        payment.id,
                        e
                    );
                    return Err(AppError::DatabaseError(format!(
                        "Metadata commit failed: {}",
                        e
                    )));
                }
            },
            Err(e) => {
                log::error!(
                    " [{}] Failed to update payment metadata: {}",
                    payment.id,
                    e
                );
                if let Err(rollback_err) = txn.rollback().await {
                    log::error!(
                        " Failed to rollback metadata transaction: {}",
                        rollback_err
                    );
                }
                return Err(e.into());
            }
        }

        // Check confirmation requirements
        if detected_tx.hash.starts_with("balance_detected_") {
            // Balance-detected payments (RPC fallback) - confirm immediately for all environments
            log::info!(
                "✅ [{}] Balance-detected payment - confirming immediately",
                payment.id
            );
            Self::confirm_payment(app_state, payment, &detected_tx.hash).await?;
        } else if detected_tx.confirmations >= required_confirmations {
            // Explorer API detected with sufficient confirmations
            log::info!(
                "✅ [{}] Payment has sufficient confirmations ({}/{}), confirming payment",
                payment.id,
                detected_tx.confirmations,
                required_confirmations
            );
            Self::confirm_payment(app_state, payment, &detected_tx.hash).await?;
        } else {
            // Transaction detected but needs more confirmations (mainnet only)
            log::info!(
                "⏳ [{}] Payment detected but needs more confirmations ({}/{}), will check again",
                payment.id,
                detected_tx.confirmations,
                required_confirmations
            );
            // Payment will be checked again in the next monitoring cycle until confirmed
        }

        log::info!("✅ [{}] Payment processed successfully", payment.id);
        Ok(())
    }

    // API Methods for manual control
    pub async fn start_monitoring_payment(
        &self,
        payment_id: &str,
        tx_hash: &str,
        network: Option<String>,
    ) -> Result<(), AppError> {
        let payment = PaymentRequest::find()
            .filter(payment_request::Column::Id.eq(payment_id))
            .one(&self.app_state.db)
            .await?
            .ok_or(AppError::NotFound("Payment not found".to_string()))?;

        let network = network.unwrap_or_else(|| Self::get_network_for_currency(&payment.currency));

        let monitor = self.blockchain_monitor.write().await;
        monitor
            .start_monitoring(payment_id, tx_hash, &network)
            .await?;

        log::info!(
            "🎯 Manually started monitoring payment {} with tx {} on {}",
            payment_id,
            tx_hash,
            network
        );
        Ok(())
    }

    pub async fn stop_monitoring_payment(&self, tx_hash: &str) -> Result<(), AppError> {
        let monitor = self.blockchain_monitor.write().await;
        monitor.stop_monitoring(tx_hash).await?;

        log::info!("🛑 Manually stopped monitoring transaction {}", tx_hash);
        Ok(())
    }

    pub async fn get_monitoring_status(&self) -> Result<serde_json::Value, AppError> {
        let monitor = self.blockchain_monitor.read().await;
        let monitored_transactions = monitor.get_monitored_transactions().await;
        let is_running = self.is_running().await;

        Ok(serde_json::json!({
            "is_running": is_running,
            "monitored_transactions_count": monitored_transactions.len(),
            "monitored_transactions": monitored_transactions,
            "config": self.config,
            "networks_enabled": self.config.networks.iter()
                .filter(|n| n.enabled)
                .map(|n| &n.name)
                .collect::<Vec<_>>()
        }))
    }
}
