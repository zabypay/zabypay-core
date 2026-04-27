use crate::shared::utils::errors::AppError;
use bip39::{Language, Mnemonic, Seed};
use bs58;
use ed25519_dalek::{SigningKey, VerifyingKey};
use slip10::{derive_key_from_path, BIP32Path, Curve};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{keypair::Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;

pub fn get_solana_keypair_from_mnemonic(mnemonic_str: &str, index: u32) -> Keypair {
    let mnemonic = Mnemonic::from_phrase(mnemonic_str, Language::English).unwrap();
    let seed = Seed::new(&mnemonic, "");

    let path = format!("m/44'/501'/0'/0'/{}'", index);
    let derivation_path = BIP32Path::from_str(&path).unwrap();

    let key = derive_key_from_path(seed.as_bytes(), Curve::Ed25519, derivation_path).unwrap();

    let signing_key = SigningKey::from_bytes(&key.key);
    let verifying_key: VerifyingKey = signing_key.verifying_key();

    let mut keypair_bytes = [0u8; 64];
    keypair_bytes[..32].copy_from_slice(&signing_key.to_bytes());
    keypair_bytes[32..].copy_from_slice(verifying_key.as_bytes());

    Keypair::try_from(&keypair_bytes[..]).unwrap()
}

/// Generate Solana wallet (new mnemonic or derive from existing)
pub fn generate_solana_wallet(
    mnemonic_str: &str,
    index: u32,
) -> Result<(String, String, String), AppError> {
    // Either use provided mnemonic or generate a new one
    let mnemonic = if mnemonic_str.is_empty() {
        Mnemonic::new(bip39::MnemonicType::Words12, Language::English)
    } else {
        Mnemonic::from_phrase(mnemonic_str, Language::English)
            .map_err(|_| AppError::ValidationError("Invalid mnemonic".into()))?
    };

    let mnemonic_phrase = mnemonic.phrase().to_string();
    let keypair = get_solana_keypair_from_mnemonic(&mnemonic_phrase, index);

    let address = keypair.pubkey().to_string();
    let private_key_base58 = bs58::encode(keypair.to_bytes()).into_string();
    println!(
        "Generated Solana wallet - address: {}\n mnemonic: {}\n private_key: {}\n",
        address, mnemonic_phrase, private_key_base58
    );
    Ok((address, private_key_base58, mnemonic_phrase))
}

/// Estimate transfer fee for Solana transaction
pub async fn estimate_transfer_fee(
    sender_pubkey: &Pubkey,
    recipient_address: &str,
    amount_lamports: u64,
    rpc_url: Option<&str>,
) -> Result<u64, AppError> {
    let rpc_url = rpc_url.unwrap_or("https://api.mainnet-beta.solana.com");
    let client = RpcClient::new(rpc_url.to_string());

    let recipient_pubkey = Pubkey::from_str(recipient_address)
        .map_err(|e| AppError::ValidationError(format!("Invalid recipient address: {}", e)))?;

    // Create transfer instruction
    let transfer_instruction =
        system_instruction::transfer(sender_pubkey, &recipient_pubkey, amount_lamports);

    // Create transaction for fee estimation
    let mut transaction = Transaction::new_with_payer(&[transfer_instruction], Some(sender_pubkey));

    // Set a recent blockhash for fee calculation
    let recent_blockhash = client
        .get_latest_blockhash()
        .await
        .map_err(|e| AppError::InternalServerError(format!("Failed to get blockhash: {}", e)))?;
    transaction.message.recent_blockhash = recent_blockhash;

    // Get fee for this transaction
    let fee = client
        .get_fee_for_message(&transaction.message)
        .await
        .map_err(|e| AppError::InternalServerError(format!("Failed to estimate fee: {}", e)))?;

    Ok(fee)
}

/// Get SOL balance for an address
pub async fn get_sol_balance(address: &str, rpc_url: Option<&str>) -> Result<u64, AppError> {
    let rpc_url = rpc_url.unwrap_or("https://api.mainnet-beta.solana.com");
    let client = RpcClient::new(rpc_url.to_string());

    let pubkey = Pubkey::from_str(address)
        .map_err(|e| AppError::ValidationError(format!("Invalid address: {}", e)))?;

    let balance = client
        .get_balance(&pubkey)
        .await
        .map_err(|e| AppError::InternalServerError(format!("Failed to get balance: {}", e)))?;

    log::info!(
        "Balance for {}: {} lamports ({} SOL)",
        address,
        balance,
        balance as f64 / 1_000_000_000.0
    );

    Ok(balance)
}

/// Transfer SOL with confirmation
pub async fn transfer_sol_with_confirmation(
    sender_keypair: &Keypair,
    recipient_address: &str,
    amount_lamports: u64,
    rpc_url: Option<&str>,
) -> Result<String, AppError> {
    let rpc_url = rpc_url.unwrap_or("https://api.mainnet-beta.solana.com");
    let client = RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());

    // Get current balance
    let sender_balance = client
        .get_balance(&sender_keypair.pubkey())
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to get sender balance: {}", e))
        })?;

    // Estimate transaction fee
    let estimated_fee = estimate_transfer_fee(
        &sender_keypair.pubkey(),
        recipient_address,
        amount_lamports,
        Some(rpc_url),
    )
    .await?;

    log::info!(
        "Transaction summary: balance={} lamports, amount={} lamports, fee={} lamports",
        sender_balance,
        amount_lamports,
        estimated_fee
    );

    // Check if sender has enough balance
    if sender_balance < amount_lamports + estimated_fee {
        return Err(AppError::ValidationError(format!(
            "Insufficient balance: Need {} lamports (including {} fee) but only have {} lamports",
            amount_lamports + estimated_fee,
            estimated_fee,
            sender_balance
        )));
    }

    // Parse recipient address
    let recipient_pubkey = Pubkey::from_str(recipient_address)
        .map_err(|e| AppError::ValidationError(format!("Invalid recipient address: {}", e)))?;

    // Get recent blockhash
    let recent_blockhash = client
        .get_latest_blockhash()
        .await
        .map_err(|e| AppError::InternalServerError(format!("Failed to get blockhash: {}", e)))?;

    // Create transfer instruction
    let transfer_instruction =
        system_instruction::transfer(&sender_keypair.pubkey(), &recipient_pubkey, amount_lamports);

    // Create and sign transaction
    let transaction = Transaction::new_signed_with_payer(
        &[transfer_instruction],
        Some(&sender_keypair.pubkey()),
        &[sender_keypair],
        recent_blockhash,
    );

    // Send transaction
    match client.send_and_confirm_transaction(&transaction).await {
        Ok(signature) => {
            log::info!("✅ Solana transaction successful: {}", signature);
            Ok(signature.to_string())
        }
        Err(e) => {
            log::error!(" Solana transaction failed: {}", e);
            Err(AppError::InternalServerError(format!(
                "Transaction failed: {}",
                e
            )))
        }
    }
}

/// Get rent exemption amount for a basic account from Solana
pub async fn get_rent_exemption_amount(rpc_url: Option<&str>) -> Result<u64, AppError> {
    let rpc_url = rpc_url.unwrap_or("https://api.mainnet-beta.solana.com");
    let client = RpcClient::new(rpc_url.to_string());

    // Get minimum rent exemption for a basic account (0 bytes data)
    let rent_exemption = client
        .get_minimum_balance_for_rent_exemption(0)
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to get rent exemption: {}", e))
        })?;

    log::info!(
        "🔒 Retrieved rent exemption from Solana: {} lamports ({} SOL)",
        rent_exemption,
        rent_exemption as f64 / 1_000_000_000.0
    );

    Ok(rent_exemption)
}

/// Simple transfer function (wrapper for transfer_sol_with_confirmation)
pub async fn transfer_sol(
    sender_keypair: &Keypair,
    recipient_address: &str,
    amount_lamports: u64,
    rpc_url: Option<&str>,
) -> Result<String, AppError> {
    transfer_sol_with_confirmation(sender_keypair, recipient_address, amount_lamports, rpc_url)
        .await
}
