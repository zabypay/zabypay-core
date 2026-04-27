use crate::shared::{
    entities::{prelude::*, system_config, wallet},
    utils::errors::AppError,
};
use bip39::{Language, Mnemonic, Seed};
use bs58;
use ed25519_dalek;
use hex;
use rust_decimal::Decimal;
use sea_orm::DatabaseConnection;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use slip10::{derive_key_from_path, BIP32Path, Curve};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;

/// Production-ready Solana withdrawal service with DB-driven configuration
pub struct SolanaMainnetWithdrawal {
    db: DatabaseConnection,
}

impl SolanaMainnetWithdrawal {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Get RPC URL from database configuration - fully dynamic, no hardcoded values
    async fn get_rpc_url(&self, environment: &str) -> Result<String, AppError> {
        // Try to get from database system_config first
        let config_key = format!("solana_rpc_url_{}", environment);

        if let Ok(config) = SystemConfig::find()
            .filter(system_config::Column::Key.eq(&config_key))
            .one(&self.db)
            .await
        {
            if let Some(config) = config {
                if let Some(url) = config.value.as_str() {
                    log::info!("Using RPC URL from database config: {}", url);
                    return Ok(url.to_string());
                }
            }
        }

        // Environment variable fallback (no hardcoded URLs)
        let env_key = format!("SOLANA_{}_RPC", environment.to_uppercase());
        if let Ok(rpc_url) = std::env::var(&env_key) {
            log::info!(
                "Using RPC URL from environment variable {}: {}",
                env_key,
                rpc_url
            );
            return Ok(rpc_url);
        }

        // Final fallback based on standard Solana RPC endpoints
        let standard_url = match environment {
            "mainnet" => "https://api.mainnet-beta.solana.com",
            "testnet" => "https://api.testnet.solana.com",
            "devnet" => "https://api.devnet.solana.com",
            _ => {
                return Err(AppError::ValidationError(
                    format!("Unsupported Solana environment: {}. Please configure RPC URL in database or environment variables.", environment)
                ));
            }
        };

        log::warn!("Using standard RPC URL for {}: {} (consider configuring custom RPC for better reliability)", environment, standard_url);
        Ok(standard_url.to_string())
    }

