use crate::shared::utils::errors::AppError;
use regex::Regex;
use rust_decimal::Decimal;
use std::str::FromStr;

/// Input validation utilities for production security
pub struct InputValidator;

impl InputValidator {
    /// Validate cryptocurrency amount
    pub fn validate_crypto_amount(amount: &str, currency: &str) -> Result<Decimal, AppError> {
        let amount_decimal = Decimal::from_str(amount)
            .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

        if amount_decimal <= Decimal::ZERO {
            return Err(AppError::ValidationError(
                "Amount must be greater than zero".to_string(),
            ));
        }

        // Check maximum values based on currency
        let max_amount = match currency.to_lowercase().as_str() {
            "btc" | "bitcoin" => Decimal::from_str("21000000").unwrap(), // Max BTC supply
            "eth" | "ethereum" => Decimal::from_str("1000000").unwrap(), // Reasonable max for ETH
            "sol" | "solana" => Decimal::from_str("1000000").unwrap(),   // Reasonable max for SOL
            "bnb" => Decimal::from_str("1000000").unwrap(),              // Reasonable max for BNB
            "usdt" => Decimal::from_str("10000000").unwrap(),            // Reasonable max for USDT
            _ => Decimal::from_str("1000000").unwrap(),                  // Default max
        };

        if amount_decimal > max_amount {
            return Err(AppError::ValidationError(format!(
                "Amount exceeds maximum allowed for {}: {}",
                currency, max_amount
            )));
        }

        Ok(amount_decimal)
    }

