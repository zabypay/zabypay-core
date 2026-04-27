use crate::shared::utils::errors::AppError;
use rust_decimal::Decimal;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig, message::Message, pubkey::Pubkey, signature::Keypair,
    system_instruction, transaction::Transaction,
};
use std::str::FromStr;

pub struct FeeEstimator;

#[derive(Debug, Clone)]
pub struct FeeEstimate {
    pub base_fee: u64,        // Base network fee in lamports
    pub priority_fee: u64,    // Priority fee in lamports
    pub total_fee: u64,       // Total fee in lamports
    pub fee_decimal: Decimal, // Total fee as decimal (SOL)
}

impl FeeEstimator {
    /// Get dynamic Solana RPC URL from config
    pub fn get_solana_rpc_url(environment: &str) -> Result<String, AppError> {
        match environment {
            "mainnet" => {
                // In production, these should come from environment variables or config
                let mainnet_urls = vec![
                    "https://api.mainnet-beta.solana.com".to_string(),
                    "https://solana-api.projectserum.com".to_string(),
                ];
                // For now, use the first one. In production, implement RPC rotation/failover
                Ok(mainnet_urls[0].clone())
            }
            "testnet" => Ok("https://api.testnet.solana.com".to_string()),
            "devnet" => Ok("https://api.devnet.solana.com".to_string()),
            _ => Err(AppError::ValidationError(format!(
                "Unsupported Solana environment: {}",
                environment
            ))),
        }
    }

    /// Estimate Solana transaction fee using get_fee_for_message
    pub async fn estimate_solana_fee(
        from_pubkey: &Pubkey,
        to_pubkey: &Pubkey,
        amount_lamports: u64,
        environment: &str,
    ) -> Result<FeeEstimate, AppError> {
        let rpc_url = Self::get_solana_rpc_url(environment)?;

        // Create RPC client with blocking call wrapped in spawn_blocking
        let rpc_url_clone = rpc_url.clone();
        let from_pubkey_clone = *from_pubkey;
        let to_pubkey_clone = *to_pubkey;

        let fee_estimate = tokio::task::spawn_blocking(move || {
            let client =
                RpcClient::new_with_commitment(rpc_url_clone, CommitmentConfig::confirmed());

            // Create a mock transfer instruction to estimate fee
            let instruction =
                system_instruction::transfer(&from_pubkey_clone, &to_pubkey_clone, amount_lamports);

            // Create message
            let message = Message::new(&[instruction], Some(&from_pubkey_clone));

            // Get recent blockhash
            let recent_blockhash = client.get_latest_blockhash().map_err(|e| {
                AppError::ExternalServiceError(format!("Failed to get blockhash: {}", e))
            })?;

            // Update message with blockhash
            let mut message_with_blockhash = message.clone();
            message_with_blockhash.recent_blockhash = recent_blockhash;

            // Get fee for message
            let base_fee = client
                .get_fee_for_message(&message_with_blockhash)
                .map_err(|e| AppError::ExternalServiceError(format!("Failed to get fee: {}", e)))?;

            // For now, set priority fee to 0. In production, calculate based on network congestion
            let priority_fee = 0u64;
            let total_fee = base_fee + priority_fee;

            // Convert to SOL (1 SOL = 1,000,000,000 lamports)
            let fee_decimal = Decimal::from(total_fee) / Decimal::from(1_000_000_000u64);

            Ok::<FeeEstimate, AppError>(FeeEstimate {
                base_fee,
                priority_fee,
                total_fee,
                fee_decimal,
            })
        })
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to spawn blocking task: {}", e))
        })??;

        Ok(fee_estimate)
    }

    /// Estimate fee for other networks (placeholder - implement actual logic)
    pub async fn estimate_network_fee(
        network: &str,
        environment: &str,
        amount: Decimal,
    ) -> Result<Decimal, AppError> {
        match network {
            "sol" => {
                // For SOL, we need actual addresses to get accurate fees
                // This is a fallback estimate
                match environment {
                    "mainnet" => Ok(Decimal::from_str_exact("0.000005").unwrap()), // 5000 lamports
                    "testnet" | "devnet" => Ok(Decimal::from_str_exact("0.000005").unwrap()),
                    _ => Err(AppError::ValidationError(format!(
                        "Unsupported environment: {}",
                        environment
                    ))),
                }
            }
            "eth" => {
                match environment {
                    "mainnet" => Ok(Decimal::from_str_exact("0.002").unwrap()), // ~$5 at current gas prices
                    "testnet" => Ok(Decimal::from_str_exact("0.001").unwrap()),
                    _ => Err(AppError::ValidationError(format!(
                        "Unsupported environment: {}",
                        environment
                    ))),
                }
            }
            "btc" => {
                match environment {
                    "mainnet" => Ok(Decimal::from_str_exact("0.0001").unwrap()), // ~10,000 sats
                    "testnet" => Ok(Decimal::from_str_exact("0.00001").unwrap()),
                    _ => Err(AppError::ValidationError(format!(
                        "Unsupported environment: {}",
                        environment
                    ))),
                }
            }
            "bnb" => match environment {
                "mainnet" => Ok(Decimal::from_str_exact("0.001").unwrap()),
                "testnet" => Ok(Decimal::from_str_exact("0.0005").unwrap()),
                _ => Err(AppError::ValidationError(format!(
                    "Unsupported environment: {}",
                    environment
                ))),
            },
            _ => Err(AppError::ValidationError(format!(
                "Unsupported network for fee estimation: {}",
                network
            ))),
        }
    }

    /// Calculate required confirmations based on network and environment
    pub fn get_required_confirmations(network: &str, environment: &str) -> u32 {
        match environment {
            "testnet" | "devnet" => 1, // Fast confirmations for testnets
            "mainnet" => match network {
                "eth" => 12, // ~3 minutes
                "btc" => 6,  // ~60 minutes
                "sol" => 32, // ~15 seconds (recommended for finality)
                "bnb" => 20, // ~1 minute
                _ => 6,      // Default
            },
            _ => 6, // Default fallback
        }
    }

    /// Get estimated confirmation time in seconds
    pub fn get_estimated_confirmation_time(network: &str, environment: &str) -> u32 {
        let confirmations = Self::get_required_confirmations(network, environment);

        let block_time_seconds = match network {
            "eth" => 15,  // ~15 seconds per block
            "btc" => 600, // ~10 minutes per block
            "sol" => 1,   // ~400ms per slot, but we use higher confirmation count
            "bnb" => 3,   // ~3 seconds per block
            _ => 30,      // Default
        };

        confirmations * block_time_seconds
    }
}
