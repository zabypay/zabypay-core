use crate::shared::utils::errors::AppError;
use aes_gcm::{
    aead::{Aead, OsRng},
    AeadCore, Aes256Gcm, KeyInit, Nonce,
};
use base64::{engine::general_purpose, Engine as _};
use std::env;

/// Encryption utility for highly sensitive data like private keys and mnemonics
/// Uses AES-256-GCM for authenticated encryption
pub struct CryptoEncryption {
    cipher: Aes256Gcm,
}

impl CryptoEncryption {
    /// Initialize encryption with a key derived from environment variable
    /// In production, this should be sourced from a secure key management system
    pub fn new() -> Result<Self, AppError> {
        let encryption_key = env::var("CRYPTO_ENCRYPTION_KEY")
            .map_err(|_| AppError::InternalServerError(
                "CRYPTO_ENCRYPTION_KEY environment variable not set. This is required for encrypting sensitive crypto data.".to_string()
            ))?;

        if encryption_key.len() != 64 {
            return Err(AppError::InternalServerError(
                "CRYPTO_ENCRYPTION_KEY must be exactly 64 characters (32 bytes in hex)".to_string(),
            ));
        }

        let key_bytes = hex::decode(&encryption_key).map_err(|_| {
            AppError::InternalServerError(
                "Invalid CRYPTO_ENCRYPTION_KEY format. Must be hex.".to_string(),
            )
        })?;

        if key_bytes.len() != 32 {
            return Err(AppError::InternalServerError(
                "CRYPTO_ENCRYPTION_KEY must be 32 bytes".to_string(),
            ));
        }

        let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|_| {
            AppError::InternalServerError("Failed to initialize cipher".to_string())
        })?;

        Ok(Self { cipher })
    }

    /// Encrypt sensitive data (private keys, mnemonics)
    /// Returns base64-encoded encrypted data with nonce prepended
    pub fn encrypt(&self, plaintext: &str) -> Result<String, AppError> {
        if plaintext.is_empty() {
            return Err(AppError::ValidationError(
                "Cannot encrypt empty string".to_string(),
            ));
        }

        // Generate random nonce
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        // Encrypt the data
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| AppError::InternalServerError("Encryption failed".to_string()))?;

        // Prepend nonce to ciphertext for storage
        let mut encrypted_data = nonce.to_vec();
        encrypted_data.extend_from_slice(&ciphertext);

        // Encode as base64 for database storage
        Ok(general_purpose::STANDARD.encode(&encrypted_data))
    }

    /// Decrypt sensitive data
    /// Expects base64-encoded data with nonce prepended
    pub fn decrypt(&self, encrypted_data: &str) -> Result<String, AppError> {
        if encrypted_data.is_empty() {
            return Err(AppError::ValidationError(
                "Cannot decrypt empty string".to_string(),
            ));
        }

        // Decode from base64
        let encrypted_bytes = general_purpose::STANDARD
            .decode(encrypted_data)
            .map_err(|_| {
                AppError::InternalServerError("Invalid base64 encrypted data".to_string())
            })?;

        if encrypted_bytes.len() < 12 {
            return Err(AppError::InternalServerError(
                "Encrypted data too short".to_string(),
            ));
        }

        // Extract nonce (first 12 bytes)
        let nonce = Nonce::from_slice(&encrypted_bytes[0..12]);

        // Extract ciphertext (remaining bytes)
        let ciphertext = &encrypted_bytes[12..];

        // Decrypt
        let plaintext = self.cipher.decrypt(nonce, ciphertext).map_err(|_| {
            AppError::InternalServerError("Decryption failed - data may be corrupted".to_string())
        })?;

        String::from_utf8(plaintext).map_err(|_| {
            AppError::InternalServerError("Decrypted data is not valid UTF-8".to_string())
        })
    }

    /// Encrypt a private key with additional validation
    pub fn encrypt_private_key(&self, private_key: &str) -> Result<String, AppError> {
        // Validate private key format (basic check)
        if private_key.len() < 32 {
            return Err(AppError::ValidationError(
                "Private key too short".to_string(),
            ));
        }

        self.encrypt(private_key)
    }

    /// Encrypt a mnemonic phrase with additional validation
    pub fn encrypt_mnemonic(&self, mnemonic: &str) -> Result<String, AppError> {
        // Validate mnemonic format (basic check)
        let words: Vec<&str> = mnemonic.split_whitespace().collect();
        if words.len() != 12 && words.len() != 24 {
            return Err(AppError::ValidationError(
                "Mnemonic must be 12 or 24 words".to_string(),
            ));
        }

        self.encrypt(mnemonic)
    }

    /// Decrypt a private key
    pub fn decrypt_private_key(&self, encrypted_private_key: &str) -> Result<String, AppError> {
        self.decrypt(encrypted_private_key)
    }

    /// Decrypt a mnemonic phrase
    pub fn decrypt_mnemonic(&self, encrypted_mnemonic: &str) -> Result<String, AppError> {
        self.decrypt(encrypted_mnemonic)
    }
}

/// Generate a new encryption key for development/testing
/// **WARNING: Only use this for development. Production keys should be generated securely.**
pub fn generate_encryption_key() -> String {
    use rand::RngCore;
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    hex::encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn setup_test_key() {
        env::set_var(
            "CRYPTO_ENCRYPTION_KEY",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        );
    }

    #[test]
    fn test_encryption_roundtrip() {
        setup_test_key();
        let crypto = CryptoEncryption::new().unwrap();

        let original = "test private key data";
        let encrypted = crypto.encrypt(original).unwrap();
        let decrypted = crypto.decrypt(&encrypted).unwrap();

        assert_eq!(original, decrypted);
    }

    #[test]
    fn test_mnemonic_validation() {
        setup_test_key();
        let crypto = CryptoEncryption::new().unwrap();

        // Valid 12-word mnemonic
        let valid_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert!(crypto.encrypt_mnemonic(valid_mnemonic).is_ok());

        // Invalid mnemonic (wrong word count)
        let invalid_mnemonic = "abandon abandon abandon";
        assert!(crypto.encrypt_mnemonic(invalid_mnemonic).is_err());
    }

    #[test]
    fn test_private_key_validation() {
        setup_test_key();
        let crypto = CryptoEncryption::new().unwrap();

        // Valid private key length
        let valid_key = "a".repeat(64);
        assert!(crypto.encrypt_private_key(&valid_key).is_ok());

        // Invalid private key (too short)
        let invalid_key = "short";
        assert!(crypto.encrypt_private_key(invalid_key).is_err());
    }
}
