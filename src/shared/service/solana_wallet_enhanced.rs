use crate::shared::entities::{prelude::*, system_config};
use crate::shared::utils::errors::AppError;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;

/// Enhanced Solana wallet functions with dynamic configuration
pub struct SolanaWalletEnhanced {
    db: DatabaseConnection,
}

impl SolanaWalletEnhanced {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Get RPC URL from database configuration
    async fn get_rpc_url(&self, environment: &str) -> Result<String, AppError> {
        let config_key = format!("solana_rpc_url_{}", environment);

        if let Ok(config) = SystemConfig::find()
            .filter(system_config::Column::Key.eq(&config_key))
            .one(&self.db)
            .await
        {
            if let Some(config) = config {
                if let Some(url) = config.value.as_str() {
                    return Ok(url.to_string());
                }
            }
        }

        // Fallback to default mainnet URL
        let fallback_url = match environment {
            "mainnet" => "https://api.mainnet-beta.solana.com",
            "testnet" => "https://api.testnet.solana.com",
            "devnet" => "https://api.devnet.solana.com",
            _ => "https://api.mainnet-beta.solana.com", // Default to mainnet for production
        };

        Ok(fallback_url.to_string())
    }

    /// Get explorer base URL based on environment
    async fn get_explorer_url(&self, environment: &str) -> String {
        match environment {
            "mainnet" => "https://explorer.solana.com",
            "testnet" => "https://explorer.solana.com?cluster=testnet",
            "devnet" => "https://explorer.solana.com?cluster=devnet",
            _ => "https://explorer.solana.com", // Default to mainnet
        }
        .to_string()
    }

    /// Estimate transfer fee dynamically
    pub async fn estimate_transfer_fee(
        &self,
        sender_pubkey: &Pubkey,
        recipient_address: &str,
        amount_lamports: u64,
        environment: &str,
    ) -> Result<u64, AppError> {
        let rpc_url = self.get_rpc_url(environment).await?;
        let client = RpcClient::new(rpc_url);

        let recipient_pubkey = Pubkey::from_str(recipient_address)
            .map_err(|e| AppError::ValidationError(format!("Invalid recipient address: {}", e)))?;

        // Create transfer instruction
        let transfer_instruction =
            system_instruction::transfer(sender_pubkey, &recipient_pubkey, amount_lamports);

        // Create transaction for fee estimation
        let mut transaction =
            Transaction::new_with_payer(&[transfer_instruction], Some(sender_pubkey));

        // Set a recent blockhash for fee calculation
        let recent_blockhash = client.get_latest_blockhash().map_err(|e| {
            AppError::InternalServerError(format!("Failed to get blockhash: {}", e))
        })?;
        transaction.message.recent_blockhash = recent_blockhash;

        // Get fee for this transaction
        let fee = client
            .get_fee_for_message(&transaction.message)
            .map_err(|e| AppError::InternalServerError(format!("Failed to estimate fee: {}", e)))?;

        Ok(fee)
    }

    /// Transfer SOL with dynamic configuration and confirmation
    pub async fn transfer_sol_with_confirmation(
        &self,
        sender_keypair: &Keypair,
        recipient_address: &str,
        amount_lamports: u64,
        environment: &str,
    ) -> Result<String, AppError> {
        let rpc_url = self.get_rpc_url(environment).await?;
        let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());

        // Get current balance
        let sender_balance = client
            .get_balance(&sender_keypair.pubkey())
            .map_err(|e| AppError::InternalServerError(format!("Failed to get balance: {}", e)))?;
        let sender_sol = sender_balance as f64 / 1_000_000_000.0;

        // Estimate transaction fee dynamically
        let estimated_fee = self
            .estimate_transfer_fee(
                &sender_keypair.pubkey(),
                recipient_address,
                amount_lamports,
                environment,
            )
            .await?;

        let fee_sol = estimated_fee as f64 / 1_000_000_000.0;
        let amount_sol = amount_lamports as f64 / 1_000_000_000.0;
        let total_cost_sol = amount_sol + fee_sol;

