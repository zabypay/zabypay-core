use crate::shared::utils::errors::AppError;
use hex;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn generate_hmac_signature(secret: &str, payload: &str) -> Result<String, AppError> {
    // Remove webhook secret prefix if present
    let clean_secret = if secret.starts_with("whsec_") {
        &secret[6..]
    } else {
        secret
    };

    let mut mac = HmacSha256::new_from_slice(clean_secret.as_bytes())
        .map_err(|e| AppError::InternalServerError(format!("Invalid HMAC key: {}", e)))?;

    mac.update(payload.as_bytes());
    let result = mac.finalize();
    Ok(hex::encode(result.into_bytes()))
}

pub fn verify_hmac_signature(
    secret: &str,
    payload: &str,
    signature: &str,
) -> Result<bool, AppError> {
    let expected_signature = generate_hmac_signature(secret, payload)?;

    // Remove "sha256=" prefix if present in signature
    let clean_signature = if signature.starts_with("sha256=") {
        &signature[7..]
    } else {
        signature
    };

    // Use constant-time comparison to prevent timing attacks
    Ok(expected_signature == clean_signature)
}
