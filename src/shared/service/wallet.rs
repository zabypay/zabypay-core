use crate::shared::service::{
    btc_wallet::generate_bitcoin_wallet, eth_wallet::generate_evm_wallet,
    solana_wallet::generate_solana_wallet,
};
use crate::shared::utils::mnemonic::generate_mnemonic;
use crate::shared::{
    entities::{prelude::*, wallet},
    service::solana_key_utils::SolanaKeyUtils,
    utils::{encryption::CryptoEncryption, errors::AppError},
    AppState,
};
use bip39::{Language, Mnemonic};
use chrono::Utc;
use rand::rngs::OsRng;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha3::{Digest, Keccak256};
use std::str::FromStr;
use uuid::Uuid;

// Bitcoin wallet generation
use bdk::bitcoin::bip32::DerivationPath;
use bdk::bitcoin::secp256k1::Secp256k1 as BdkSecp256k1;
use bdk::bitcoin::{Address, Network, PrivateKey};
use bdk::keys::{DerivableKey, ExtendedKey};
use bdk::miniscript::Segwitv0;

// Solana wallet generation (simplified approach)
use bs58;
use slip10::{derive_key_from_path, BIP32Path, Curve};

#[derive(Debug, Clone)]
pub enum Currency {
    Bitcoin,
    Ethereum,
    USDT, // ERC-20 token on Ethereum
    Solana,
    BNB, // Binance Smart Chain
}

impl Currency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Currency::Bitcoin => "bitcoin",
            Currency::Ethereum => "ethereum",
            Currency::USDT => "usdt",
            Currency::Solana => "solana",
            Currency::BNB => "bnb",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, AppError> {
        match s.to_lowercase().as_str() {
            "bitcoin" | "btc" => Ok(Currency::Bitcoin),
            "ethereum" | "eth" => Ok(Currency::Ethereum),
            "usdt" | "tether" => Ok(Currency::USDT),
            "usdt_bnb" | "usdt_bep20" | "tether_bnb" => Ok(Currency::BNB), // USDT on BNB Chain uses BNB addresses
            "solana" | "sol" => Ok(Currency::Solana),
            "bnb" | "binance" | "binancecoin" => Ok(Currency::BNB),
            _ => Err(AppError::ValidationError(format!(
                "Unsupported currency: {}",
                s
            ))),
        }
    }

    pub fn get_supported_currencies() -> Vec<&'static str> {
        vec!["bitcoin", "ethereum", "usdt", "solana", "bnb"]
    }
}

pub struct WalletService;

impl WalletService {
    pub fn new() -> Self {
        Self
    }

    // fn generate_ethereum_wallet(mnemonic_str: &str, index: u32) -> Result<(String, String, String), AppError> {
    //     let mnemonic = Mnemonic::parse_in_normalized(Language::English, mnemonic_str)
    //         .map_err(|_| AppError::ValidationError("Invalid mnemonic phrase".to_string()))?;

    //     let seed = mnemonic.to_seed("");

    //     // Derive Ethereum private key using BIP44 path: m/44'/60'/0'/0/{index}
    //     let mut rng = OsRng;
    //     let private_key = SecretKey::new(&mut rng);
    //     let secp = Secp256k1::new();
    //     let public_key = PublicKey::from_secret_key(&secp, &private_key);

    //     // Generate Ethereum address from public key
    //     let public_key_bytes = public_key.serialize_uncompressed();
    //     let hash = Keccak256::digest(&public_key_bytes[1..]);
    //     let address = format!("0x{}", hex::encode(&hash[12..]));
    //     let private_key_hex = hex::encode(private_key.secret_bytes());
    //     println!("Memonic Ethereum: {} \n Private Key: {:?}", mnemonic.to_string(), private_key_hex);
    //     Ok((address, private_key_hex, mnemonic.to_string()))
    // }

    // fn generate_bitcoin_wallet(mnemonic_str: &str, index: u32) -> Result<(String, String, String), AppError> {
    //     let mnemonic = Mnemonic::parse_in_normalized(Language::English, mnemonic_str)
    //         .map_err(|_| AppError::ValidationError("Invalid mnemonic phrase".to_string()))?;

    //     let mnemonic_string = mnemonic.to_string(); // Store before consuming
    //     let xkey: ExtendedKey<Segwitv0> = mnemonic.into_extended_key()
    //         .map_err(|_| AppError::InternalServerError("Failed to create extended key".to_string()))?;
    //     let xprv = xkey.into_xprv(Network::Bitcoin)
    //         .ok_or_else(|| AppError::InternalServerError("Failed to create extended private key".to_string()))?;

    //     let derivation_path: DerivationPath = format!("m/44'/0'/0'/0/{}", index)
    //         .parse()
    //         .map_err(|_| AppError::ValidationError("Invalid derivation path".to_string()))?;

