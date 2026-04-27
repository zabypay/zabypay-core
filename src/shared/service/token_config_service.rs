use crate::shared::utils::errors::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenConfig {
    pub network: String,
    pub contract_address: String,
    pub decimals: u8,
    pub chain_id: u64,
    pub rpc_url: String,
    pub explorer_base_url: String,
    pub gas_limit: u64,
    pub required_confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasSponsorConfig {
    pub network: String,
    pub environment: String,
    pub sponsor_address: String,
    pub sponsor_private_key: String, // Should be encrypted in production
    pub min_topup_amount: String,
    pub safety_buffer_bps: u16, // Basis points (e.g., 500 = 5%)
}

pub struct TokenConfigService;

impl TokenConfigService {
    /// Get token configuration dynamically (from DB/env, not hardcoded)
    pub fn get_token_config(currency: &str, environment: &str) -> Result<TokenConfig, AppError> {
        // This should eventually load from database, but for now use environment variables
        // with fallback to reasonable defaults
        match currency.to_lowercase().as_str() {
            "usdt_erc20" => {
                let rpc_url = match environment {
                    "mainnet" => std::env::var("ETH_MAINNET_RPC").unwrap_or_else(|_| {
                        "https://mainnet.infura.io/v3/eef650a32682456db1cb76fa3f4e1206".to_string()
                    }),
                    "testnet" => std::env::var("ETH_TESTNET_RPC").unwrap_or_else(|_| {
                        "https://sepolia.infura.io/v3/eef650a32682456db1cb76fa3f4e1206".to_string()
                    }),
                    _ => {
                        return Err(AppError::ValidationError(format!(
                            "Unsupported environment: {}",
                            environment
                        )))
                    }
                };

                let (chain_id, contract_address, explorer_url) = match environment {
                    "mainnet" => (
                        1u64,
                        "0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(), // Official USDT contract
                        "https://etherscan.io".to_string(),
                    ),
                    "testnet" => (
                        11155111u64,                                              // Sepolia
                        "0x7169D38820dfd117C3FA1f22a697dBA58d90BA06".to_string(), // USDT test contract
                        "https://sepolia.etherscan.io".to_string(),
                    ),
                    _ => {
                        return Err(AppError::ValidationError(format!(
                            "Unsupported environment: {}",
                            environment
                        )))
                    }
                };

                Ok(TokenConfig {
                    network: "ethereum".to_string(),
                    contract_address,
                    decimals: 6, // CRITICAL: USDT ERC-20 has 6 decimals
                    chain_id,
                    rpc_url,
                    explorer_base_url: explorer_url,
                    gas_limit: 60000, // ERC-20 transfer needs more gas than native
                    required_confirmations: if environment == "mainnet" { 12 } else { 1 },
                })
            }
            "usdt_bep20" => {
                let rpc_url = match environment {
                    "mainnet" => std::env::var("BSC_MAINNET_RPC")
                        .unwrap_or_else(|_| "https://bsc-dataseed.binance.org/".to_string()),
                    "testnet" => std::env::var("BSC_TESTNET_RPC").unwrap_or_else(|_| {
                        "https://data-seed-prebsc-1-s1.binance.org:8545".to_string()
                    }),
                    _ => {
                        return Err(AppError::ValidationError(format!(
                            "Unsupported environment: {}",
                            environment
                        )))
                    }
                };

                let (chain_id, contract_address, explorer_url) = match environment {
                    "mainnet" => (
                        56u64,
                        "0x55d398326f99059fF775485246999027B3197955".to_string(), // Official USDT contract on BSC
                        "https://bscscan.com".to_string(),
                    ),
                    "testnet" => (
                        97u64,                                                    // BSC Testnet
                        "0x7ef95a0FEE0Dd31b22626fA2e10Ee6A223F8a684".to_string(), // USDT test contract on BSC testnet
                        "https://testnet.bscscan.com".to_string(),
                    ),
                    _ => {
                        return Err(AppError::ValidationError(format!(
                            "Unsupported environment: {}",
                            environment
                        )))
                    }
                };

                Ok(TokenConfig {
                    network: "bsc".to_string(),
                    contract_address,
                    decimals: 18, // CRITICAL: USDT BEP-20 has 18 decimals
                    chain_id,
                    rpc_url,
                    explorer_base_url: explorer_url,
                    gas_limit: 60000, // BEP-20 transfer
                    required_confirmations: if environment == "mainnet" { 3 } else { 1 },
                })
            }
            _ => Err(AppError::ValidationError(format!(
                "Unsupported token currency: {}",
                currency
            ))),
        }
    }

    /// Get gas sponsor configuration for a network
    pub fn get_gas_sponsor_config(
        network: &str,
        environment: &str,
    ) -> Result<GasSponsorConfig, AppError> {
        // In production, this should load from encrypted database storage
        // For now, use environment variables with secure fallbacks

        let env_key_prefix = format!(
            "{}_{}_SPONSOR",
            network.to_uppercase(),
            environment.to_uppercase()
        );

        let sponsor_address =
            std::env::var(format!("{}_ADDRESS", env_key_prefix)).map_err(|_| {
                AppError::InternalServerError(format!(
                    "Gas sponsor address not configured for {} {}",
                    network, environment
                ))
            })?;

        let sponsor_private_key = std::env::var(format!("{}_PRIVATE_KEY", env_key_prefix))
            .map_err(|_| {
                AppError::InternalServerError(format!(
                    "Gas sponsor private key not configured for {} {}",
                    network, environment
                ))
            })?;

        // Default minimum top-up amounts (in base units)
        let min_topup_amount = match network {
            "ethereum" => match environment {
                "mainnet" => "10000000000000000".to_string(), // 0.01 ETH
                "testnet" => "1000000000000000".to_string(),  // 0.001 ETH
                _ => "1000000000000000".to_string(),
            },
            "bsc" => match environment {
                "mainnet" => "10000000000000000".to_string(), // 0.01 BNB
                "testnet" => "1000000000000000".to_string(),  // 0.001 BNB
                _ => "1000000000000000".to_string(),
            },
            _ => {
                return Err(AppError::ValidationError(format!(
                    "Unsupported sponsor network: {}",
                    network
                )))
            }
        };

        Ok(GasSponsorConfig {
            network: network.to_string(),
            environment: environment.to_string(),
            sponsor_address,
            sponsor_private_key,
            min_topup_amount,
            safety_buffer_bps: 500, // 5% safety buffer
        })
    }

    /// Convert amount to base units using token decimals
    pub fn amount_to_base_units(
        amount_str: &str,
        decimals: u8,
        amount_type: &str,
    ) -> Result<String, AppError> {
        use rust_decimal::Decimal;
        use std::str::FromStr;

        let amount_decimal = Decimal::from_str(amount_str)
            .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

        if amount_decimal <= Decimal::ZERO {
            return Err(AppError::ValidationError(
                "Amount must be greater than zero".to_string(),
            ));
        }

        // For USD amounts, we'd need to convert via price oracle first
        // For now, assume crypto amounts
        if amount_type != "crypto" {
            return Err(AppError::ValidationError(
                "USD amount type not yet implemented".to_string(),
            ));
        }

        // Convert to base units: amount * 10^decimals
        let multiplier = Decimal::from(10u64.pow(decimals as u32));
        let base_units = (amount_decimal * multiplier).floor();

        // Convert to string without decimal places
        let base_units_str = base_units.to_string();
        if base_units_str.contains('.') {
            Ok(base_units_str.split('.').next().unwrap().to_string())
        } else {
            Ok(base_units_str)
        }
    }

    /// Convert base units back to display amount
    pub fn base_units_to_amount(base_units_str: &str, decimals: u8) -> Result<String, AppError> {
        use rust_decimal::Decimal;
        use std::str::FromStr;

        let base_units = Decimal::from_str(base_units_str)
            .map_err(|_| AppError::ValidationError("Invalid base units format".to_string()))?;

        let divisor = Decimal::from(10u64.pow(decimals as u32));
        let amount = base_units / divisor;

        Ok(amount.to_string())
    }

    /// Validate that CRYPTO_ENCRYPTION_KEY is available
    pub fn validate_encryption_key() -> Result<(), AppError> {
        std::env::var("CRYPTO_ENCRYPTION_KEY")
            .map_err(|_| AppError::InternalServerError(
                "CRYPTO_ENCRYPTION_KEY environment variable is not set. Cannot decrypt wallet private keys.".to_string()
            ))?;
        Ok(())
    }

    /// Get supported token currencies
    pub fn get_supported_token_currencies() -> Vec<String> {
        vec!["usdt_erc20".to_string(), "usdt_bep20".to_string()]
    }

    /// Check if currency is a supported token
    pub fn is_supported_token(currency: &str) -> bool {
        Self::get_supported_token_currencies().contains(&currency.to_lowercase())
    }

    /// Map network names to currency codes
    pub fn map_network_to_currency(network: &str) -> Result<String, AppError> {
        match network.to_lowercase().as_str() {
            "usdt_erc20" => Ok("usdt_erc20".to_string()),
            "usdt_bep20" => Ok("usdt_bep20".to_string()),
            _ => Err(AppError::ValidationError(format!(
                "Unsupported token network: {}",
                network
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amount_to_base_units_erc20() {
        // USDT ERC-20 has 6 decimals
        let result = TokenConfigService::amount_to_base_units("1.5", 6, "crypto").unwrap();
        assert_eq!(result, "1500000"); // 1.5 * 10^6
    }

    #[test]
    fn test_amount_to_base_units_bep20() {
        // USDT BEP-20 has 18 decimals
        let result = TokenConfigService::amount_to_base_units("1.5", 18, "crypto").unwrap();
        assert_eq!(result, "1500000000000000000"); // 1.5 * 10^18
    }

    #[test]
    fn test_base_units_to_amount() {
        let result = TokenConfigService::base_units_to_amount("1500000", 6).unwrap();
        assert_eq!(result, "1.5");
    }

    #[test]
    fn test_is_supported_token() {
        assert!(TokenConfigService::is_supported_token("usdt_erc20"));
        assert!(TokenConfigService::is_supported_token("USDT_BEP20"));
        assert!(!TokenConfigService::is_supported_token("btc"));
    }
}
