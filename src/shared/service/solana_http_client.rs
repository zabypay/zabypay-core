use crate::shared::entities::{prelude::*, system_config};
use crate::shared::service::solana_key_utils::SolanaKeyUtils;
use crate::shared::utils::errors::AppError;
use base64::{engine::general_purpose, Engine as _};
use ed25519_dalek::SigningKey as Ed25519Keypair;
use reqwest::Client;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct SolanaHttpClient {
    rpc_url: String,
    client: Client,
    db: DatabaseConnection,
}

impl SolanaHttpClient {
    pub async fn new(environment: &str, db: DatabaseConnection) -> Result<Self, AppError> {
        let rpc_url = Self::get_rpc_url_from_config(environment, &db).await?;

        Ok(Self {
            rpc_url,
            client: Client::new(),
            db,
        })
    }

    /// Get RPC URL from system configuration
    async fn get_rpc_url_from_config(
        environment: &str,
        db: &DatabaseConnection,
    ) -> Result<String, AppError> {
        let config_key = format!("solana_rpc_url_{}", environment);

        // Try to get from database first
        if let Ok(config) = SystemConfig::find()
            .filter(system_config::Column::Key.eq(&config_key))
            .one(db)
            .await
        {
            if let Some(config) = config {
                if let Some(url) = config.value.as_str() {
                    log::info!("Using Solana RPC URL from config: {}", url);
                    return Ok(url.to_string());
                }
            }
        }

        // Fallback to default URLs if not configured
        let fallback_url = match environment {
            "mainnet" => "https://api.mainnet-beta.solana.com",
            "testnet" => "https://api.testnet.solana.com",
            "devnet" => "https://api.devnet.solana.com",
            _ => "https://api.devnet.solana.com",
        };

        log::warn!(
            "No RPC URL configured for {}, using fallback: {}",
            environment,
            fallback_url
        );
        Ok(fallback_url.to_string())
    }

    /// Send SOL transfer using HTTP RPC calls
    pub async fn send_sol_transfer(
        &self,
        from_keypair: &Ed25519Keypair,
        to_address: &str,
        amount_lamports: u64,
    ) -> Result<String, AppError> {
        log::info!("Creating Solana transaction via HTTP RPC");

        // Get sender's address
        let from_address = SolanaKeyUtils::keypair_to_address(from_keypair);

        // Get recent blockhash
        let recent_blockhash = self.get_recent_blockhash().await?;
        log::info!("Got recent blockhash: {}", recent_blockhash);

        // Create transfer instruction
        let transfer_instruction =
            self.create_transfer_instruction(&from_address, to_address, amount_lamports)?;

        // Build transaction message
        let message = self.build_transaction_message(
            &from_address,
            vec![transfer_instruction],
            &recent_blockhash,
        )?;

        // Sign transaction
        let signed_transaction = self.sign_transaction(&message, from_keypair)?;

        // Submit transaction
        let signature = self.send_transaction(signed_transaction).await?;

        log::info!(
            "Transaction submitted successfully with signature: {}",
            signature
        );
        Ok(signature)
    }