        log::info!(" Transaction Summary:");
        log::info!("   Environment: {}", environment);
        log::info!("   RPC URL: {}", rpc_url);
        log::info!(
            "   Current Balance: {} SOL ({} lamports)",
            sender_sol,
            sender_balance
        );
        log::info!(
            "   Transfer Amount: {} SOL ({} lamports)",
            amount_sol,
            amount_lamports
        );
        log::info!(
            "   Estimated Fee: {} SOL ({} lamports)",
            fee_sol,
            estimated_fee
        );
        log::info!(
            "   Total Cost: {} SOL ({} lamports)",
            total_cost_sol,
            amount_lamports + estimated_fee
        );
        log::info!("   Remaining Balance: {} SOL", sender_sol - total_cost_sol);

        // Check if sender has enough balance
        if sender_balance < amount_lamports + estimated_fee {
            return Err(AppError::InsufficientBalance(format!(
                "Insufficient funds! Need {} SOL but only have {} SOL",
                total_cost_sol, sender_sol
            )));
        }

        // Parse recipient address
        let recipient_pubkey = Pubkey::from_str(recipient_address)
            .map_err(|e| AppError::ValidationError(format!("Invalid recipient address: {}", e)))?;

        // Get recent blockhash
        let recent_blockhash = client.get_latest_blockhash().map_err(|e| {
            AppError::InternalServerError(format!("Failed to get blockhash: {}", e))
        })?;

        // Create transfer instruction
        let transfer_instruction = system_instruction::transfer(
            &sender_keypair.pubkey(),
            &recipient_pubkey,
            amount_lamports,
        );

        // Create and sign transaction
        let transaction = Transaction::new_signed_with_payer(
            &[transfer_instruction],
            Some(&sender_keypair.pubkey()),
            &[sender_keypair],
            recent_blockhash,
        );

        // Send transaction
        match client.send_and_confirm_transaction(&transaction) {
            Ok(signature) => {
                let explorer_url = self.get_explorer_url(environment).await;
                let cluster_param = if environment == "mainnet" {
                    ""
                } else {
                    &format!("&cluster={}", environment)
                };

                log::info!("✅ Transaction successful!");
                log::info!("🔗 Transaction signature: {}", signature);
                log::info!(
                    " View on Solana Explorer: {}/tx/{}{}",
                    explorer_url,
                    signature,
                    cluster_param
                );
                Ok(signature.to_string())
            }
            Err(e) => {
                log::error!(" Transaction failed: {}", e);
                Err(AppError::TransactionFailed(format!(
                    "Transaction failed: {}",
                    e
                )))
            }
        }
    }

    /// Simple transfer wrapper
    pub async fn transfer_sol(
        &self,
        sender_keypair: &Keypair,
        recipient_address: &str,
        amount_lamports: u64,
        environment: &str,
    ) -> Result<String, AppError> {
        self.transfer_sol_with_confirmation(
            sender_keypair,
            recipient_address,
            amount_lamports,
            environment,
        )
        .await
    }

    /// Get SOL balance with dynamic RPC
    pub async fn get_sol_balance(&self, address: &str, environment: &str) -> Result<f64, AppError> {
        let rpc_url = self.get_rpc_url(environment).await?;
        let client = RpcClient::new(rpc_url.clone());

        let pubkey = Pubkey::from_str(address)
            .map_err(|e| AppError::ValidationError(format!("Invalid address: {}", e)))?;
        let balance = client
            .get_balance(&pubkey)
            .map_err(|e| AppError::InternalServerError(format!("Failed to get balance: {}", e)))?;

        // Convert lamports to SOL (1 SOL = 1,000,000,000 lamports)
        let sol_balance = balance as f64 / 1_000_000_000.0;
        log::info!(
            "💰 Balance for {} ({}): {} SOL ({} lamports)",
            address,
            environment,
            sol_balance,
            balance
        );

        Ok(sol_balance)
    }

    /// Check if address is valid for the specified environment
    pub async fn validate_address_for_environment(
        &self,
        address: &str,
        _environment: &str,
    ) -> Result<bool, AppError> {
        // Basic Solana address validation
        match Pubkey::from_str(address) {
            Ok(_) => {
                // Additional environment-specific validations could be added here
                // For now, any valid Solana address works across all environments
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}