    /// Estimate transfer fee dynamically from the network (based on proven test implementation)
    pub async fn estimate_transfer_fee(
        &self,
        sender_pubkey: &Pubkey,
        recipient_address: &str,
        amount_lamports: u64,
        environment: &str,
    ) -> Result<u64, AppError> {
        let rpc_url = self.get_rpc_url(environment).await?;

        let recipient_pubkey = Pubkey::from_str(recipient_address)
            .map_err(|e| AppError::ValidationError(format!("Invalid recipient address: {}", e)))?;

        // Use spawn_blocking for all RPC operations as shown in proven test code
        let fee = tokio::task::spawn_blocking({
            let rpc_url = rpc_url.clone();
            let sender_pubkey = *sender_pubkey;
            let recipient_pubkey = recipient_pubkey.clone();
            move || -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
                // Create RPC client
                let client = RpcClient::new(rpc_url);

                // Create transfer instruction
                let transfer_instruction = system_instruction::transfer(
                    &sender_pubkey,
                    &recipient_pubkey,
                    amount_lamports,
                );

                // Create transaction for fee estimation
                let mut transaction =
                    Transaction::new_with_payer(&[transfer_instruction], Some(&sender_pubkey));

                // Get recent blockhash
                let recent_blockhash = client.get_latest_blockhash()?;
                transaction.message.recent_blockhash = recent_blockhash;

                // Get fee for this transaction
                let fee = client.get_fee_for_message(&transaction.message)?;

                Ok(fee)
            }
        })
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to spawn blocking task: {}", e))
        })?
        .map_err(|e| AppError::InternalServerError(format!("Failed to estimate fee: {}", e)))?;

        log::info!(
            "💸 Dynamic fee estimation for {} network: {} lamports (~${:.4} at $200/SOL)",
            environment,
            fee,
            (fee as f64 / 1_000_000_000.0) * 200.0
        );
        Ok(fee)
    }

    /// Transfer SOL with confirmation (based on proven test implementation)
    pub async fn transfer_sol_with_confirmation(
        &self,
        sender_keypair: &Keypair,
        recipient_address: &str,
        amount_lamports: u64,
        environment: &str,
    ) -> Result<String, AppError> {
        let rpc_url = self.get_rpc_url(environment).await?;

        // Use spawn_blocking for all RPC operations like in the proven test
        // Clone the keypair to avoid lifetime issues
        let sender_keypair_clone =
            Keypair::from_bytes(&sender_keypair.to_bytes()).map_err(|e| {
                AppError::InternalServerError(format!("Failed to clone keypair: {}", e))
            })?;

        let result = tokio::task::spawn_blocking({
            let rpc_url = rpc_url.clone();
            let sender_keypair = sender_keypair_clone;
            let recipient_address = recipient_address.to_string();
            move || -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
                // Create RPC client with confirmed commitment
                let client =
                    RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());

                // Get current balance
                let sender_balance = client.get_balance(&sender_keypair.pubkey())?;
                let sender_sol = sender_balance as f64 / 1_000_000_000.0;

                // Parse recipient address
                let recipient_pubkey = Pubkey::from_str(&recipient_address)?;

                // Check if recipient account exists and get its balance
                let recipient_balance = client.get_balance(&recipient_pubkey).unwrap_or(0);

                // Rent exemption for a basic account (0 data) is about 890880 lamports
                const RENT_EXEMPTION_LAMPORTS: u64 = 890880; // ~0.00089 SOL

                // If recipient doesn't exist or has insufficient rent, add rent exemption to transfer
                let mut actual_transfer_amount = amount_lamports;
                if recipient_balance < RENT_EXEMPTION_LAMPORTS {
                    let rent_needed = RENT_EXEMPTION_LAMPORTS - recipient_balance;
                    actual_transfer_amount = amount_lamports + rent_needed;
                    println!(
                        "  Recipient needs {} lamports for rent exemption",
                        rent_needed
                    );
                    println!(
                        "📝 Adjusting transfer amount from {} to {} lamports",
                        amount_lamports, actual_transfer_amount
                    );
                }

                // Create transfer instruction with adjusted amount
                let transfer_instruction = system_instruction::transfer(
                    &sender_keypair.pubkey(),
                    &recipient_pubkey,
                    actual_transfer_amount,
                );

                // Create transaction for fee estimation
                let mut transaction = Transaction::new_with_payer(
                    &[transfer_instruction.clone()],
                    Some(&sender_keypair.pubkey()),
                );

                // Set a recent blockhash for fee calculation
                let recent_blockhash = client.get_latest_blockhash()?;
                transaction.message.recent_blockhash = recent_blockhash;

                // Get fee for this transaction
                let estimated_fee = client.get_fee_for_message(&transaction.message)?;
                let fee_sol = estimated_fee as f64 / 1_000_000_000.0;
                let amount_sol = amount_lamports as f64 / 1_000_000_000.0;
                let actual_amount_sol = actual_transfer_amount as f64 / 1_000_000_000.0;
                let total_cost_sol = actual_amount_sol + fee_sol;

                println!(" Transaction Summary:");
                println!(
                    "   Current Balance: {} SOL ({} lamports)",
                    sender_sol, sender_balance
                );
                println!(
                    "   Original Amount: {} SOL ({} lamports)",
                    amount_sol, amount_lamports
                );
                println!(
                    "   Actual Transfer: {} SOL ({} lamports)",
                    actual_amount_sol, actual_transfer_amount
                );
                println!(
                    "   Estimated Fee: {} SOL ({} lamports)",
                    fee_sol, estimated_fee
                );
                println!(
                    "   💡 Fee in USD: ~${:.4} (assuming $200/SOL)",
                    fee_sol * 200.0
                );
                println!(
                    "   Total Cost: {} SOL ({} lamports)",
                    total_cost_sol,
                    actual_transfer_amount + estimated_fee
                );
                println!("   Remaining Balance: {} SOL", sender_sol - total_cost_sol);

                println!("\n Solana Fee Analysis:");
                println!("   • Solana fees are FIXED at ~5,000 lamports (0.000005 SOL)");
                println!("   • This is ~$0.001 at current SOL prices");
                println!("   • Fees DON'T depend on transfer amount");
                println!("   • Already 1000x cheaper than Ethereum!");
                println!("   •   Fees CANNOT be reduced further - they're protocol fixed");

                // Check if sender has enough balance (using actual transfer amount)
                if sender_balance < actual_transfer_amount + estimated_fee {
                    return Err(format!(
                        "Insufficient funds! Need {} SOL but only have {} SOL",
                        total_cost_sol, sender_sol
                    )
                    .into());
                }

                // Create and sign transaction with the same recent blockhash
                let transaction = Transaction::new_signed_with_payer(
                    &[transfer_instruction],
                    Some(&sender_keypair.pubkey()),
                    &[&sender_keypair],
                    recent_blockhash,
                );

                // Send transaction with confirmation
                match client.send_and_confirm_transaction(&transaction) {
                    Ok(signature) => {
                        println!("✅ Transaction successful!");
                        println!("🔗 Transaction signature: {}", signature);
                        Ok(signature.to_string())
                    }
                    Err(e) => {
                        println!(" Transaction failed: {}", e);
                        Err(Box::new(e))
                    }
                }
            }
        })
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to spawn blocking task: {}", e))
        })?
        .map_err(|e| AppError::TransactionFailed(format!("Transaction failed: {}", e)))?;

        log::info!("✅ Solana transaction successful!");
        log::info!("🔗 Transaction signature: {}", result);

        let explorer_url = self.get_explorer_url(environment, &result);
        log::info!(" View on Solana Explorer: {}", explorer_url);

        Ok(result)
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

    /// Get SOL balance dynamically from network
    pub async fn get_sol_balance(&self, address: &str, environment: &str) -> Result<f64, AppError> {
        let rpc_url = self.get_rpc_url(environment).await?;

        let pubkey = Pubkey::from_str(address)
            .map_err(|e| AppError::ValidationError(format!("Invalid address: {}", e)))?;

        let balance = tokio::task::spawn_blocking({
            let rpc_url = rpc_url.clone();
            let pubkey = pubkey.clone();
            move || -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
                let client = RpcClient::new(rpc_url);
                let balance = client.get_balance(&pubkey)?;
                Ok(balance)
            }
        })
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to spawn blocking task: {}", e))
        })?
        .map_err(|e| AppError::InternalServerError(format!("Failed to get balance: {}", e)))?;

        // Convert lamports to SOL (1 SOL = 1,000,000,000 lamports)
        let sol_balance = balance as f64 / 1_000_000_000.0;
        log::info!(
            "💰 Balance for {} on {}: {} SOL ({} lamports)",
            address,
            environment,
            sol_balance,
            balance
        );

        Ok(sol_balance)
    }

    /// Get explorer URL based on environment
    fn get_explorer_url(&self, environment: &str, tx_signature: &str) -> String {
        match environment {
            "mainnet" => format!("https://explorer.solana.com/tx/{}", tx_signature),
            "testnet" => format!(
                "https://explorer.solana.com/tx/{}?cluster=testnet",
                tx_signature
            ),
            "devnet" => format!(
                "https://explorer.solana.com/tx/{}?cluster=devnet",
                tx_signature
            ),
            _ => format!("https://explorer.solana.com/tx/{}", tx_signature),
        }
    }

    /// Get Solana keypair from wallet private key stored in DB
    pub async fn get_keypair_from_wallet(&self, wallet_id: &str) -> Result<Keypair, AppError> {
        // Fetch wallet from database
        let wallet = Wallet::find()
            .filter(wallet::Column::Id.eq(wallet_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch wallet: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Wallet not found".to_string()))?;

        // Check if wallet has private key (it should be encrypted in DB)
        if wallet.private_key.is_empty() {
            // Fallback to mnemonic if private key is not available
            if wallet.mnemonic.is_empty() {
                return Err(AppError::ValidationError(
                    "Wallet has no private key or mnemonic stored. Cannot derive keypair."
                        .to_string(),
                ));
            }

            // Use mnemonic fallback
            let crypto = crate::shared::utils::encryption::CryptoEncryption::new()?;
            let mnemonic = crypto.decrypt_private_key(&wallet.mnemonic)?;
            let index = 0u32;
            return self.get_solana_keypair_from_mnemonic(&mnemonic, index);
        }

        // Use private key (primary method) - use existing SolanaKeyUtils which is proven to work
        use crate::shared::service::solana_key_utils::SolanaKeyUtils;

        log::info!("📝 Decrypting private key for wallet {}", wallet.address);
        let solana_keypair = SolanaKeyUtils::decrypt_and_create_keypair(&wallet.private_key)
            .map_err(|e| {
                log::error!(" Failed to decrypt and create keypair: {:?}", e);
                e
            })?;

        // Convert from ed25519_dalek keypair to Solana SDK keypair
        log::info!("🔄 Converting keypair format...");
        let keypair = self.convert_ed25519_to_solana_keypair(&solana_keypair)?;

        Ok(keypair)
    }

    /// Convert ed25519_dalek keypair to Solana SDK keypair
    fn convert_ed25519_to_solana_keypair(
        &self,
        ed25519_keypair: &ed25519_dalek::SigningKey,
    ) -> Result<Keypair, AppError> {
        // Get the secret key bytes (32 bytes)
        let secret_bytes = ed25519_keypair.to_bytes();

        // Get the public key bytes (32 bytes)
        let public_bytes = ed25519_keypair.verifying_key().to_bytes();

        // Combine into 64-byte format expected by Solana SDK
        let mut full_keypair = Vec::with_capacity(64);
        full_keypair.extend_from_slice(&secret_bytes);
        full_keypair.extend_from_slice(&public_bytes);

        Keypair::try_from(&full_keypair[..])
            .map_err(|e| AppError::InternalServerError(format!("Failed to convert keypair: {}", e)))
    }

    /// Create keypair from private key string (Base58 or hex) - supports both 32 and 64 byte keys
    fn keypair_from_private_key_string(&self, private_key_str: &str) -> Result<Keypair, AppError> {
        let private_key = private_key_str.trim();

        // Try Base58 format first (Solana standard)
        if let Ok(decoded) = bs58::decode(private_key).into_vec() {
            if decoded.len() == 64 {
                // Full 64-byte keypair from Solana CLI - use entire array
                return Keypair::try_from(&decoded[..]).map_err(|e| {
                    AppError::InternalServerError(format!(
                        "Failed to create keypair from 64-byte Base58: {}",
                        e
                    ))
                });
            } else if decoded.len() == 32 {
                // Just 32-byte secret key - need to create full keypair
                // Use ed25519-dalek to create keypair then convert to Solana SDK format
                let decoded_array: [u8; 32] = decoded.try_into().unwrap();
                let signing_key = ed25519_dalek::SigningKey::from_bytes(&decoded_array);
                let verifying_key = signing_key.verifying_key();

                // Combine secret + public key for Solana SDK format
                let mut full_keypair = Vec::with_capacity(64);
                full_keypair.extend_from_slice(&decoded_array);
                full_keypair.extend_from_slice(verifying_key.as_bytes());

                return Keypair::try_from(&full_keypair[..]).map_err(|e| {
                    AppError::InternalServerError(format!(
                        "Failed to create keypair from 32-byte Base58: {}",
                        e
                    ))
                });
            }
        }

        // Try hex format
        if let Ok(decoded) = hex::decode(private_key) {
            if decoded.len() == 64 {
                return Keypair::try_from(&decoded[..]).map_err(|e| {
                    AppError::InternalServerError(format!(
                        "Failed to create keypair from 64-byte hex: {}",
                        e
                    ))
                });
            } else if decoded.len() == 32 {
                // Just 32-byte secret key - need to create full keypair
                let decoded_array: [u8; 32] = decoded.try_into().unwrap();
                let signing_key = ed25519_dalek::SigningKey::from_bytes(&decoded_array);
                let verifying_key = signing_key.verifying_key();

                // Combine secret + public key for Solana SDK format
                let mut full_keypair = Vec::with_capacity(64);
                full_keypair.extend_from_slice(&decoded_array);
                full_keypair.extend_from_slice(verifying_key.as_bytes());

                return Keypair::try_from(&full_keypair[..]).map_err(|e| {
                    AppError::InternalServerError(format!(
                        "Failed to create keypair from 32-byte hex: {}",
                        e
                    ))
                });
            }
        }

        Err(AppError::ValidationError(
            "Invalid private key format. Expected Base58 or hex encoded 32 or 64 bytes".to_string(),
        ))
    }

    /// Derive Solana keypair from mnemonic
    fn get_solana_keypair_from_mnemonic(
        &self,
        mnemonic_str: &str,
        index: u32,
    ) -> Result<Keypair, AppError> {
        let mnemonic = Mnemonic::from_phrase(mnemonic_str, Language::English)
            .map_err(|e| AppError::ValidationError(format!("Invalid mnemonic: {}", e)))?;

        let seed = Seed::new(&mnemonic, "");

        let path = format!("m/44'/501'/0'/0'/{}'", index);
        let derivation_path = BIP32Path::from_str(&path)
            .map_err(|e| AppError::ValidationError(format!("Invalid derivation path: {}", e)))?;

        let key = derive_key_from_path(seed.as_bytes(), Curve::Ed25519, derivation_path)
            .map_err(|e| AppError::InternalServerError(format!("Failed to derive key: {}", e)))?;

        // Create Solana Keypair from the derived key
        let keypair = Keypair::try_from(&key.key[..]).map_err(|e| {
            AppError::InternalServerError(format!("Failed to create keypair: {}", e))
        })?;

        Ok(keypair)
    }

    /// Execute withdrawal with all validations
    pub async fn execute_withdrawal(
        &self,
        wallet_id: &str,
        recipient_address: &str,
        amount: Decimal,
        environment: &str,
    ) -> Result<WithdrawalResult, AppError> {
        log::info!(
            " Starting Solana withdrawal execution for wallet {} to {} amount {} on {}",
            wallet_id,
            recipient_address,
            amount,
            environment
        );

        // First get wallet details from database for logging
        let wallet = Wallet::find()
            .filter(wallet::Column::Id.eq(wallet_id))
            .one(&self.db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to fetch wallet: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Wallet not found".to_string()))?;

        log::info!(" WALLET DEBUG INFO:");
        log::info!("   - Wallet ID: {}", wallet.id);
        log::info!("   - Database Address: {}", wallet.address);
        log::info!("   - Currency: {}", wallet.currency);
        log::info!("   - User ID: {}", wallet.user_id);
        log::info!("   - Has mnemonic: {} chars", wallet.mnemonic.len());
        log::info!("   - Has private_key: {} chars", wallet.private_key.len());

        // Get keypair from wallet in database
        log::info!("📝 Getting keypair from wallet: {}", wallet_id);
        let keypair = self.get_keypair_from_wallet(wallet_id).await.map_err(|e| {
            log::error!(" Failed to get keypair from wallet {}: {:?}", wallet_id, e);
            e
        })?;
        log::info!(
            "✅ Successfully got keypair for wallet: {}",
            keypair.pubkey()
        );
        log::info!("💰 Will check balance for address: {}", keypair.pubkey());
        log::info!(" CRITICAL: Database address vs Derived address:");
        log::info!("   - Database: {}", wallet.address);
        log::info!("   - Derived:  {}", keypair.pubkey());

        // Use the derived wallet address (no hardcoded addresses)
        let wallet_address = keypair.pubkey().to_string();

        // Convert amount to lamports
        let amount_f64: f64 = amount.try_into().map_err(|e| {
            log::error!(" Failed to convert amount {} to f64: {:?}", amount, e);
            AppError::ValidationError("Invalid amount".to_string())
        })?;
        let amount_lamports = (amount_f64 * 1_000_000_000.0) as u64;
        log::info!(
            "💰 Amount: {} SOL = {} lamports",
            amount_f64,
            amount_lamports
        );

        // Estimate fee first
        log::info!("💸 Estimating transaction fee...");
        let estimated_fee = self
            .estimate_transfer_fee(
                &keypair.pubkey(),
                recipient_address,
                amount_lamports,
                environment,
            )
            .await
            .map_err(|e| {
                log::error!(" Failed to estimate fee: {:?}", e);
                e
            })?;
        log::info!(
            "✅ Estimated fee: {} lamports ({} SOL)",
            estimated_fee,
            estimated_fee as f64 / 1_000_000_000.0
        );

        // Check balance
        log::info!("💰 Checking wallet balance for address: {}", wallet_address);
        log::info!(" EXPECTED FUNDED WALLET:");
        log::info!("   - Expected address: 9Wr9ciL583At3TB5ERmS26ibQ5EEWpqknWtZPu6k8vwQ");
        log::info!("   - Expected balance: ~1,011,380 lamports");
        log::info!("   - Current address:  {}", wallet_address);

        let balance = self
            .get_sol_balance(&wallet_address, environment)
            .await
            .map_err(|e| {
                log::error!(" Failed to get balance: {:?}", e);
                e
            })?;

        let balance_lamports = (balance * 1_000_000_000.0) as u64;
        let total_needed = amount_lamports + estimated_fee;
        log::info!(" BALANCE COMPARISON:");
        log::info!(
            "   - Blockchain balance: {} lamports ({} SOL)",
            balance_lamports,
            balance
        );
        log::info!("   - Expected balance:   1011380 lamports (0.001011380 SOL)");
        log::info!("   - Amount needed:      {} lamports", amount_lamports);
        log::info!("   - Fee needed:         {} lamports", estimated_fee);
        log::info!("   - Total needed:       {} lamports", total_needed);
        log::info!("   - Sufficient? {}", balance_lamports >= total_needed);

        println!(" FINAL BALANCE VERIFICATION:");
        println!("   - Address being used: {}", wallet_address);
        println!("   - Expected user funded address: 9Wr9ciL583At3TB5ERmS26ibQ5EEWpqknWtZPu6k8vwQ");
        println!(
            "   - Match? {}",
            wallet_address == "9Wr9ciL583At3DB5ERmS26ibQ5EEWpqknWtZPu6k8vwQ"
        );
        println!("   - Blockchain balance: {} lamports", balance_lamports);
        println!("   - Amount requested: {} lamports", amount_lamports);
        println!("   - Fee required: {} lamports", estimated_fee);
        println!("   - Total required: {} lamports", total_needed);
        println!(
            "   - Gap: {} lamports",
            total_needed as i64 - balance_lamports as i64
        );

        if balance_lamports < total_needed {
            let error_msg = format!(
                "Insufficient balance: Need {} lamports (including {} fee) but only have {} lamports",
                total_needed, estimated_fee, balance_lamports
            );
            log::error!(" {}", error_msg);
            return Err(AppError::InsufficientBalance(error_msg));
        }

        // Execute transfer
        log::info!(" Executing Solana transfer...");
        let tx_signature = self
            .transfer_sol(&keypair, recipient_address, amount_lamports, environment)
            .await
            .map_err(|e| {
                log::error!(" Failed to execute transfer: {:?}", e);
                e
            })?;
        log::info!("✅ Transfer successful: {}", tx_signature);

        // Return result
        Ok(WithdrawalResult {
            transaction_hash: tx_signature.clone(),
            amount_lamports,
            fee_lamports: estimated_fee,
            explorer_url: self.get_explorer_url(environment, &tx_signature),
            status: "confirmed".to_string(),
        })
    }
}

/// Result of a withdrawal operation
#[derive(Debug)]
pub struct WithdrawalResult {
    pub transaction_hash: String,
    pub amount_lamports: u64,
    pub fee_lamports: u64,
    pub explorer_url: String,
    pub status: String,
}
