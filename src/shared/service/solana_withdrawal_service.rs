use bs58;
use chrono::Utc;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use solana_sdk::signature::{Keypair, Signer};

use crate::shared::{
    entities::{prelude::*, wallet, withdrawal_request},
    service::solana_wallet::{
        estimate_transfer_fee, get_sol_balance, get_solana_keypair_from_mnemonic,
        transfer_sol_with_confirmation,
    },
    utils::{encryption::CryptoEncryption, errors::AppError},
};

/// Enhanced Solana withdrawal service with DB-driven configuration
pub struct SolanaWithdrawalService {
    pub db: sea_orm::DatabaseConnection,
}

#[derive(Debug)]
pub struct WithdrawalResult {
    pub transaction_hash: String,
    pub explorer_url: String,
    pub fee_lamports: u64,
    pub net_amount_lamports: u64,
}

impl SolanaWithdrawalService {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }

    /// Get RPC URL based on environment from config/DB
    fn get_rpc_url(&self, environment: &str) -> String {
        match environment {
            "mainnet" => "https://api.mainnet-beta.solana.com".to_string(),
            "testnet" => "https://api.testnet.solana.com".to_string(),
            "devnet" => "https://api.devnet.solana.com".to_string(),
            _ => "https://api.mainnet-beta.solana.com".to_string(), // Default to mainnet
        }
    }

    /// Get explorer base URL based on environment
    fn get_explorer_url(&self, environment: &str, signature: &str) -> String {
        let cluster_param = match environment {
            "mainnet" => "",
            "testnet" => "?cluster=testnet",
            "devnet" => "?cluster=devnet",
            _ => "",
        };
        format!(
            "https://explorer.solana.com/tx/{}{}",
            signature, cluster_param
        )
    }

    /// Get required confirmations based on environment
    fn get_required_confirmations(&self, environment: &str) -> i32 {
        match environment {
            "mainnet" => 32, // Mainnet requires more confirmations
            "testnet" => 10,
            "devnet" => 5,
            _ => 32,
        }
    }

    /// Get keypair from wallet using stored mnemonic/private_key
    async fn get_keypair_from_wallet(&self, wallet_id: &str) -> Result<Keypair, AppError> {
        log::info!("Getting keypair for wallet: {}", wallet_id);

        // Get wallet from database
        let wallet = Wallet::find()
            .filter(wallet::Column::Id.eq(wallet_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch wallet: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Wallet not found".to_string()))?;

        // Check if wallet has required data
        if wallet.mnemonic.is_empty() && wallet.private_key.is_empty() {
            return Err(AppError::ValidationError(
                "Wallet has no private key or mnemonic stored. Cannot derive keypair.".to_string(),
            ));
        }

        // Initialize encryption service
        let crypto = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;

        // Try to decrypt mnemonic first, then fall back to private key
        if !wallet.mnemonic.is_empty() {
            log::info!("Deriving keypair from encrypted mnemonic");
            let decrypted_mnemonic = crypto.decrypt_mnemonic(&wallet.mnemonic).map_err(|e| {
                AppError::InternalServerError(format!("Failed to decrypt mnemonic: {}", e))
            })?;

            // Use derivation index 0 (could be stored in DB if needed)
            let keypair = get_solana_keypair_from_mnemonic(&decrypted_mnemonic, 0);

            // Verify the derived address matches the stored address
            let derived_address = keypair.pubkey().to_string();
            if derived_address != wallet.address {
                log::warn!(
                    "Derived address {} doesn't match stored address {}",
                    derived_address,
                    wallet.address
                );
                // Continue anyway - the stored address might be outdated
            }

            Ok(keypair)
        } else if !wallet.private_key.is_empty() {
            log::info!("Creating keypair from encrypted private key");
            let decrypted_private_key =
                crypto
                    .decrypt_private_key(&wallet.private_key)
                    .map_err(|e| {
                        AppError::InternalServerError(format!(
                            "Failed to decrypt private key: {}",
                            e
                        ))
                    })?;

            // Decode base58 private key to bytes
            let private_key_bytes =
                bs58::decode(&decrypted_private_key)
                    .into_vec()
                    .map_err(|e| {
                        AppError::ValidationError(format!("Invalid private key format: {}", e))
                    })?;

            if private_key_bytes.len() != 64 {
                return Err(AppError::ValidationError(
                    "Private key must be 64 bytes".to_string(),
                ));
            }

            let keypair = Keypair::try_from(&private_key_bytes[..])
                .map_err(|e| AppError::ValidationError(format!("Invalid keypair bytes: {}", e)))?;

            Ok(keypair)
        } else {
            Err(AppError::ValidationError(
                "No valid key material found in wallet".to_string(),
            ))
        }
    }

    /// Execute Solana withdrawal with full DB-driven configuration
    pub async fn execute_withdrawal(
        &self,
        withdrawal_id: &str,
        _merchant_id: &str,
        to_address: &str,
        amount: Decimal,
        environment: &str,
    ) -> Result<WithdrawalResult, AppError> {
        log::info!(
            " Starting DB-driven Solana withdrawal: {} SOL to {} on {}",
            amount,
            to_address,
            environment
        );

        // Get withdrawal record to find the wallet
        let withdrawal = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::Id.eq(withdrawal_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch withdrawal: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

        // Get wallet from withdrawal
        let wallet_id = &withdrawal.wallet_id;
        log::info!("Using wallet {} for withdrawal", wallet_id);

        // Get keypair from wallet (with proper error handling)
        let keypair = self.get_keypair_from_wallet(wallet_id).await?;
        let sender_address = keypair.pubkey().to_string();
        log::info!("Sender wallet address: {}", sender_address);

        // Get RPC URL for this environment
        let rpc_url = self.get_rpc_url(environment);
        log::info!("Using RPC URL: {}", rpc_url);

        // Convert amount from Decimal to lamports
        let amount_lamports = (amount * Decimal::new(1_000_000_000, 0))
            .to_u64()
            .ok_or_else(|| AppError::ValidationError("Invalid amount conversion".to_string()))?;

        log::info!(
            "Transfer amount: {} lamports ({} SOL)",
            amount_lamports,
            amount
        );

        // Get current on-chain balance
        let sender_pubkey = keypair.pubkey();
        let on_chain_balance = get_sol_balance(&sender_address, Some(&rpc_url)).await?;
        log::info!(
            "On-chain balance: {} lamports ({} SOL)",
            on_chain_balance,
            on_chain_balance as f64 / 1_000_000_000.0
        );

        // Estimate transaction fee
        let estimated_fee =
            estimate_transfer_fee(&sender_pubkey, to_address, amount_lamports, Some(&rpc_url))
                .await?;
        log::info!(
            "Estimated fee: {} lamports ({} SOL)",
            estimated_fee,
            estimated_fee as f64 / 1_000_000_000.0
        );

        // Validate sufficient balance
        let total_needed = amount_lamports + estimated_fee;
        if on_chain_balance < total_needed {
            return Err(AppError::ValidationError(format!(
                "Insufficient balance: Need {} lamports (including {} fee) but only have {} lamports",
                total_needed, estimated_fee, on_chain_balance
            )));
        }

        // Execute the transfer
        log::info!(" Broadcasting Solana transaction...");
        let transaction_signature =
            transfer_sol_with_confirmation(&keypair, to_address, amount_lamports, Some(&rpc_url))
                .await?;

        log::info!(
            "✅ Transaction broadcast successful: {}",
            transaction_signature
        );

        // Generate explorer URL
        let explorer_url = self.get_explorer_url(environment, &transaction_signature);

        // Calculate net amount (amount - fee, but fee is already accounted for in the transfer)
        let net_amount_lamports = amount_lamports; // The amount actually transferred

        // Update withdrawal record with transaction details
        // For Solana, since we use send_and_confirm_transaction, the transaction is already confirmed
        log::info!("📝 Updating withdrawal record with transaction details");
        let mut withdrawal_active: withdrawal_request::ActiveModel = withdrawal.into();
        withdrawal_active.tx_hash = Set(Some(transaction_signature.clone()));
        withdrawal_active.status = Set("confirmed".to_string()); // Immediately confirmed for Solana
        withdrawal_active.broadcast_at = Set(Some(Utc::now().into()));
        withdrawal_active.confirmed_at = Set(Some(Utc::now().into())); // Set confirmed time
        withdrawal_active.blockchain_confirmations = Set(Some(1)); // Solana confirms quickly
        withdrawal_active.required_confirmations =
            Set(self.get_required_confirmations(environment));
        withdrawal_active.updated_at = Set(Utc::now().into());

        withdrawal_active
            .update(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to update withdrawal: {}", e)))?;

        log::info!("🎉 Solana withdrawal completed successfully!");
        log::info!("   Transaction: {}", transaction_signature);
        log::info!("   Explorer: {}", explorer_url);
        log::info!("   Fee: {} lamports", estimated_fee);
        log::info!("   Net Amount: {} lamports", net_amount_lamports);

        Ok(WithdrawalResult {
            transaction_hash: transaction_signature,
            explorer_url,
            fee_lamports: estimated_fee,
            net_amount_lamports,
        })
    }
}