    //     let secp = BdkSecp256k1::new();
    //     let derived_prv = xprv.derive_priv(&secp, &derivation_path)
    //         .map_err(|_| AppError::InternalServerError("Failed to derive private key".to_string()))?;

    //     let private_key = PrivateKey {
    //         inner: derived_prv.private_key,
    //         compressed: true,
    //         network: Network::Bitcoin,
    //     };

    //     let public_key = private_key.public_key(&secp);
    //     let address = Address::p2wpkh(&public_key, Network::Bitcoin)
    //         .map_err(|_| AppError::InternalServerError("Failed to generate address".to_string()))?;

    //     println!("Memonic Bitcoin: {}, Private Key: {:?}", mnemonic_string, private_key.to_wif());
    //     Ok((
    //         address.to_string(),
    //         private_key.to_wif(),
    //         mnemonic_string,
    //     ))
    // }

    // fn generate_solana_wallet(_mnemonic_str: &str, _index: u32) -> Result<(String, String, String), AppError> {
    //     // Use the simplified keypair generation approach with proper random mnemonic
    //     let (address, private_key_base58, mnemonic) = SolanaKeyUtils::generate_new_keypair_simple()?;

    //     println!("Generated Solana wallet - Address: {} \n Mnemoic {:?} \n Private Key: {:?}", address, mnemonic, private_key_base58);

    //     Ok((
    //         address,
    //         private_key_base58,
    //         mnemonic,
    //     ))
    // }

    pub async fn generate_wallet(
        &self,
        user_id: &str,
        currency: Currency,
        db: &sea_orm::DatabaseConnection,
    ) -> Result<(String, String), AppError> {
        self.generate_wallet_with_key(user_id, currency.clone(), currency.as_str(), db)
            .await
    }