    /// Validate wallet address format
    pub fn validate_wallet_address(address: &str, network: &str) -> Result<(), AppError> {
        if address.is_empty() {
            return Err(AppError::ValidationError(
                "Wallet address cannot be empty".to_string(),
            ));
        }

        if address.len() > 200 {
            return Err(AppError::ValidationError(
                "Wallet address too long".to_string(),
            ));
        }

        // Basic format validation based on network
        match network.to_lowercase().as_str() {
            "bitcoin" | "btc" => {
                if !Self::is_valid_bitcoin_address(address) {
                    return Err(AppError::ValidationError(
                        "Invalid Bitcoin address format".to_string(),
                    ));
                }
            }
            "ethereum" | "eth" | "usdt" => {
                if !Self::is_valid_ethereum_address(address) {
                    return Err(AppError::ValidationError(
                        "Invalid Ethereum address format".to_string(),
                    ));
                }
            }
            "solana" | "sol" => {
                if !Self::is_valid_solana_address(address) {
                    return Err(AppError::ValidationError(
                        "Invalid Solana address format".to_string(),
                    ));
                }
            }
            "bnb" | "usdt_bnb" => {
                if !Self::is_valid_ethereum_address(address) {
                    // BNB uses same format as ETH
                    return Err(AppError::ValidationError(
                        "Invalid BNB address format".to_string(),
                    ));
                }
            }
            _ => {
                // For unknown networks, just check basic alphanumeric
                if !address.chars().all(|c| c.is_alphanumeric()) {
                    return Err(AppError::ValidationError(
                        "Invalid address format".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Validate email format
    pub fn validate_email(email: &str) -> Result<(), AppError> {
        let email_regex = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
            .map_err(|_| AppError::InternalServerError("Email validation error".to_string()))?;

        if !email_regex.is_match(email) {
            return Err(AppError::ValidationError(
                "Invalid email format".to_string(),
            ));
        }

        if email.len() > 255 {
            return Err(AppError::ValidationError(
                "Email address too long".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate merchant ID format
    pub fn validate_merchant_id(merchant_id: &str) -> Result<(), AppError> {
        if merchant_id.is_empty() {
            return Err(AppError::ValidationError(
                "Merchant ID cannot be empty".to_string(),
            ));
        }

        if merchant_id.len() > 50 {
            return Err(AppError::ValidationError(
                "Merchant ID too long".to_string(),
            ));
        }

        // Allow alphanumeric, hyphens, and underscores
        let valid_chars = merchant_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        if !valid_chars {
            return Err(AppError::ValidationError(
                "Merchant ID contains invalid characters".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate API key format
    pub fn validate_api_key(api_key: &str) -> Result<(), AppError> {
        if api_key.is_empty() {
            return Err(AppError::ValidationError(
                "API key cannot be empty".to_string(),
            ));
        }

        if api_key.len() < 32 || api_key.len() > 128 {
            return Err(AppError::ValidationError(
                "API key length invalid".to_string(),
            ));
        }

        // API keys should be alphanumeric
        if !api_key.chars().all(|c| c.is_alphanumeric()) {
            return Err(AppError::ValidationError(
                "API key format invalid".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate network/environment combination.
    ///
    /// Supported chains for this distribution: Ethereum (ETH), BNB Smart Chain
    /// (BNB native) and Solana (SOL native). Bitcoin and USDT code paths exist
    /// in the codebase but are intentionally not exposed via the public API.
    pub fn validate_network_environment(network: &str, environment: &str) -> Result<(), AppError> {
        let valid_networks = ["ethereum", "eth", "bnb", "solana", "sol"];
        let valid_environments = ["testnet", "mainnet"];

        if !valid_networks.contains(&network.to_lowercase().as_str()) {
            return Err(AppError::ValidationError(format!(
                "Unsupported network: {}. Supported networks: ETH, BNB, SOL (native currencies only).",
                network
            )));
        }

        if !valid_environments.contains(&environment.to_lowercase().as_str()) {
            return Err(AppError::ValidationError(format!(
                "Invalid environment: {}",
                environment
            )));
        }

        Ok(())
    }

    /// Sanitize string input to prevent injection attacks
    pub fn sanitize_string(input: &str) -> String {
        // Remove potentially dangerous characters
        input
            .chars()
            .filter(|c| c.is_alphanumeric() || " .-_@".contains(*c))
            .collect::<String>()
            .trim()
            .to_string()
    }

    // Private helper methods
    fn is_valid_bitcoin_address(address: &str) -> bool {
        // Basic Bitcoin address validation (simplified)
        if address.len() < 26 || address.len() > 35 {
            return false;
        }

        // Check if it starts with valid Bitcoin prefixes
        address.starts_with('1') || address.starts_with('3') || address.starts_with("bc1")
    }

    fn is_valid_ethereum_address(address: &str) -> bool {
        // Ethereum addresses are 42 characters long and start with 0x
        if address.len() != 42 || !address.starts_with("0x") {
            return false;
        }

        // Check if the rest are valid hex characters
        address[2..].chars().all(|c| c.is_ascii_hexdigit())
    }

    fn is_valid_solana_address(address: &str) -> bool {
        // Solana addresses are base58 encoded, typically 32-44 characters
        if address.len() < 32 || address.len() > 44 {
            return false;
        }

        // Basic check for base58 characters (simplified)
        address
            .chars()
            .all(|c| "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(c))
    }
}

/// Request size limits for different endpoints
pub struct RequestLimits;

impl RequestLimits {
    pub const MAX_JSON_PAYLOAD: usize = 1024 * 1024; // 1MB
    pub const MAX_QUERY_PARAMS: usize = 50;
    pub const MAX_HEADER_VALUE: usize = 8192;

    /// Validate request payload size
    pub fn validate_payload_size(size: usize, endpoint: &str) -> Result<(), AppError> {
        let max_size = match endpoint {
            path if path.contains("/upload") => 10 * 1024 * 1024, // 10MB for uploads
            path if path.contains("/webhook") => 512 * 1024,      // 512KB for webhooks
            _ => Self::MAX_JSON_PAYLOAD,                          // 1MB default
        };

        if size > max_size {
            return Err(AppError::ValidationError(format!(
                "Request payload too large: {} bytes (max: {} bytes)",
                size, max_size
            )));
        }

        Ok(())
    }
}
