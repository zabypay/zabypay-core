use crate::shared::utils::errors::AppError;
use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use base64::{engine::general_purpose, Engine as _};
use rand::{distributions::Alphanumeric, Rng};

pub struct ApiKeyPair {
    pub api_key: String,
    pub secret_key: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub secret_hash: String,
}

pub fn generate_api_key_pair() -> Result<ApiKeyPair, AppError> {
    // Generate random API key (32 bytes -> base64)
    let api_key_bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen()).collect();
    let api_key = general_purpose::URL_SAFE_NO_PAD.encode(&api_key_bytes);

    // Generate random secret key (32 bytes -> base64)
    let secret_key_bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen()).collect();
    let secret_key = general_purpose::URL_SAFE_NO_PAD.encode(&secret_key_bytes);

    // Create key prefix (first 8 characters for identification)
    let key_prefix = api_key[..8].to_string();

    // Hash both keys with Argon2
    let argon2 = Argon2::default();
    let salt = SaltString::generate(&mut OsRng);

    let key_hash = argon2
        .hash_password(api_key.as_bytes(), &salt)
        .map_err(|e| AppError::InternalServerError(format!("Failed to hash API key: {}", e)))?
        .to_string();

    let secret_salt = SaltString::generate(&mut OsRng);
    let secret_hash = argon2
        .hash_password(secret_key.as_bytes(), &secret_salt)
        .map_err(|e| AppError::InternalServerError(format!("Failed to hash secret key: {}", e)))?
        .to_string();

    Ok(ApiKeyPair {
        api_key: format!("ak_{}", api_key), // Prefix for identification
        secret_key: format!("sk_{}", secret_key), // Prefix for identification
        key_prefix,
        key_hash,
        secret_hash,
    })
}

pub fn verify_api_key(key: &str, hash: &str) -> Result<bool, AppError> {
    // Remove prefix if present
    let clean_key = if key.starts_with("ak_") {
        &key[3..]
    } else {
        key
    };

    let argon2 = Argon2::default();
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| AppError::InternalServerError(format!("Invalid hash format: {}", e)))?;

    Ok(argon2
        .verify_password(clean_key.as_bytes(), &parsed_hash)
        .is_ok())
}

pub fn verify_secret_key(secret: &str, hash: &str) -> Result<bool, AppError> {
    // Remove prefix if present
    let clean_secret = if secret.starts_with("sk_") {
        &secret[3..]
    } else {
        secret
    };

    let argon2 = Argon2::default();
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| AppError::InternalServerError(format!("Invalid hash format: {}", e)))?;

    Ok(argon2
        .verify_password(clean_secret.as_bytes(), &parsed_hash)
        .is_ok())
}

pub fn generate_webhook_secret() -> String {
    // Generate 32 byte random secret for webhook signing
    let secret_bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen()).collect();
    general_purpose::URL_SAFE_NO_PAD.encode(&secret_bytes)
}
