use crate::shared::utils::{encryption::CryptoEncryption, errors::AppError};
use bs58;
use ed25519_dalek::{SecretKey, Signer, SigningKey as Ed25519Keypair, VerifyingKey as PublicKey};

pub struct SolanaKeyUtils;

impl SolanaKeyUtils {
    /// Decrypt private key from database and create Ed25519 keypair
    pub fn decrypt_and_create_keypair(
        encrypted_private_key: &str,
    ) -> Result<Ed25519Keypair, AppError> {
        // Check if private key is empty (legacy wallets)
        if encrypted_private_key.is_empty() {
            return Err(AppError::ValidationError(
                "Wallet has no private key stored. This wallet was created before private key encryption was implemented. Please regenerate this wallet.".to_string()
            ));
        }

        // Initialize encryption utility
        let crypto = CryptoEncryption::new()?;

        // Decrypt the private key
        let private_key = crypto.decrypt_private_key(encrypted_private_key)?;

        log::info!("Decrypted private key successfully");

        // Parse and create Ed25519 keypair
        Self::parse_private_key_for_signing(&private_key)
    }

    /// Decrypt private key and return raw bytes for Solana SDK Keypair
    pub fn decrypt_private_key(encrypted_private_key: &str) -> Result<Vec<u8>, AppError> {
        // Check if private key is empty (legacy wallets)
        if encrypted_private_key.is_empty() {
            return Err(AppError::ValidationError(
                "Wallet has no private key stored. This wallet was created before private key encryption was implemented. Please regenerate this wallet.".to_string()
            ));
        }

        // Initialize encryption utility
        let crypto = CryptoEncryption::new()?;

        // Decrypt the private key
        let private_key = crypto.decrypt_private_key(encrypted_private_key)?;
        let private_key = private_key.trim();

        // Try Base58 format first (Solana standard)
        if let Ok(decoded) = bs58::decode(private_key).into_vec() {
            if decoded.len() == 64 {
                // Full 64-byte keypair from Solana CLI (contains both private and public keys)
                return Ok(decoded);
            } else if decoded.len() == 32 {
                // Just the 32-byte secret key - need to expand to full keypair
                // Create ed25519 keypair to get the public key
                let mut key_bytes = [0u8; 32];
                key_bytes.copy_from_slice(&decoded[0..32]);
                let signing_key = Ed25519Keypair::from_bytes(&key_bytes);
                let public_key = signing_key.verifying_key();

                // Combine private and public keys for Solana SDK format
                let mut full_keypair = Vec::with_capacity(64);
                full_keypair.extend_from_slice(&decoded);
                full_keypair.extend_from_slice(public_key.as_bytes());
                return Ok(full_keypair);
            }
        }

        // Try hex format
        if let Ok(decoded) = hex::decode(private_key) {
            if decoded.len() == 64 {
                return Ok(decoded);
            } else if decoded.len() == 32 {
                // Create full keypair from 32-byte secret
                let mut key_bytes = [0u8; 32];
                key_bytes.copy_from_slice(&decoded[0..32]);
                let signing_key = Ed25519Keypair::from_bytes(&key_bytes);
                let public_key = signing_key.verifying_key();

                let mut full_keypair = Vec::with_capacity(64);
                full_keypair.extend_from_slice(&decoded);
                full_keypair.extend_from_slice(public_key.as_bytes());
                return Ok(full_keypair);
            }
        }

        Err(AppError::ValidationError(
            "Invalid private key format. Expected Base58 or hex encoded 32 or 64 bytes".to_string(),
        ))
    }

    /// Parse private key for signing operations
    pub fn parse_private_key_for_signing(private_key: &str) -> Result<Ed25519Keypair, AppError> {
        let private_key = private_key.trim();

        // Try Base58 format first
        if let Ok(decoded) = bs58::decode(private_key).into_vec() {
            if decoded.len() == 64 {
                // Full 64-byte keypair from Solana CLI (32-byte secret + 32-byte public)
                return Self::create_keypair_from_bytes(&decoded[0..32]);
            } else if decoded.len() == 32 {
                // Just the 32-byte secret key
                return Self::create_keypair_from_bytes(&decoded);
            }
        }

        // Try hex format
        if let Ok(decoded) = hex::decode(private_key) {
            if decoded.len() == 64 {
                // Full 64-byte keypair
                return Self::create_keypair_from_bytes(&decoded[0..32]);
            } else if decoded.len() == 32 {
                return Self::create_keypair_from_bytes(&decoded);
            }
        }

        Err(AppError::ValidationError(
            "Invalid private key format. Expected Base58 or hex encoded 32 or 64 bytes".to_string(),
        ))
    }