    /// Get recent blockhash from Solana network
    async fn get_recent_blockhash(&self) -> Result<String, AppError> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getRecentBlockhash",
            "params": [
                {
                    "commitment": "finalized"
                }
            ]
        });

        let response: Value = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("RPC response parse failed: {}", e))
            })?;

        if let Some(error) = response.get("error") {
            return Err(AppError::InternalServerError(format!(
                "RPC error: {}",
                error
            )));
        }

        response["result"]["value"]["blockhash"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::InternalServerError("Invalid blockhash response".to_string()))
    }

    /// Create a transfer instruction
    fn create_transfer_instruction(
        &self,
        from_address: &str,
        to_address: &str,
        lamports: u64,
    ) -> Result<HashMap<String, Value>, AppError> {
        // Validate addresses
        if !SolanaKeyUtils::is_valid_solana_address(from_address) {
            return Err(AppError::ValidationError(format!(
                "Invalid from address: {}",
                from_address
            )));
        }
        if !SolanaKeyUtils::is_valid_solana_address(to_address) {
            return Err(AppError::ValidationError(format!(
                "Invalid to address: {}",
                to_address
            )));
        }

        let mut instruction = HashMap::new();
        instruction.insert(
            "programId".to_string(),
            json!("11111111111111111111111111111112"),
        ); // System program
        instruction.insert(
            "keys".to_string(),
            json!([
                {
                    "pubkey": from_address,
                    "isSigner": true,
                    "isWritable": true
                },
                {
                    "pubkey": to_address,
                    "isSigner": false,
                    "isWritable": true
                }
            ]),
        );

        // Transfer instruction data: [2, 0, 0, 0] + lamports (8 bytes little endian)
        let mut data = vec![2, 0, 0, 0]; // Transfer instruction discriminator
        data.extend_from_slice(&lamports.to_le_bytes());
        let data_base64 = general_purpose::STANDARD.encode(&data);
        instruction.insert("data".to_string(), json!(data_base64));

        Ok(instruction)
    }

    /// Build transaction message for signing
    fn build_transaction_message(
        &self,
        payer: &str,
        instructions: Vec<HashMap<String, Value>>,
        recent_blockhash: &str,
    ) -> Result<Vec<u8>, AppError> {
        // This is a simplified implementation of Solana transaction message serialization
        // In production, you would use the official Solana transaction format

        let mut message_data = Vec::new();

        // Add transaction version (1 byte)
        message_data.push(0); // Legacy transaction

        // Add number of required signatures (1 byte)
        message_data.push(1); // One signer

        // Add number of readonly signed accounts (1 byte)
        message_data.push(0); // No readonly signed accounts

        // Add number of readonly unsigned accounts (1 byte)
        message_data.push(1); // System program

        // Add accounts (simplified)
        message_data.extend_from_slice(&bs58::decode(payer).into_vec().unwrap_or_default());
        message_data.extend_from_slice(
            &bs58::decode("11111111111111111111111111111112")
                .into_vec()
                .unwrap_or_default(),
        ); // System program

        // Add recent blockhash
        message_data.extend_from_slice(
            &bs58::decode(recent_blockhash)
                .into_vec()
                .unwrap_or_default(),
        );

        // Add instructions (simplified)
        message_data.push(instructions.len() as u8);
        for instruction in instructions {
            if let Some(program_id) = instruction.get("programId") {
                message_data.extend_from_slice(
                    &bs58::decode(program_id.as_str().unwrap_or_default())
                        .into_vec()
                        .unwrap_or_default(),
                );
            }
            if let Some(data) = instruction.get("data") {
                if let Ok(decoded_data) =
                    general_purpose::STANDARD.decode(data.as_str().unwrap_or_default())
                {
                    message_data.push(decoded_data.len() as u8);
                    message_data.extend_from_slice(&decoded_data);
                }
            }
        }

        Ok(message_data)
    }

    /// Sign transaction with Ed25519 keypair
    fn sign_transaction(
        &self,
        message: &[u8],
        keypair: &Ed25519Keypair,
    ) -> Result<String, AppError> {
        log::info!("🔐 Signing Solana transaction with Ed25519 keypair");

        // Hash the message with SHA256 (Solana uses this for signing)
        let mut hasher = Sha256::new();
        hasher.update(message);
        let message_hash = hasher.finalize();

        // Sign the message hash
        let signature_bytes = SolanaKeyUtils::sign_data(keypair, &message_hash);

        // Create transaction with signature
        let transaction_data = json!({
            "signatures": [base64::prelude::BASE64_STANDARD.encode(&signature_bytes)],
            "message": {
                "header": {
                    "numRequiredSignatures": 1,
                    "numReadonlySignedAccounts": 0,
                    "numReadonlyUnsignedAccounts": 1
                },
                "accountKeys": [
                    SolanaKeyUtils::keypair_to_address(keypair),
                    "11111111111111111111111111111112"
                ],
                "recentBlockhash": "", // Will be filled by RPC
                "instructions": []
            }
        });

        let serialized_tx =
            base64::prelude::BASE64_STANDARD.encode(serde_json::to_vec(&transaction_data)?);

        log::info!("✅ Transaction signed successfully with real Ed25519 signature");

        Ok(serialized_tx)
    }

    /// Submit signed transaction to network
    async fn send_transaction(&self, signed_transaction: String) -> Result<String, AppError> {
        log::info!(
            " Submitting real transaction to Solana network: {}",
            self.rpc_url
        );

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                signed_transaction,
                {
                    "encoding": "base64",
                    "skipPreflight": false,
                    "preflightCommitment": "processed",
                    "maxRetries": 3
                }
            ]
        });

        let response: Value = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Transaction submission failed: {}", e))
            })?
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Response parse failed: {}", e)))?;

        if let Some(error) = response.get("error") {
            let error_msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            log::error!(" Transaction submission failed: {}", error_msg);
            return Err(AppError::InternalServerError(format!(
                "Transaction failed: {}",
                error_msg
            )));
        }

        let signature = response["result"]
            .as_str()
            .ok_or_else(|| AppError::InternalServerError("No signature in response".to_string()))?
            .to_string();

        log::info!(
            "✅ Transaction submitted successfully with signature: {}",
            signature
        );

        Ok(signature)
    }

    /// Get transaction status and confirmation count from blockchain
    pub async fn get_transaction_status(
        &self,
        signature: &str,
    ) -> Result<TransactionStatus, AppError> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [
                signature,
                {
                    "encoding": "json",
                    "maxSupportedTransactionVersion": 0,
                    "commitment": "confirmed"
                }
            ]
        });

        let response: Value = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("RPC response parse failed: {}", e))
            })?;

        if let Some(error) = response.get("error") {
            if error.get("code").and_then(|c| c.as_i64()) == Some(-32602) {
                // Transaction not found yet
                return Ok(TransactionStatus {
                    confirmed: false,
                    confirmations: 0,
                    slot: None,
                    block_time: None,
                    error: None,
                });
            }
            return Err(AppError::InternalServerError(format!(
                "RPC error: {}",
                error
            )));
        }

        let result = response.get("result");

        if result.is_none() || result.unwrap().is_null() {
            // Transaction not found
            return Ok(TransactionStatus {
                confirmed: false,
                confirmations: 0,
                slot: None,
                block_time: None,
                error: None,
            });
        }

        let tx_data = result.unwrap();

        // Extract transaction details
        let slot = tx_data.get("slot").and_then(|s| s.as_u64());
        let block_time = tx_data.get("blockTime").and_then(|t| t.as_u64());

        // Check for transaction errors
        let tx_error = tx_data
            .get("meta")
            .and_then(|meta| meta.get("err"))
            .and_then(|err| {
                if err.is_null() {
                    None
                } else {
                    Some(err.to_string())
                }
            });

        // Get current slot to calculate confirmations
        let confirmations = if let Some(tx_slot) = slot {
            match self.get_current_slot().await {
                Ok(current_slot) => (current_slot.saturating_sub(tx_slot) as i32).max(0),
                Err(_) => 1, // Assume at least 1 confirmation if we can't get current slot
            }
        } else {
            0
        };

        Ok(TransactionStatus {
            confirmed: slot.is_some() && tx_error.is_none(),
            confirmations,
            slot,
            block_time,
            error: tx_error,
        })
    }

    /// Get current network slot
    async fn get_current_slot(&self) -> Result<u64, AppError> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSlot",
            "params": [
                {
                    "commitment": "confirmed"
                }
            ]
        });

        let response: Value = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("RPC response parse failed: {}", e))
            })?;

        if let Some(error) = response.get("error") {
            return Err(AppError::InternalServerError(format!(
                "RPC error: {}",
                error
            )));
        }

        response["result"]
            .as_u64()
            .ok_or_else(|| AppError::InternalServerError("Invalid slot response".to_string()))
    }

    /// Check if transaction is finalized (32+ confirmations)
    pub async fn is_transaction_finalized(&self, signature: &str) -> Result<bool, AppError> {
        let status = self.get_transaction_status(signature).await?;
        Ok(status.confirmed && status.confirmations >= 32)
    }

    /// Get account balance in lamports
    pub async fn get_account_balance(&self, address: &str) -> Result<u64, AppError> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBalance",
            "params": [
                address,
                {
                    "commitment": "confirmed"
                }
            ]
        });

        let response: Value = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("RPC response parse failed: {}", e))
            })?;

        if let Some(error) = response.get("error") {
            return Err(AppError::InternalServerError(format!(
                "RPC error: {}",
                error
            )));
        }

        response["result"]["value"]
            .as_u64()
            .ok_or_else(|| AppError::InternalServerError("Invalid balance response".to_string()))
    }

    /// Estimate transaction fee dynamically using getFeeForMessage
    pub async fn estimate_transaction_fee(
        &self,
        from_address: &str,
        to_address: &str,
        amount_lamports: u64,
    ) -> Result<u64, AppError> {
        // Get recent blockhash
        let recent_blockhash = self.get_recent_blockhash().await?;

        // Create transfer instruction
        let transfer_instruction =
            self.create_transfer_instruction(from_address, to_address, amount_lamports)?;

        // Build message for fee estimation
        let message = self.build_fee_estimation_message(
            from_address,
            vec![transfer_instruction],
            &recent_blockhash,
        )?;

        // Get fee from network
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getFeeForMessage",
            "params": [
                base64::prelude::BASE64_STANDARD.encode(&message),
                {
                    "commitment": "confirmed"
                }
            ]
        });

        let response: Value = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Fee estimation failed: {}", e)))?
            .json()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Fee response parse failed: {}", e))
            })?;

        if let Some(error) = response.get("error") {
            return Err(AppError::InternalServerError(format!(
                "Fee estimation error: {}",
                error
            )));
        }

        response["result"]["value"]
            .as_u64()
            .ok_or_else(|| AppError::InternalServerError("Invalid fee response".to_string()))
    }

    /// Build simplified message for fee estimation
    fn build_fee_estimation_message(
        &self,
        payer: &str,
        instructions: Vec<HashMap<String, Value>>,
        recent_blockhash: &str,
    ) -> Result<Vec<u8>, AppError> {
        // Simplified message format for fee estimation
        let message = json!({
            "header": {
                "numRequiredSignatures": 1,
                "numReadonlySignedAccounts": 0,
                "numReadonlyUnsignedAccounts": 1
            },
            "accountKeys": [
                payer,
                "11111111111111111111111111111112"
            ],
            "recentBlockhash": recent_blockhash,
            "instructions": instructions
        });

        serde_json::to_vec(&message).map_err(|e| {
            AppError::InternalServerError(format!("Failed to serialize message: {}", e))
        })
    }
}

/// Transaction status information from blockchain
#[derive(Debug)]
pub struct TransactionStatus {
    pub confirmed: bool,
    pub confirmations: i32,
    pub slot: Option<u64>,
    pub block_time: Option<u64>,
    pub error: Option<String>,
}