    pub async fn generate_wallet_with_key(
        &self,
        user_id: &str,
        currency: Currency,
        currency_key: &str,
        db: &sea_orm::DatabaseConnection,
    ) -> Result<(String, String), AppError> {
        // Generate a fresh mnemonic for this wallet
        let mnemonic_phrase = generate_mnemonic().map_err(|_| {
            AppError::InternalServerError("Failed to generate mnemonic".to_string())
        })?;

        let index = 0; // Default to first derived key

        let (address, private_key, mnemonic) = match currency {
            Currency::Bitcoin => generate_bitcoin_wallet(&mnemonic_phrase, index)?,
            Currency::Ethereum | Currency::USDT | Currency::BNB => {
                generate_evm_wallet(&mnemonic_phrase, index).map_err(|e| {
                    AppError::InternalServerError(format!("EVM wallet generation failed: {}", e))
                })?
            }
            Currency::Solana => generate_solana_wallet(&mnemonic_phrase, index)?,
        };

        // Encrypt sensitive data before storing
        let crypto = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;

        let encrypted_private_key = crypto.encrypt_private_key(&private_key).map_err(|e| {
            AppError::InternalServerError(format!("Failed to encrypt private key: {}", e))
        })?;

        let encrypted_mnemonic = crypto.encrypt_mnemonic(&mnemonic).map_err(|e| {
            AppError::InternalServerError(format!("Failed to encrypt mnemonic: {}", e))
        })?;

        // Save wallet to database with the provided currency_key
        let wallet = wallet::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id.to_string()),
            currency: Set(currency_key.to_string()), // Use the currency_key instead of currency.as_str()
            address: Set(address.clone()),
            public_key: Set(address.to_string()), // Will be populated if needed
            private_key: Set(encrypted_private_key), // Store encrypted private key
            mnemonic: Set(encrypted_mnemonic),    // Store encrypted mnemonic
            created_at: Set(Utc::now().into()),
        };

        wallet
            .insert(db)
            .await
            .map_err(|e| AppError::InternalServerError(format!("Failed to save wallet: {}", e)))?;

        log::info!(
            "Generated {} wallet for user {}: {}",
            currency_key,
            user_id,
            address
        );

        Ok((address, mnemonic))
    }

    /// Generate a merchant-specific wallet for withdrawals
    pub async fn generate_merchant_wallet(
        &self,
        merchant_id: &str,
        user_id: &str,
        currency: Currency,
        currency_key: &str,
        environment: &str, // mainnet, testnet
        db: &sea_orm::DatabaseConnection,
    ) -> Result<(String, String), AppError> {
        // Check if merchant already has a wallet for this currency
        let existing_wallet = wallet::Entity::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .filter(wallet::Column::Currency.eq(currency_key))
            .one(db)
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to check existing wallet: {}", e))
            })?;

        if let Some(existing) = existing_wallet {
            log::info!(
                "Merchant {} already has {} wallet: {}",
                merchant_id,
                currency_key,
                existing.address
            );

            // Decrypt and return existing mnemonic for consistency
            let crypto = CryptoEncryption::new().map_err(|e| {
                AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
            })?;

            let decrypted_mnemonic = crypto.decrypt_mnemonic(&existing.mnemonic).map_err(|e| {
                AppError::InternalServerError(format!("Failed to decrypt existing mnemonic: {}", e))
            })?;

            return Ok((existing.address, decrypted_mnemonic));
        }

        // Generate a fresh mnemonic for this merchant wallet
        let mnemonic_phrase = generate_mnemonic().map_err(|_| {
            AppError::InternalServerError("Failed to generate mnemonic".to_string())
        })?;

        let index = 0; // Default to first derived key

        let (address, private_key, mnemonic) = match currency {
            Currency::Bitcoin => generate_bitcoin_wallet(&mnemonic_phrase, index)?,
            Currency::Ethereum | Currency::USDT | Currency::BNB => {
                generate_evm_wallet(&mnemonic_phrase, index).map_err(|e| {
                    AppError::InternalServerError(format!("EVM wallet generation failed: {}", e))
                })?
            }
            Currency::Solana => generate_solana_wallet(&mnemonic_phrase, index)?,
        };

        // Encrypt sensitive data before storing
        let crypto = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;

        let encrypted_private_key = crypto.encrypt_private_key(&private_key).map_err(|e| {
            AppError::InternalServerError(format!("Failed to encrypt private key: {}", e))
        })?;

        let encrypted_mnemonic = crypto.encrypt_mnemonic(&mnemonic).map_err(|e| {
            AppError::InternalServerError(format!("Failed to encrypt mnemonic: {}", e))
        })?;

        // Create merchant-specific currency key with environment context
        let merchant_currency_key = format!(
            "{}_{}_merchant_{}",
            currency_key.split('_').next().unwrap_or(currency_key), // e.g., "sol" from "sol_mainnet_xxx"
            environment,
            merchant_id
        );

        // Save merchant wallet to database
        let wallet = wallet::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id.to_string()),
            currency: Set(merchant_currency_key.clone()),
            address: Set(address.clone()),
            public_key: Set(address.to_string()),
            private_key: Set(encrypted_private_key),
            mnemonic: Set(encrypted_mnemonic),
            created_at: Set(Utc::now().into()),
        };

        wallet.insert(db).await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to save merchant wallet: {}", e))
        })?;

        log::info!(
            "Generated merchant {} wallet for merchant {} ({}): {}",
            currency_key,
            merchant_id,
            environment,
            address
        );

        Ok((address, mnemonic))
    }

    /// Get all wallets for a user
    pub async fn get_user_wallets(
        db: &sea_orm::DatabaseConnection,
        user_id: &str,
    ) -> Result<Vec<wallet::Model>, AppError> {
        Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .all(db)
            .await
            .map_err(|e| AppError::InternalServerError(format!("Failed to fetch wallets: {}", e)))
    }

    /// Get wallet by user and currency
    pub async fn get_wallet_by_currency(
        db: &sea_orm::DatabaseConnection,
        user_id: &str,
        currency: &str,
    ) -> Result<Option<wallet::Model>, AppError> {
        Wallet::find()
            .filter(wallet::Column::UserId.eq(user_id))
            .filter(wallet::Column::Currency.eq(currency.to_lowercase()))
            .one(db)
            .await
            .map_err(|e| AppError::InternalServerError(format!("Failed to fetch wallet: {}", e)))
    }
}

pub fn validate_wallet_address(currency: &Currency, address: &str) -> Result<bool, AppError> {
    match currency {
        Currency::Bitcoin => validate_bitcoin_address(address),
        Currency::Ethereum | Currency::USDT | Currency::BNB => validate_ethereum_address(address),
        Currency::Solana => validate_solana_address(address),
    }
}

fn validate_bitcoin_address(address: &str) -> Result<bool, AppError> {
    match bdk::bitcoin::Address::from_str(address) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn validate_ethereum_address(address: &str) -> Result<bool, AppError> {
    if !address.starts_with("0x") || address.len() != 42 {
        return Ok(false);
    }

    match hex::decode(&address[2..]) {
        Ok(bytes) => Ok(bytes.len() == 20),
        Err(_) => Ok(false),
    }
}

fn validate_solana_address(address: &str) -> Result<bool, AppError> {
    // Basic Solana address validation
    match bs58::decode(address).into_vec() {
        Ok(bytes) => Ok(bytes.len() == 32),
        Err(_) => Ok(false),
    }
}

pub fn get_currency_decimals(currency: &Currency) -> u8 {
    match currency {
        Currency::Bitcoin => 8,
        Currency::Ethereum => 18,
        Currency::USDT => 6,
        Currency::Solana => 9,
        Currency::BNB => 18,
    }
}