    /// Create Ed25519 keypair from private key bytes
    fn create_keypair_from_bytes(private_bytes: &[u8]) -> Result<Ed25519Keypair, AppError> {
        if private_bytes.len() != 32 {
            return Err(AppError::ValidationError(format!(
                "Private key must be 32 bytes, got {}",
                private_bytes.len()
            )));
        }

        // Create keypair directly from bytes for ed25519-dalek v2.0
        let mut keypair_bytes = [0u8; 32];
        keypair_bytes.copy_from_slice(private_bytes);
        let keypair = Ed25519Keypair::from_bytes(&keypair_bytes);

        log::info!(
            "Created Ed25519 keypair with public key: {}",
            bs58::encode(keypair.verifying_key().to_bytes()).into_string()
        );

        Ok(keypair)
    }

    /// Get Solana address from Ed25519 keypair
    pub fn keypair_to_address(keypair: &Ed25519Keypair) -> String {
        bs58::encode(keypair.verifying_key().to_bytes()).into_string()
    }

    /// Sign arbitrary data with Ed25519 keypair
    pub fn sign_data(keypair: &Ed25519Keypair, data: &[u8]) -> Vec<u8> {
        let signature = keypair.sign(data);
        signature.to_bytes().to_vec()
    }

    /// Validate if a string is a valid Solana address (basic validation)
    pub fn is_valid_solana_address(address: &str) -> bool {
        // Basic validation: Solana addresses are typically 32-44 characters, base58 encoded
        if address.len() < 32 || address.len() > 44 {
            return false;
        }

        // Try to decode as base58
        bs58::decode(address).into_vec().is_ok()
    }

    /// Convert SOL amount (as string) to lamports
    pub fn sol_string_to_lamports(sol_str: &str) -> Result<u64, AppError> {
        let sol_amount: f64 = sol_str
            .parse()
            .map_err(|_| AppError::ValidationError(format!("Invalid SOL amount: {}", sol_str)))?;

        if sol_amount < 0.0 {
            return Err(AppError::ValidationError(
                "SOL amount cannot be negative".to_string(),
            ));
        }

        // Convert to lamports (1 SOL = 1,000,000,000 lamports)
        let lamports = (sol_amount * 1_000_000_000.0) as u64;
        Ok(lamports)
    }

    /// Generate a new Solana keypair (simplified version for wallet generation)
    pub fn generate_new_keypair_simple() -> Result<(String, String, String), AppError> {
        // Generate random 32-byte private key
        let mut private_key_bytes = [0u8; 32];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut private_key_bytes);

        // Create Ed25519 keypair
        let keypair = Ed25519Keypair::from_bytes(&private_key_bytes);

        // Generate proper Solana address (base58-encoded public key)
        let address = bs58::encode(keypair.verifying_key().to_bytes()).into_string();

        // Encode private key as base58
        let private_key_base58 = bs58::encode(&private_key_bytes).into_string();

        // Generate a proper random BIP39 mnemonic
        let mnemonic = crate::shared::utils::mnemonic::generate_mnemonic()?;
        println!(
            "mnemonic: {}, address: {}, private_key_base58: {}",
            mnemonic, address, private_key_base58
        );
        Ok((address, private_key_base58, mnemonic))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sol_to_lamports() {
        assert_eq!(
            SolanaKeyUtils::sol_string_to_lamports("1.0").unwrap(),
            1_000_000_000
        );
        assert_eq!(
            SolanaKeyUtils::sol_string_to_lamports("0.001").unwrap(),
            1_000_000
        );
        assert_eq!(
            SolanaKeyUtils::sol_string_to_lamports("0.000000001").unwrap(),
            1
        );

        assert!(SolanaKeyUtils::sol_string_to_lamports("-1.0").is_err());
        assert!(SolanaKeyUtils::sol_string_to_lamports("invalid").is_err());
    }

    #[test]
    fn test_address_validation() {
        // Valid Solana addresses
        assert!(SolanaKeyUtils::is_valid_solana_address(
            "11111111111111111111111111111112"
        ));
        assert!(SolanaKeyUtils::is_valid_solana_address(
            "3KfatZ2DwXiqm6ofPoXrcuJVbMprmrgPYR41m3k6o3gp"
        ));

        // Invalid addresses
        assert!(!SolanaKeyUtils::is_valid_solana_address("invalid"));
        assert!(!SolanaKeyUtils::is_valid_solana_address(""));
        assert!(!SolanaKeyUtils::is_valid_solana_address(
            "0x1234567890abcdef"
        ));
    }

    #[test]
    fn test_private_key_parsing() {
        // Test hex private key (32 bytes for Ed25519)
        let hex_key = "a".repeat(64); // 64 hex chars = 32 bytes
        let result = SolanaKeyUtils::parse_private_key_for_signing(&hex_key);
        assert!(result.is_ok());

        // Test Base58 private key (32 bytes)
        let mock_bytes = [1u8; 32]; // Ed25519 uses 32-byte keys
        let base58_key = bs58::encode(&mock_bytes).into_string();
        let result = SolanaKeyUtils::parse_private_key_for_signing(&base58_key);
        assert!(result.is_ok());
    }
}
