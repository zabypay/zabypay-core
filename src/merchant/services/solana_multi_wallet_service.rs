use crate::merchant::models::withdrawal::CreateWithdrawalRequest;
use crate::shared::{
    entities::{payment_request, wallet},
    service::solana_wallet::{
        estimate_transfer_fee, get_rent_exemption_amount, get_sol_balance,
        get_solana_keypair_from_mnemonic, transfer_sol_with_confirmation,
    },
    utils::{encryption::CryptoEncryption, errors::AppError},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Signer;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaWalletCandidate {
    pub address: String,
    pub balance_lamports: u64,
    pub rent_exempt_minimum: u64,
    pub estimated_fee: u64,
    pub transferable_lamports: u64,
    pub mnemonic: String,
    pub private_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaMultiWalletResult {
    pub withdrawal_id: String,
    pub total_transferred_lamports: u64,
    pub total_fee_lamports: u64,
    pub wallets_used: Vec<SolanaWalletUsage>,
    pub tx_hashes: Vec<String>,
    pub explorer_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaWalletUsage {
    pub address: String,
    pub transferred_lamports: u64,
    pub fee_lamports: u64,
    pub tx_hash: String,
}

pub struct SolanaMultiWalletService;

impl SolanaMultiWalletService {
    pub async fn execute_multi_wallet_withdrawal(
        db: &DatabaseConnection,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
        payout_address: &str,
    ) -> Result<SolanaMultiWalletResult, AppError> {
        log::info!(
            "🔄 Starting Solana multi-wallet withdrawal for merchant {}",
            merchant_id
        );

        let requested_amount_sol: f64 = request
            .amount
            .parse()
            .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;
        let requested_amount_lamports = (requested_amount_sol * 1_000_000_000.0) as u64;

        log::info!(
            "💰 Requested amount: {} SOL ({} lamports)",
            requested_amount_sol,
            requested_amount_lamports
        );

        let mut deposit_wallets = Self::discover_deposit_wallets(db, merchant_id).await?;

        if deposit_wallets.is_empty() {
            log::info!(
                "🔄 No deposit wallets found from payments, checking regular merchant wallets..."
            );
            deposit_wallets = Self::discover_merchant_wallets(db, merchant_id).await?;
        }

        if deposit_wallets.is_empty() {
            return Err(AppError::ValidationError(
                "No SOL wallets found for this merchant. Create some wallets first.".to_string(),
            ));
        }

        log::info!(" Found {} potential deposit wallets", deposit_wallets.len());

        let candidates = Self::evaluate_wallet_candidates(deposit_wallets).await?;

        if candidates.is_empty() {
            return Err(AppError::ValidationError(
                "No wallets have transferable balance after rent exemption and fees".to_string(),
            ));
        }

        let total_transferable: u64 = candidates.iter().map(|c| c.transferable_lamports).sum();
        let total_transferable_sol = total_transferable as f64 / 1_000_000_000.0;

        log::info!(
            "💸 Total transferable across {} wallets: {} SOL ({} lamports)",
            candidates.len(),
            total_transferable_sol,
            total_transferable
        );

        if total_transferable < requested_amount_lamports {
            let shortage = requested_amount_lamports - total_transferable;
            let shortage_sol = shortage as f64 / 1_000_000_000.0;

            return Err(AppError::ValidationError(format!(
                "Insufficient balance across all wallets: Requested {} SOL, available {} SOL (shortage: {} SOL)",
                requested_amount_sol, total_transferable_sol, shortage_sol
            )));
        }

        let result = Self::execute_aggregated_transfers(
            candidates,
            requested_amount_lamports,
            payout_address,
            &request.idempotency_key,
        )
        .await?;

        log::info!(
            "✅ Multi-wallet withdrawal completed: {} lamports from {} wallets",
            result.total_transferred_lamports,
            result.wallets_used.len()
        );

        Ok(result)
    }

    async fn discover_deposit_wallets(
        db: &DatabaseConnection,
        merchant_id: &str,
    ) -> Result<Vec<wallet::Model>, AppError> {
        log::info!(" Discovering deposit wallets from paid SOL payments...");

        let paid_payments = payment_request::Entity::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Environment.eq("mainnet"))
            .filter(payment_request::Column::Currency.eq("SOL"))
            .filter(payment_request::Column::Status.eq("paid"))
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query payments: {}", e)))?;

        log::info!(" Found {} paid SOL payments", paid_payments.len());

        let mut wallet_addresses: Vec<String> = paid_payments
            .into_iter()
            .map(|p| p.wallet_address)
            .collect();

        wallet_addresses.sort();
        wallet_addresses.dedup();

        log::info!(
            "🔑 Found {} unique deposit wallet addresses",
            wallet_addresses.len()
        );

        let wallets = wallet::Entity::find()
            .filter(wallet::Column::Address.is_in(wallet_addresses))
            .filter(wallet::Column::Currency.contains("sol_mainnet"))
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query wallets: {}", e)))?;

        log::info!("💼 Retrieved {} wallets with credentials", wallets.len());

        Ok(wallets)
    }

    async fn discover_merchant_wallets(
        db: &DatabaseConnection,
        merchant_id: &str,
    ) -> Result<Vec<wallet::Model>, AppError> {
        log::info!(" Discovering regular merchant wallets for SOL mainnet...");

        use crate::shared::entities::merchant;
        let merchant = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to find merchant: {}", e)))?
            .ok_or_else(|| AppError::ValidationError("Merchant not found".to_string()))?;

        let wallets = wallet::Entity::find()
            .filter(wallet::Column::UserId.eq(&merchant.user_id))
            .filter(wallet::Column::Currency.contains("sol_mainnet"))
            .all(db)
            .await
            .map_err(|e| {
                AppError::DatabaseError(format!("Failed to query merchant wallets: {}", e))
            })?;

        log::info!(
            "💼 Retrieved {} SOL mainnet wallets for merchant",
            wallets.len()
        );

        Ok(wallets)
    }

    async fn evaluate_wallet_candidates(
        wallets: Vec<wallet::Model>,
    ) -> Result<Vec<SolanaWalletCandidate>, AppError> {
        log::info!(" Evaluating {} wallet candidates...", wallets.len());

        let mut candidates = Vec::new();
        let encryption = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;
        let mainnet_rpc = "https://api.mainnet-beta.solana.com";

        let rent_exempt_minimum = get_rent_exemption_amount(Some(mainnet_rpc))
            .await
            .unwrap_or(890880);

        for wallet in wallets {
            log::info!(" Evaluating wallet: {}", wallet.address);

            let decrypted_mnemonic = match encryption.decrypt(&wallet.mnemonic) {
                Ok(decrypted) => Some(decrypted),
                Err(_) => None,
            };

            let decrypted_private_key = match encryption.decrypt(&wallet.private_key) {
                Ok(decrypted) => Some(decrypted),
                Err(_) => None,
            };

            if decrypted_mnemonic.is_none() || decrypted_private_key.is_none() {
                log::warn!(" Wallet {} missing credentials - skipping", wallet.address);
                continue;
            }

            let mnemonic = decrypted_mnemonic.unwrap();
            let private_key = decrypted_private_key.unwrap();

            let balance_lamports = match get_sol_balance(&wallet.address, Some(mainnet_rpc)).await {
                Ok(balance) => balance,
                Err(e) => {
                    log::warn!(
                        " Failed to get balance for wallet {}: {}",
                        wallet.address,
                        e
                    );
                    continue;
                }
            };

            let keypair = get_solana_keypair_from_mnemonic(&mnemonic, 0);
            let estimated_fee = match estimate_transfer_fee(
                &keypair.pubkey(),
                "11111111111111111111111111111112",
                100000,
                Some(mainnet_rpc),
            )
            .await
            {
                Ok(fee) => fee,
                Err(_) => 5000_u64,
            };

            let transferable_lamports = balance_lamports
                .saturating_sub(rent_exempt_minimum)
                .saturating_sub(estimated_fee);

            let balance_sol = balance_lamports as f64 / 1_000_000_000.0;
            let transferable_sol = transferable_lamports as f64 / 1_000_000_000.0;

            log::info!(
                "💰 Wallet {}: Balance {} SOL, Transferable {} SOL",
                wallet.address,
                balance_sol,
                transferable_sol
            );

            if transferable_lamports > 0 {
                candidates.push(SolanaWalletCandidate {
                    address: wallet.address,
                    balance_lamports,
                    rent_exempt_minimum,
                    estimated_fee,
                    transferable_lamports,
                    mnemonic,
                    private_key,
                });
            } else {
                log::info!(
                    "⏭️ Wallet {} has 0 transferable balance - skipping",
                    wallet.address
                );
            }
        }

        candidates.sort_by(|a, b| b.transferable_lamports.cmp(&a.transferable_lamports));

        log::info!(
            "✅ Found {} wallets with transferable balance",
            candidates.len()
        );

        Ok(candidates)
    }

    async fn execute_aggregated_transfers(
        candidates: Vec<SolanaWalletCandidate>,
        requested_amount_lamports: u64,
        payout_address: &str,
        idempotency_key: &str,
    ) -> Result<SolanaMultiWalletResult, AppError> {
        log::info!(
            " Executing aggregated transfers for {} lamports",
            requested_amount_lamports
        );

        let mut remaining_needed = requested_amount_lamports;
        let mut wallets_used = Vec::new();
        let mut tx_hashes = Vec::new();
        let mut explorer_urls = Vec::new();
        let mut total_transferred = 0u64;
        let mut total_fees = 0u64;
        let mainnet_rpc = "https://api.mainnet-beta.solana.com";

        for candidate in candidates {
            if remaining_needed == 0 {
                break;
            }

            let transfer_amount = std::cmp::min(remaining_needed, candidate.transferable_lamports);

            if transfer_amount == 0 {
                continue;
            }

            log::info!(
                "💸 Transferring {} lamports from wallet {}",
                transfer_amount,
                candidate.address
            );

            let keypair = get_solana_keypair_from_mnemonic(&candidate.mnemonic, 0);

            if keypair.pubkey().to_string() != candidate.address {
                log::error!(
                    " Address mismatch for wallet {} - skipping",
                    candidate.address
                );
                continue;
            }

            match transfer_sol_with_confirmation(
                &keypair,
                payout_address,
                transfer_amount,
                Some(mainnet_rpc),
            )
            .await
            {
                Ok(tx_hash) => {
                    log::info!(
                        "✅ Transfer successful: {} lamports, tx: {}",
                        transfer_amount,
                        tx_hash
                    );

                    let explorer_url = format!(
                        "https://explorer.solana.com/tx/{}?cluster=mainnet-beta",
                        tx_hash
                    );

                    wallets_used.push(SolanaWalletUsage {
                        address: candidate.address,
                        transferred_lamports: transfer_amount,
                        fee_lamports: candidate.estimated_fee,
                        tx_hash: tx_hash.clone(),
                    });

                    tx_hashes.push(tx_hash);
                    explorer_urls.push(explorer_url);

                    total_transferred += transfer_amount;
                    total_fees += candidate.estimated_fee;
                    remaining_needed -= transfer_amount;
                }
                Err(e) => {
                    log::error!(" Transfer failed from wallet {}: {}", candidate.address, e);
                    continue;
                }
            }
        }

        if remaining_needed > 0 {
            return Err(AppError::InternalServerError(format!(
                "Could not transfer full amount: {} lamports still needed after {} successful transfers",
                remaining_needed, wallets_used.len()
            )));
        }

        let withdrawal_id = uuid::Uuid::new_v4().to_string();

        Ok(SolanaMultiWalletResult {
            withdrawal_id,
            total_transferred_lamports: total_transferred,
            total_fee_lamports: total_fees,
            wallets_used,
            tx_hashes,
            explorer_urls,
        })
    }
}
