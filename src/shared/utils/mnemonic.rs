use crate::shared::utils::errors::AppError;
use bip39::{Language, Mnemonic, MnemonicType};
use rand::RngCore;

// /// Generate a new 12-word BIP39 mnemonic phrase
// pub fn generate_mnemonic() -> Result<String, AppError> {
//     let mut entropy = [0u8; 16]; // 128 bits for 12 words
//     rand::thread_rng().fill_bytes(&mut entropy);

//     let mnemonic = Mnemonic::from_entropy(&entropy)
//         .map_err(|_| AppError::InternalServerError("Failed to generate mnemonic from entropy".to_string()))?;

//     Ok(mnemonic.to_string())
// }

pub fn generate_mnemonic() -> Result<String, AppError> {
    let mnemonic = Mnemonic::new(MnemonicType::Words24, Language::English);

    // println!("Mnemonic: {}", mnemonic.to_string());
    Ok(mnemonic.to_string())
}

// /// Generate a new 24-word BIP39 mnemonic phrase for higher security
// pub fn generate_mnemonic_24() -> Result<String, AppError> {
//     let mut entropy = [0u8; 32]; // 256 bits for 24 words
//     rand::thread_rng().fill_bytes(&mut entropy);

//     let mnemonic = Mnemonic::from_entropy(&entropy)
//         .map_err(|_| AppError::InternalServerError("Failed to generate mnemonic from entropy".to_string()))?;

//     Ok(mnemonic.to_string())
// }

// /// Validate a mnemonic phrase
// pub fn validate_mnemonic(phrase: &str) -> Result<bool, AppError> {
//     match Mnemonic::parse_in_normalized(Language::English, phrase) {
//         Ok(_) => Ok(true),
//         Err(_) => Ok(false),
//     }
// }

// /// Get the seed from a mnemonic phrase
// pub fn mnemonic_to_seed(phrase: &str) -> Result<[u8; 64], AppError> {
//     let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase)
//         .map_err(|_| AppError::ValidationError("Invalid mnemonic phrase".to_string()))?;

//     Ok(mnemonic.to_seed(""))
// }
