use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Network configuration for blockchain networks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub chain_id: u64,
    pub chain_name: String,
    pub native_currency: CurrencyInfo,
    pub rpc_urls: Vec<String>,
    pub block_explorer_urls: Vec<String>,
    pub network_type: NetworkType,
    pub environment_type: EnvironmentType,
    pub supports_eip681: bool, // Supports EIP-681 payment URIs
    pub wallet_support: WalletSupport,
    pub token_contract: Option<TokenContractInfo>, // For ERC-20/BEP-20 tokens
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenContractInfo {
    pub contract_address: String,
    pub token_type: TokenType, // ERC-20, BEP-20, etc.
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TokenType {
    ERC20, // Ethereum ERC-20 token
    BEP20, // BNB Chain BEP-20 token
    SPL,   // Solana SPL token
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrencyInfo {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkType {
    Ethereum,
    Bitcoin,
    Solana,
    BinanceSmartChain,
    Polygon,
    Avalanche,
    Arbitrum,
    Optimism,
    Base,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EnvironmentType {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletSupport {
    pub metamask: bool,
    pub trust_wallet: bool,
    pub coinbase_wallet: bool,
    pub phantom: bool,
    pub rainbow: bool,
}

/// Network configuration manager
pub struct NetworkConfigManager;

impl NetworkConfigManager {
    /// Get network configuration for a currency
    pub fn get_network_config(currency: &str) -> Option<NetworkConfig> {
        let configs = Self::get_all_configs();
        configs.get(&currency.to_lowercase()).cloned()
    }

    /// Get all network configurations
    pub fn get_all_configs() -> HashMap<String, NetworkConfig> {
        let mut configs = HashMap::new();

        // Ethereum Mainnet
        configs.insert(
            "ethereum".to_string(),
            NetworkConfig {
                chain_id: 1,
                chain_name: "Ethereum Mainnet".to_string(),
                native_currency: CurrencyInfo {
                    name: "Ethereum".to_string(),
                    symbol: "ETH".to_string(),
                    decimals: 18,
                },
                rpc_urls: vec![
                    "https://mainnet.infura.io/v3/eef650a32682456db1cb76fa3f4e1206".to_string(),
                    "https://eth-mainnet.alchemyapi.io/v2/".to_string(),
                    "https://cloudflare-eth.com".to_string(),
                ],
                block_explorer_urls: vec!["https://etherscan.io".to_string()],
                network_type: NetworkType::Ethereum,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: true,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("eth".to_string(), configs["ethereum"].clone());

        // USDT configurations commented out - will be used later
        // // USDT (ERC-20 on Ethereum)
        // configs.insert("usdt".to_string(), NetworkConfig {
        //     chain_id: 1,
        //     chain_name: "Ethereum Mainnet".to_string(),
        //     native_currency: CurrencyInfo {
        //         name: "Tether USD (ERC-20)".to_string(),
        //         symbol: "USDT".to_string(),
        //         decimals: 6,
        //     },
        //     rpc_urls: vec![
        //         "https://mainnet.infura.io/v3/eef650a32682456db1cb76fa3f4e1206".to_string(),
        //         "https://eth-mainnet.alchemyapi.io/v2/".to_string(),
        //         "https://cloudflare-eth.com".to_string(),
        //     ],
        //     block_explorer_urls: vec!["https://etherscan.io".to_string()],
        //     network_type: NetworkType::Ethereum,
        //     environment_type: EnvironmentType::Mainnet,
        //     supports_eip681: true, // FIXED: Enable EIP-681 support for proper QR codes
        //     wallet_support: WalletSupport {
        //         metamask: true,
        //         trust_wallet: true,
        //         coinbase_wallet: true,
        //         phantom: false,
        //         rainbow: true,
        //     },
        //     token_contract: Some(TokenContractInfo {
        //         contract_address: "0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(), // Official USDT contract on Ethereum
        //         token_type: TokenType::ERC20,
        //         decimals: 6,
        //     }),
        // });

        // configs.insert("tether".to_string(), configs["usdt"].clone());

        // // USDT (BEP-20 on BNB Chain) - USDT has 18 decimals on BNB Smart Chain
        // configs.insert("usdt_bnb".to_string(), NetworkConfig {
        //     chain_id: 56,
        //     chain_name: "BNB Smart Chain".to_string(),
        //     native_currency: CurrencyInfo {
        //         name: "Tether USD (BEP-20)".to_string(),
        //         symbol: "USDT".to_string(),
        //         decimals: 18, // FIXED: USDT on BNB Smart Chain uses 18 decimals
        //     },
        //     rpc_urls: vec![
        //         "https://bsc-dataseed.binance.org/".to_string(),
        //         "https://bsc-dataseed1.defibit.io/".to_string(),
        //         "https://bsc-dataseed1.ninicoin.io/".to_string(),
        //     ],
        //     block_explorer_urls: vec!["https://bscscan.com".to_string()],
        //     network_type: NetworkType::BinanceSmartChain,
        //     environment_type: EnvironmentType::Mainnet,
        //     supports_eip681: true, // FIXED: Enable EIP-681 support for proper QR codes
        //     wallet_support: WalletSupport {
        //         metamask: true,
        //         trust_wallet: true,
        //         coinbase_wallet: true,
        //         phantom: false,
        //         rainbow: true,
        //     },
        //     token_contract: Some(TokenContractInfo {
        //         contract_address: "0x55d398326f99059fF775485246999027B3197955".to_string(), // Official USDT contract on BNB Chain
        //         token_type: TokenType::BEP20,
        //         decimals: 18, // FIXED: USDT on BNB Smart Chain uses 18 decimals
        //     }),
        // });

        // configs.insert("usdt_bep20".to_string(), configs["usdt_bnb"].clone());
        // configs.insert("tether_bnb".to_string(), configs["usdt_bnb"].clone());

        // Test token with 18 decimals on BNB Chain (for testing purposes)
        configs.insert(
            "test18_bnb".to_string(),
            NetworkConfig {
                chain_id: 56,
                chain_name: "BNB Smart Chain".to_string(),
                native_currency: CurrencyInfo {
                    name: "Test Token 18 Decimals (BEP-20)".to_string(),
                    symbol: "TEST18".to_string(),
                    decimals: 18,
                },
                rpc_urls: vec![
                    "https://bsc-dataseed.binance.org/".to_string(),
                    "https://bsc-dataseed1.defibit.io/".to_string(),
                    "https://bsc-dataseed1.ninicoin.io/".to_string(),
                ],
                block_explorer_urls: vec!["https://bscscan.com".to_string()],
                network_type: NetworkType::BinanceSmartChain,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: true,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: Some(TokenContractInfo {
                    contract_address: "0x0000000000000000000000000000000000000001".to_string(), // Dummy contract for testing
                    token_type: TokenType::BEP20,
                    decimals: 18, // 18 decimals for testing
                }),
            },
        );

        // USDC (ERC-20 on Ethereum)
        configs.insert(
            "usdc".to_string(),
            NetworkConfig {
                chain_id: 1,
                chain_name: "Ethereum Mainnet".to_string(),
                native_currency: CurrencyInfo {
                    name: "USD Coin".to_string(),
                    symbol: "USDC".to_string(),
                    decimals: 6,
                },
                rpc_urls: vec![
                    "https://mainnet.infura.io/v3/eef650a32682456db1cb76fa3f4e1206".to_string(),
                    "https://eth-mainnet.alchemyapi.io/v2/".to_string(),
                    "https://cloudflare-eth.com".to_string(),
                ],
                block_explorer_urls: vec!["https://etherscan.io".to_string()],
                network_type: NetworkType::Ethereum,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: false,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: Some(TokenContractInfo {
                    contract_address: "0xA0b86a33E6441e213e5d42c1eEfE6f6B0c86D6a5".to_string(), // USDC contract on Ethereum
                    token_type: TokenType::ERC20,
                    decimals: 6,
                }),
            },
        );

        configs.insert("usd-coin".to_string(), configs["usdc"].clone());

        // Binance Smart Chain
        configs.insert(
            "bnb".to_string(),
            NetworkConfig {
                chain_id: 56,
                chain_name: "BNB Smart Chain".to_string(),
                native_currency: CurrencyInfo {
                    name: "BNB".to_string(),
                    symbol: "BNB".to_string(),
                    decimals: 18,
                },
                rpc_urls: vec![
                    "https://bsc-dataseed.binance.org/".to_string(),
                    "https://bsc-dataseed1.defibit.io/".to_string(),
                    "https://bsc-dataseed1.ninicoin.io/".to_string(),
                ],
                block_explorer_urls: vec!["https://bscscan.com".to_string()],
                network_type: NetworkType::BinanceSmartChain,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: true,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("binancecoin".to_string(), configs["bnb"].clone());

        // Polygon
        configs.insert(
            "matic".to_string(),
            NetworkConfig {
                chain_id: 137,
                chain_name: "Polygon Mainnet".to_string(),
                native_currency: CurrencyInfo {
                    name: "MATIC".to_string(),
                    symbol: "MATIC".to_string(),
                    decimals: 18,
                },
                rpc_urls: vec![
                    "https://polygon-rpc.com/".to_string(),
                    "https://rpc-mainnet.matic.network".to_string(),
                    "https://matic-mainnet.chainstacklabs.com".to_string(),
                ],
                block_explorer_urls: vec!["https://polygonscan.com".to_string()],
                network_type: NetworkType::Polygon,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: true,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("polygon".to_string(), configs["matic"].clone());

        // Avalanche
        configs.insert(
            "avax".to_string(),
            NetworkConfig {
                chain_id: 43114,
                chain_name: "Avalanche C-Chain".to_string(),
                native_currency: CurrencyInfo {
                    name: "Avalanche".to_string(),
                    symbol: "AVAX".to_string(),
                    decimals: 18,
                },
                rpc_urls: vec![
                    "https://api.avax.network/ext/bc/C/rpc".to_string(),
                    "https://rpc.ankr.com/avalanche".to_string(),
                ],
                block_explorer_urls: vec!["https://snowtrace.io".to_string()],
                network_type: NetworkType::Avalanche,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: true,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("avalanche".to_string(), configs["avax"].clone());

        // Bitcoin (no chain ID - different protocol)
        configs.insert(
            "bitcoin".to_string(),
            NetworkConfig {
                chain_id: 0, // Bitcoin doesn't use chain IDs
                chain_name: "Bitcoin".to_string(),
                native_currency: CurrencyInfo {
                    name: "Bitcoin".to_string(),
                    symbol: "BTC".to_string(),
                    decimals: 8,
                },
                rpc_urls: vec![], // Bitcoin uses different RPC structure
                block_explorer_urls: vec!["https://blockstream.info".to_string()],
                network_type: NetworkType::Bitcoin,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: false, // Uses BIP21 instead
                wallet_support: WalletSupport {
                    metamask: false,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: false,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("btc".to_string(), configs["bitcoin"].clone());

        // Solana (no chain ID - different protocol)
        configs.insert(
            "solana".to_string(),
            NetworkConfig {
                chain_id: 0, // Solana doesn't use chain IDs
                chain_name: "Solana Mainnet".to_string(),
                native_currency: CurrencyInfo {
                    name: "Solana".to_string(),
                    symbol: "SOL".to_string(),
                    decimals: 9,
                },
                rpc_urls: vec![
                    "https://api.mainnet-beta.solana.com".to_string(),
                    "https://solana-api.projectserum.com".to_string(),
                ],
                block_explorer_urls: vec!["https://solscan.io".to_string()],
                network_type: NetworkType::Solana,
                environment_type: EnvironmentType::Mainnet,
                supports_eip681: false, // Uses Solana Pay instead
                wallet_support: WalletSupport {
                    metamask: false,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: true,
                    rainbow: false,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("sol".to_string(), configs["solana"].clone());

        // Base Sepolia Testnet
        configs.insert(
            "base_sepolia".to_string(),
            NetworkConfig {
                chain_id: 84532,
                chain_name: "Base Sepolia Testnet".to_string(),
                native_currency: CurrencyInfo {
                    name: "Ethereum".to_string(),
                    symbol: "ETH".to_string(),
                    decimals: 18,
                },
                rpc_urls: vec![
                    "https://sepolia.base.org".to_string(), // Primary: Official Base Sepolia RPC
                    "https://base-sepolia.public.blastapi.io".to_string(),
                    "https://base-sepolia.blockpi.network/v1/rpc/public".to_string(),
                ],
                block_explorer_urls: vec!["https://sepolia-explorer.base.org".to_string()],
                network_type: NetworkType::Base,
                environment_type: EnvironmentType::Testnet,
                supports_eip681: true,
                wallet_support: WalletSupport {
                    metamask: true,
                    trust_wallet: true,
                    coinbase_wallet: true,
                    phantom: false,
                    rainbow: true,
                },
                token_contract: None, // Native currency
            },
        );

        configs.insert("base".to_string(), configs["base_sepolia"].clone());
        configs.insert("testnet".to_string(), configs["base_sepolia"].clone());

        configs
    }

    /// Check if a currency is supported
    pub fn is_supported(currency: &str) -> bool {
        Self::get_network_config(currency).is_some()
    }

    /// Get network configuration for a currency with environment filter
    pub fn get_network_config_by_environment(
        currency: &str,
        environment: EnvironmentType,
    ) -> Option<NetworkConfig> {
        let config = Self::get_network_config(currency)?;
        if config.environment_type == environment {
            Some(config)
        } else {
            None
        }
    }

    /// Get all networks for a specific environment
    pub fn get_networks_by_environment(
        environment: EnvironmentType,
    ) -> HashMap<String, NetworkConfig> {
        Self::get_all_configs()
            .into_iter()
            .filter(|(_, config)| config.environment_type == environment)
            .collect()
    }

    /// Get supported currencies for a specific environment
    pub fn get_supported_currencies_by_environment(environment: EnvironmentType) -> Vec<String> {
        Self::get_networks_by_environment(environment)
            .keys()
            .cloned()
            .collect()
    }

    /// Get network setup instructions for manual configuration
    pub fn get_network_setup_instructions(currency: &str) -> Option<NetworkSetupInstructions> {
        let config = Self::get_network_config(currency)?;
        let instructions = Self::generate_setup_instructions(&config);

        Some(NetworkSetupInstructions {
            network_name: config.chain_name,
            chain_id: config.chain_id,
            currency_symbol: config.native_currency.symbol,
            rpc_urls: config.rpc_urls,
            block_explorer_urls: config.block_explorer_urls,
            instructions,
        })
    }

    fn generate_setup_instructions(config: &NetworkConfig) -> Vec<String> {
        let mut instructions = Vec::new();

        match config.network_type {
            NetworkType::Bitcoin | NetworkType::Solana => {
                instructions
                    .push("This network doesn't require manual setup in most wallets.".to_string());
                instructions.push("Use a compatible wallet app for this currency.".to_string());
            }
            _ => {
                instructions.push("To add this network to MetaMask:".to_string());
                instructions.push("1. Open MetaMask and click the network dropdown".to_string());
                instructions.push("2. Click 'Add Network' or 'Custom RPC'".to_string());
                instructions.push("3. Enter the network details shown above".to_string());
                instructions.push("4. Click 'Save' to add the network".to_string());
                instructions.push("5. Switch to the new network and try again".to_string());
            }
        }

        instructions
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSetupInstructions {
    pub network_name: String,
    pub chain_id: u64,
    pub currency_symbol: String,
    pub rpc_urls: Vec<String>,
    pub block_explorer_urls: Vec<String>,
    pub instructions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ethereum_config() {
        let config = NetworkConfigManager::get_network_config("ethereum").unwrap();
        assert_eq!(config.chain_id, 1);
        assert_eq!(config.chain_name, "Ethereum Mainnet");
        assert_eq!(config.environment_type, EnvironmentType::Mainnet);
        assert!(config.supports_eip681);
    }

    #[test]
    fn test_bsc_config() {
        let config = NetworkConfigManager::get_network_config("bnb").unwrap();
        assert_eq!(config.chain_id, 56);
        assert_eq!(config.chain_name, "BNB Smart Chain");
        assert_eq!(config.environment_type, EnvironmentType::Mainnet);
        assert!(config.supports_eip681);
    }

    #[test]
    fn test_bitcoin_config() {
        let config = NetworkConfigManager::get_network_config("bitcoin").unwrap();
        assert_eq!(config.chain_id, 0);
        assert_eq!(config.environment_type, EnvironmentType::Mainnet);
        assert!(!config.supports_eip681);
        assert!(matches!(config.network_type, NetworkType::Bitcoin));
    }

    #[test]
    fn test_base_sepolia_config() {
        let config = NetworkConfigManager::get_network_config("base_sepolia").unwrap();
        assert_eq!(config.chain_id, 84532);
        assert_eq!(config.chain_name, "Base Sepolia Testnet");
        assert_eq!(config.environment_type, EnvironmentType::Testnet);
        assert!(config.supports_eip681);
        assert!(matches!(config.network_type, NetworkType::Base));
    }

    #[test]
    fn test_environment_filtering() {
        let mainnet_configs =
            NetworkConfigManager::get_networks_by_environment(EnvironmentType::Mainnet);
        let testnet_configs =
            NetworkConfigManager::get_networks_by_environment(EnvironmentType::Testnet);

        assert!(mainnet_configs.len() > 0);
        assert!(testnet_configs.len() > 0);
        assert!(mainnet_configs.contains_key("ethereum"));
        assert!(testnet_configs.contains_key("base_sepolia"));
    }

    #[test]
    fn test_unsupported_currency() {
        let config = NetworkConfigManager::get_network_config("unknown");
        assert!(config.is_none());
    }

    #[test]
    fn test_setup_instructions() {
        let instructions = NetworkConfigManager::get_network_setup_instructions("bnb").unwrap();
        assert_eq!(instructions.chain_id, 56);
        assert!(instructions.instructions.len() > 0);
    }
}
