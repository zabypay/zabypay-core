use crate::merchant::models::withdrawal::CreateWithdrawalRequest;
use crate::shared::{
    entities::{payment_request, wallet},
    utils::{encryption::CryptoEncryption, errors::AppError},
};
use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcWalletCandidate {
    pub address: String,
    pub balance_satoshis: u64,
    pub fee_satoshis: u64,
    pub transferable_satoshis: u64,
    pub private_key: String,
    pub utxos: Vec<BtcUtxo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcUtxo {
    pub txid: String,
    pub vout: u32,
    pub amount_satoshis: u64,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcMultiWalletResult {
    pub withdrawal_id: String,
    pub total_transferred_satoshis: u64,
    pub total_fee_satoshis: u64,
    pub wallets_used: Vec<BtcWalletUsage>,
    pub tx_hashes: Vec<String>,
    pub explorer_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcWalletUsage {
    pub address: String,
    pub transferred_satoshis: u64,
    pub fee_satoshis: u64,
    pub tx_hash: String,
    pub utxos_used: Vec<BtcUtxo>,
}

pub struct BtcMultiWalletService;

impl BtcMultiWalletService {
    /// Execute multi-wallet Bitcoin withdrawal by aggregating across deposit wallets
    pub async fn execute_multi_wallet_withdrawal(
        db: &DatabaseConnection,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
        payout_address: &str,
    ) -> Result<BtcMultiWalletResult, AppError> {
        log::info!(
            "🔄 Starting BTC multi-wallet withdrawal for merchant {} - {} BTC to {}",
            merchant_id,
            request.amount,
            payout_address
        );

        // Parse requested amount to satoshis
        let requested_amount_btc: f64 = request
            .amount
            .parse()
            .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;
        let requested_amount_satoshis = (requested_amount_btc * 100_000_000.0) as u64;

        log::info!(
            "💰 Requested amount: {} BTC ({} satoshis)",
            requested_amount_btc,
            requested_amount_satoshis
        );

        // Step 1: Discover deposit wallets from paid Bitcoin payments
        let mut deposit_wallets =
            Self::discover_deposit_wallets(db, merchant_id, &request.environment).await?;

        // Fallback: If no deposit wallets found, use regular merchant wallets
        if deposit_wallets.is_empty() {
            log::info!(
                "🔄 No deposit wallets found from payments, checking regular merchant wallets..."
            );
            deposit_wallets =
                Self::discover_merchant_wallets(db, merchant_id, &request.environment).await?;
        }

        if deposit_wallets.is_empty() {
            return Err(AppError::ValidationError(
                "No BTC wallets found for this merchant. Create some wallets first by receiving payments.".to_string()
            ));
        }

        log::info!(" Found {} potential deposit wallets", deposit_wallets.len());

        // Step 2: Evaluate each wallet for transferable balance
        let candidates =
            Self::evaluate_wallet_candidates(deposit_wallets, &request.environment).await?;

        if candidates.is_empty() {
            return Err(AppError::ValidationError(
                "No BTC wallets have transferable balance after fees".to_string(),
            ));
        }

        // Step 3: Calculate total transferable across all wallets
        let total_transferable_satoshis: u64 =
            candidates.iter().map(|c| c.transferable_satoshis).sum();
        let total_transferable_btc = total_transferable_satoshis as f64 / 100_000_000.0;

        log::info!(
            "💸 Total transferable across {} wallets: {} BTC ({} satoshis)",
            candidates.len(),
            total_transferable_btc,
            total_transferable_satoshis
        );

        if total_transferable_satoshis < requested_amount_satoshis {
            let shortage_satoshis = requested_amount_satoshis - total_transferable_satoshis;
            let shortage_btc = shortage_satoshis as f64 / 100_000_000.0;

            return Err(AppError::ValidationError(format!(
                "Insufficient BTC balance across all wallets: Requested {} BTC, available {} BTC (shortage: {} BTC)",
                requested_amount_btc, total_transferable_btc, shortage_btc
            )));
        }

        // Step 4: Select wallets and execute transfers
        let result = Self::execute_aggregated_transfers(
            candidates,
            requested_amount_satoshis,
            payout_address,
            &request.environment,
            &request.idempotency_key,
        )
        .await?;

        log::info!(
            "✅ Multi-wallet BTC withdrawal completed: {} satoshis from {} wallets",
            result.total_transferred_satoshis,
            result.wallets_used.len()
        );

        Ok(result)
    }

    /// Discover deposit wallets from paid Bitcoin payments
    async fn discover_deposit_wallets(
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: &str,
    ) -> Result<Vec<wallet::Model>, AppError> {
        log::info!(" Discovering deposit wallets from paid BTC payments...");

        // Find all paid Bitcoin payments to get wallet addresses
        let paid_payments = payment_request::Entity::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Environment.eq(environment))
            .filter(payment_request::Column::Currency.eq("BTC"))
            .filter(payment_request::Column::Status.eq("paid"))
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query payments: {}", e)))?;

        log::info!(" Found {} paid BTC payments", paid_payments.len());

        // Extract unique wallet addresses from payments
        let mut wallet_addresses: Vec<String> = paid_payments
            .into_iter()
            .map(|p| p.wallet_address)
            .collect();

        // Remove duplicates
        wallet_addresses.sort();
        wallet_addresses.dedup();

        log::info!(
            "🔑 Found {} unique deposit wallet addresses",
            wallet_addresses.len()
        );

        // Query wallet details with credentials
        let currency_pattern = format!("btc_{}", environment);
        let wallets = wallet::Entity::find()
            .filter(wallet::Column::Address.is_in(wallet_addresses))
            .filter(wallet::Column::Currency.contains(&currency_pattern))
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query wallets: {}", e)))?;

        log::info!("💼 Retrieved {} wallets with credentials", wallets.len());

        Ok(wallets)
    }

    /// Discover regular merchant wallets as fallback
    async fn discover_merchant_wallets(
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: &str,
    ) -> Result<Vec<wallet::Model>, AppError> {
        log::info!(
            " Discovering regular merchant wallets for BTC {}...",
            environment
        );

        // Get merchant's user_id first
        use crate::shared::entities::merchant;
        let merchant = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to find merchant: {}", e)))?
            .ok_or_else(|| AppError::ValidationError("Merchant not found".to_string()))?;

        // Query all Bitcoin wallets for this user
        let currency_pattern = format!("btc_{}", environment);
        let wallets = wallet::Entity::find()
            .filter(wallet::Column::UserId.eq(&merchant.user_id))
            .filter(wallet::Column::Currency.contains(&currency_pattern))
            .all(db)
            .await
            .map_err(|e| {
                AppError::DatabaseError(format!("Failed to query merchant wallets: {}", e))
            })?;

        log::info!(
            "💼 Retrieved {} BTC {} wallets for merchant",
            wallets.len(),
            environment
        );

        Ok(wallets)
    }

    /// Evaluate wallet candidates for transferable balance
    async fn evaluate_wallet_candidates(
        wallets: Vec<wallet::Model>,
        environment: &str,
    ) -> Result<Vec<BtcWalletCandidate>, AppError> {
        log::info!(" Evaluating {} wallet candidates for BTC...", wallets.len());

        let mut candidates = Vec::new();
        let encryption = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;

        for wallet in wallets {
            log::info!(" Evaluating wallet: {}", wallet.address);

            // Decrypt private key
            let private_key = match encryption.decrypt(&wallet.private_key) {
                Ok(decrypted) => decrypted,
                Err(_) => {
                    log::warn!(
                        " Wallet {} missing valid private key - skipping",
                        wallet.address
                    );
                    continue;
                }
            };

            // Get UTXOs and balance for this wallet
            let (balance_satoshis, utxos) =
                match Self::get_wallet_balance_and_utxos(&wallet.address, environment).await {
                    Ok(result) => result,
                    Err(e) => {
                        log::warn!(
                            " Failed to get balance/UTXOs for wallet {}: {}",
                            wallet.address,
                            e
                        );
                        continue;
                    }
                };

            if balance_satoshis == 0 {
                log::info!("⏭️ Wallet {} has 0 balance - skipping", wallet.address);
                continue;
            }

            // Estimate transaction fee (simplified - should use proper fee estimation)
            let fee_satoshis = Self::estimate_transaction_fee(utxos.len(), 1).await; // 1 output

            // Calculate transferable amount
            let transferable_satoshis = balance_satoshis.saturating_sub(fee_satoshis);

            let balance_btc = balance_satoshis as f64 / 100_000_000.0;
            let transferable_btc = transferable_satoshis as f64 / 100_000_000.0;

            log::info!(
                "💰 Wallet {}: Balance {} BTC, Transferable {} BTC",
                wallet.address,
                balance_btc,
                transferable_btc
            );

            if transferable_satoshis > 0 {
                candidates.push(BtcWalletCandidate {
                    address: wallet.address,
                    balance_satoshis,
                    fee_satoshis,
                    transferable_satoshis,
                    private_key,
                    utxos,
                });
            } else {
                log::info!(
                    "⏭️ Wallet {} has 0 transferable balance after fees - skipping",
                    wallet.address
                );
            }
        }

        // Sort by transferable amount (highest first) for optimal aggregation
        candidates.sort_by(|a, b| b.transferable_satoshis.cmp(&a.transferable_satoshis));

        log::info!(
            "✅ Found {} wallets with transferable balance",
            candidates.len()
        );

        Ok(candidates)
    }

    /// Execute aggregated transfers across selected wallets
    async fn execute_aggregated_transfers(
        candidates: Vec<BtcWalletCandidate>,
        requested_amount_satoshis: u64,
        payout_address: &str,
        environment: &str,
        _idempotency_key: &str,
    ) -> Result<BtcMultiWalletResult, AppError> {
        log::info!(
            " Executing aggregated transfers for {} satoshis",
            requested_amount_satoshis
        );

        let mut remaining_needed = requested_amount_satoshis;
        let mut wallets_used = Vec::new();
        let mut tx_hashes = Vec::new();
        let mut explorer_urls = Vec::new();
        let mut total_transferred = 0u64;
        let mut total_fees = 0u64;

        for candidate in candidates {
            if remaining_needed == 0 {
                break;
            }

            // Calculate how much to transfer from this wallet
            let transfer_amount = std::cmp::min(remaining_needed, candidate.transferable_satoshis);

            if transfer_amount == 0 {
                continue;
            }

            log::info!(
                "💸 Transferring {} satoshis from wallet {}",
                transfer_amount,
                candidate.address
            );

            // Execute transfer
            match Self::send_bitcoin_transaction(
                &candidate,
                payout_address,
                transfer_amount,
                environment,
            )
            .await
            {
                Ok(tx_hash) => {
                    log::info!(
                        "✅ Transfer successful: {} satoshis, tx: {}",
                        transfer_amount,
                        tx_hash
                    );

                    let explorer_url = Self::get_explorer_url(environment, &tx_hash);

                    wallets_used.push(BtcWalletUsage {
                        address: candidate.address,
                        transferred_satoshis: transfer_amount,
                        fee_satoshis: candidate.fee_satoshis,
                        tx_hash: tx_hash.clone(),
                        utxos_used: candidate.utxos.clone(),
                    });

                    tx_hashes.push(tx_hash);
                    explorer_urls.push(explorer_url);

                    total_transferred += transfer_amount;
                    total_fees += candidate.fee_satoshis;
                    remaining_needed -= transfer_amount;
                }
                Err(e) => {
                    log::error!(" Transfer failed from wallet {}: {}", candidate.address, e);
                    // Continue with next wallet instead of failing completely
                    continue;
                }
            }
        }

        if remaining_needed > 0 {
            return Err(AppError::InternalServerError(format!(
                "Could not transfer full amount: {} satoshis still needed after {} successful transfers",
                remaining_needed, wallets_used.len()
            )));
        }

        let withdrawal_id = uuid::Uuid::new_v4().to_string();

        Ok(BtcMultiWalletResult {
            withdrawal_id,
            total_transferred_satoshis: total_transferred,
            total_fee_satoshis: total_fees,
            wallets_used,
            tx_hashes,
            explorer_urls,
        })
    }

    /// Get wallet balance and UTXOs (placeholder - implement with Bitcoin RPC)
    async fn get_wallet_balance_and_utxos(
        _address: &str,
        _environment: &str,
    ) -> Result<(u64, Vec<BtcUtxo>), AppError> {
        // TODO: Implement Bitcoin RPC calls to get real UTXOs
        // For now, return mock data
        log::warn!(" Using mock Bitcoin balance/UTXOs - implement Bitcoin RPC integration");

        Ok((
            50000,
            vec![BtcUtxo {
                txid: "mock_txid".to_string(),
                vout: 0,
                amount_satoshis: 50000,
                confirmations: 6,
            }],
        ))
    }

    /// Estimate transaction fee based on inputs and outputs
    async fn estimate_transaction_fee(input_count: usize, output_count: usize) -> u64 {
        // Simplified fee estimation: ~148 bytes per input + ~34 bytes per output + ~10 bytes overhead
        let tx_size = (input_count * 148) + (output_count * 34) + 10;
        let fee_rate_sat_per_byte = 10; // TODO: Get dynamic fee rate

        (tx_size * fee_rate_sat_per_byte) as u64
    }

    /// Send Bitcoin transaction (placeholder - implement with Bitcoin libraries)
    async fn send_bitcoin_transaction(
        _candidate: &BtcWalletCandidate,
        _to_address: &str,
        _amount_satoshis: u64,
        _environment: &str,
    ) -> Result<String, AppError> {
        // TODO: Implement Bitcoin transaction creation and broadcasting
        // This would involve:
        // 1. Creating transaction with UTXOs as inputs
        // 2. Adding output to recipient
        // 3. Adding change output if needed
        // 4. Signing with private key
        // 5. Broadcasting to Bitcoin network

        log::warn!(" Using mock Bitcoin transaction - implement Bitcoin transaction creation");

        // Return mock transaction hash
        Ok("mock_btc_tx_hash_".to_string() + &uuid::Uuid::new_v4().to_string()[0..8])
    }

    /// Get explorer URL for transaction
    fn get_explorer_url(environment: &str, tx_hash: &str) -> String {
        match environment {
            "mainnet" => format!("https://blockstream.info/tx/{}", tx_hash),
            "testnet" => format!("https://blockstream.info/testnet/tx/{}", tx_hash),
            _ => format!("https://blockstream.info/tx/{}", tx_hash),
        }
    }
}
