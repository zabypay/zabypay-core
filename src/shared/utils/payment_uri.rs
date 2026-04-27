use super::network_config::{NetworkConfigManager, NetworkSetupInstructions};
use rust_decimal::Decimal;
use sha3::{Digest, Keccak256};
use std::str::FromStr;
use urlencoding;

/// Payment URI generator for various cryptocurrencies
/// Implements standards like BIP21 (Bitcoin), EIP-681 (Ethereum), and Solana Pay
pub struct PaymentUriGenerator;

impl PaymentUriGenerator {
    /// Convert an Ethereum address to proper EIP-55 checksum format
    /// This ensures wallet compatibility by using the standard checksummed format
    fn to_checksum_address(address: &str) -> String {
        let address = address.trim_start_matches("0x").to_lowercase();
        let hash = Keccak256::digest(address.as_bytes());
        let hash_hex = hex::encode(hash);

        let mut checksummed = String::with_capacity(42);
        checksummed.push_str("0x");

        for (i, c) in address.chars().enumerate() {
            if c.is_ascii_digit() {
                checksummed.push(c);
            } else {
                // Get the corresponding hash character
                let hash_char = hash_hex.chars().nth(i).unwrap_or('0');
                let hash_value = u8::from_str_radix(&hash_char.to_string(), 16).unwrap_or(0);

                if hash_value >= 8 {
                    checksummed.push(c.to_ascii_uppercase());
                } else {
                    checksummed.push(c);
                }
            }
        }

        checksummed
    }
    /// Generate a payment URI based on the currency type
    pub fn generate_uri(
        currency: &str,
        address: &str,
        amount: &str,
        payment_id: &str,
        external_id: &str,
    ) -> Result<String, String> {
        Self::generate_uri_with_environment(
            currency,
            address,
            amount,
            payment_id,
            external_id,
            None,
        )
    }

    /// Generate a payment URI with environment-specific configuration
    pub fn generate_uri_with_environment(
        currency: &str,
        address: &str,
        amount: &str,
        payment_id: &str,
        external_id: &str,
        environment: Option<&str>,
    ) -> Result<String, String> {
        match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => {
                Self::generate_bitcoin_uri(address, amount, payment_id, external_id)
            }
            "ethereum" | "eth" => {
                if environment == Some("testnet") {
                    Self::generate_base_sepolia_uri(address, amount)
                } else {
                    Self::generate_ethereum_uri(address, amount)
                }
            }
            "usdt" | "tether" => Self::generate_erc20_uri(address, amount, "usdt"),
            "usdt_bnb" | "usdt_bep20" | "tether_bnb" => {
                Self::generate_bep20_uri(address, amount, "usdt_bnb")
            }
            "usdc" | "usd-coin" => Self::generate_erc20_uri(address, amount, "usdc"),
            "solana" | "sol" => Self::generate_solana_uri(address, amount, payment_id, external_id),
            "bnb" | "binancecoin" => Self::generate_bnb_uri(address, amount),
            "matic" | "polygon" => Self::generate_polygon_uri(address, amount),
            "avax" | "avalanche" => Self::generate_avalanche_uri(address, amount),
            _ => Ok(address.to_string()), // Fallback to address only
        }
    }

    /// Generate Bitcoin URI following BIP21 standard
    /// Format: bitcoin:<address>?amount=<amount>&label=<label>&message=<message>
    fn generate_bitcoin_uri(
        address: &str,
        amount: &str,
        payment_id: &str,
        external_id: &str,
    ) -> Result<String, String> {
        // Validate Bitcoin amount (positive number with up to 8 decimal places)
        let btc_amount =
            Decimal::from_str(amount).map_err(|_| "Invalid Bitcoin amount".to_string())?;

        if btc_amount <= Decimal::ZERO {
            return Err("Bitcoin amount must be positive".to_string());
        }

        let label_str = format!("Payment {}", payment_id);
        let message_str = format!("Order {}", external_id);
        let label = urlencoding::encode(&label_str);
        let message = urlencoding::encode(&message_str);

        Ok(format!(
            "bitcoin:{}?amount={}&label={}&message={}",
            address, amount, label, message
        ))
    }

    /// Generate Ethereum URI following EIP-681 standard with chain ID
    /// Format: ethereum:<address>@<chain_id>?value=<amount_in_wei>
    fn generate_ethereum_uri(address: &str, eth_amount: &str) -> Result<String, String> {
        // Get network configuration
        let network_config = NetworkConfigManager::get_network_config("ethereum")
            .ok_or("Ethereum network configuration not found")?;

        // Convert ETH to Wei (1 ETH = 10^18 Wei)
        let eth =
            Decimal::from_str(eth_amount).map_err(|_| "Invalid Ethereum amount".to_string())?;

        if eth <= Decimal::ZERO {
            return Err("Ethereum amount must be positive".to_string());
        }

        // Convert to Wei (multiply by 10^18)
        let wei_multiplier = Decimal::from_str("1000000000000000000").expect("Valid decimal");
        let wei = eth * wei_multiplier;

        // Remove decimal places for Wei (must be integer)
        let wei_str = wei.trunc().to_string();

        // Include chain ID for wallet compatibility
        Ok(format!(
            "ethereum:{}@{}?value={}",
            address, network_config.chain_id, wei_str
        ))
    }

    /// Generate ERC-20 token URI following EIP-681 standard for token transfers
    /// Format: ethereum:<recipient_address>@<chain_id>/transfer?address=<token_contract>&uint256=<amount_in_token_units>
    fn generate_erc20_uri(address: &str, amount: &str, token: &str) -> Result<String, String> {
        // Get network configuration for the token
        let network_config = NetworkConfigManager::get_network_config(token)
            .ok_or("Token network configuration not found")?;

        let token_contract = network_config
            .token_contract
            .ok_or("Token contract information not found")?;

        // Parse amount and convert to token base units (considering decimals)
        let token_amount =
            Decimal::from_str(amount).map_err(|_| "Invalid token amount".to_string())?;

        if token_amount <= Decimal::ZERO {
            return Err("Token amount must be positive".to_string());
        }

        // Convert to token base units using the token's decimals
        let multiplier = Decimal::from_str(&format!(
            "1{}",
            "0".repeat(token_contract.decimals as usize)
        ))
        .map_err(|_| "Failed to calculate token decimals multiplier")?;
        let base_units = token_amount * multiplier;
        let base_units_str = base_units.trunc().to_string();

        // Generate EIP-681 compliant token transfer URI with checksummed addresses
        // IMPORTANT: For ERC-20 tokens, the format is:
        // ethereum:<token_contract>@<chainId>/transfer?address=<recipient>&uint256=<amount>
        // The token contract address comes FIRST, not the recipient!
        let checksummed_recipient = Self::to_checksum_address(address);
        let checksummed_contract = Self::to_checksum_address(&token_contract.contract_address);

        let uri = format!(
            "ethereum:{}@{}/transfer?address={}&uint256={}",
            checksummed_contract, // Token contract comes FIRST
            network_config.chain_id,
            checksummed_recipient, // Recipient is in the query params
            base_units_str
        );

        log::info!(
            "Generated ERC-20 URI for {}: {} (contract: {}, amount: {} tokens = {} base units, decimals: {})",
            token, uri, token_contract.contract_address, amount, base_units_str, token_contract.decimals
        );

        Ok(uri)
    }

    /// Generate BEP-20 token URI following EIP-681 standard for token transfers on BNB Chain
    /// Format: ethereum:<recipient_address>@<chain_id>/transfer?address=<token_contract>&uint256=<amount_in_token_units>
    fn generate_bep20_uri(address: &str, amount: &str, token: &str) -> Result<String, String> {
        // Get network configuration for the BEP-20 token
        let network_config = NetworkConfigManager::get_network_config(token)
            .ok_or("BEP-20 token network configuration not found")?;

        let token_contract = network_config
            .token_contract
            .ok_or("BEP-20 token contract information not found")?;

        // Parse amount and convert to token base units (considering decimals)
        let token_amount =
            Decimal::from_str(amount).map_err(|_| "Invalid BEP-20 token amount".to_string())?;

        if token_amount <= Decimal::ZERO {
            return Err("BEP-20 token amount must be positive".to_string());
        }

        // Convert to token base units using the token's decimals
        let multiplier = Decimal::from_str(&format!(
            "1{}",
            "0".repeat(token_contract.decimals as usize)
        ))
        .map_err(|_| "Failed to calculate BEP-20 token decimals multiplier")?;
        let base_units = token_amount * multiplier;
        let base_units_str = base_units.trunc().to_string();

        // Generate EIP-681 compliant token transfer URI (BNB Chain uses ethereum: scheme) with checksummed addresses
        // IMPORTANT: For BEP-20 tokens, the format is:
        // ethereum:<token_contract>@<chainId>/transfer?address=<recipient>&uint256=<amount>
        // The token contract address comes FIRST, not the recipient!
        let checksummed_recipient = Self::to_checksum_address(address);
        let checksummed_contract = Self::to_checksum_address(&token_contract.contract_address);

        let uri = format!(
            "ethereum:{}@{}/transfer?address={}&uint256={}",
            checksummed_contract, // Token contract comes FIRST
            network_config.chain_id,
            checksummed_recipient, // Recipient is in the query params
            base_units_str
        );

        log::info!(
            "Generated BEP-20 URI for {}: {} (contract: {}, amount: {} tokens = {} base units, decimals: {})",
            token, uri, token_contract.contract_address, amount, base_units_str, token_contract.decimals
        );

        Ok(uri)
    }

    /// Generate Solana URI following Solana Pay standard
    /// Format: solana:<address>?amount=<amount>&label=<label>&message=<message>
    fn generate_solana_uri(
        address: &str,
        amount: &str,
        payment_id: &str,
        external_id: &str,
    ) -> Result<String, String> {
        // Validate Solana amount
        let sol_amount =
            Decimal::from_str(amount).map_err(|_| "Invalid Solana amount".to_string())?;

        if sol_amount <= Decimal::ZERO {
            return Err("Solana amount must be positive".to_string());
        }

        let label_str = format!("Payment {}", payment_id);
        let message_str = format!("Order {}", external_id);
        let label = urlencoding::encode(&label_str);
        let message = urlencoding::encode(&message_str);

        Ok(format!(
            "solana:{}?amount={}&label={}&message={}",
            address, amount, label, message
        ))
    }

    /// Generate BNB URI (similar to Ethereum but with BSC chain ID)
    /// Format: ethereum:<address>@<chain_id>?value=<amount_in_wei>
    fn generate_bnb_uri(address: &str, bnb_amount: &str) -> Result<String, String> {
        // Get BSC network configuration
        let network_config = NetworkConfigManager::get_network_config("bnb")
            .ok_or("BNB network configuration not found")?;

        // Convert BNB to Wei (1 BNB = 10^18 Wei)
        let bnb = Decimal::from_str(bnb_amount).map_err(|_| "Invalid BNB amount".to_string())?;

        if bnb <= Decimal::ZERO {
            return Err("BNB amount must be positive".to_string());
        }

        // Convert to Wei
        let wei_multiplier = Decimal::from_str("1000000000000000000").expect("Valid decimal");
        let wei = bnb * wei_multiplier;
        let wei_str = wei.trunc().to_string();

        // Include BSC chain ID (56) for wallet compatibility
        Ok(format!(
            "ethereum:{}@{}?value={}",
            address, network_config.chain_id, wei_str
        ))
    }

    /// Generate Polygon URI (similar to Ethereum but with Polygon chain ID)
    /// Format: ethereum:<address>@<chain_id>?value=<amount_in_wei>
    fn generate_polygon_uri(address: &str, matic_amount: &str) -> Result<String, String> {
        // Get Polygon network configuration
        let network_config = NetworkConfigManager::get_network_config("matic")
            .ok_or("Polygon network configuration not found")?;

        // Convert MATIC to Wei (1 MATIC = 10^18 Wei)
        let matic =
            Decimal::from_str(matic_amount).map_err(|_| "Invalid MATIC amount".to_string())?;

        if matic <= Decimal::ZERO {
            return Err("MATIC amount must be positive".to_string());
        }

        // Convert to Wei
        let wei_multiplier = Decimal::from_str("1000000000000000000").expect("Valid decimal");
        let wei = matic * wei_multiplier;
        let wei_str = wei.trunc().to_string();

        // Include Polygon chain ID (137) for wallet compatibility
        Ok(format!(
            "ethereum:{}@{}?value={}",
            address, network_config.chain_id, wei_str
        ))
    }

    /// Generate Avalanche URI (similar to Ethereum but with Avalanche chain ID)
    /// Format: ethereum:<address>@<chain_id>?value=<amount_in_wei>
    fn generate_avalanche_uri(address: &str, avax_amount: &str) -> Result<String, String> {
        // Get Avalanche network configuration
        let network_config = NetworkConfigManager::get_network_config("avax")
            .ok_or("Avalanche network configuration not found")?;

        // Convert AVAX to Wei (1 AVAX = 10^18 Wei)
        let avax = Decimal::from_str(avax_amount).map_err(|_| "Invalid AVAX amount".to_string())?;

        if avax <= Decimal::ZERO {
            return Err("AVAX amount must be positive".to_string());
        }

        // Convert to Wei
        let wei_multiplier = Decimal::from_str("1000000000000000000").expect("Valid decimal");
        let wei = avax * wei_multiplier;
        let wei_str = wei.trunc().to_string();

        // Include Avalanche chain ID (43114) for wallet compatibility
        Ok(format!(
            "ethereum:{}@{}?value={}",
            address, network_config.chain_id, wei_str
        ))
    }

    /// Generate Base Sepolia testnet URI (similar to Ethereum but with Base Sepolia chain ID)
    /// Format: ethereum:<address>@<chain_id>?value=<amount_in_wei>
    fn generate_base_sepolia_uri(address: &str, eth_amount: &str) -> Result<String, String> {
        // Get Base Sepolia network configuration
        let network_config = NetworkConfigManager::get_network_config("base_sepolia")
            .ok_or("Base Sepolia network configuration not found")?;

        // Convert ETH to Wei (1 ETH = 10^18 Wei)
        let eth =
            Decimal::from_str(eth_amount).map_err(|_| "Invalid Ethereum amount".to_string())?;

        if eth <= Decimal::ZERO {
            return Err("Ethereum amount must be positive".to_string());
        }

        // Convert to Wei (multiply by 10^18)
        let wei_multiplier = Decimal::from_str("1000000000000000000").expect("Valid decimal");
        let wei = eth * wei_multiplier;

        // Remove decimal places for Wei (must be integer)
        let wei_str = wei.trunc().to_string();

        // Include Base Sepolia chain ID (84532) for wallet compatibility
        Ok(format!(
            "ethereum:{}@{}?value={}",
            address, network_config.chain_id, wei_str
        ))
    }

    /// Get QR code metadata for a payment with network setup instructions
    pub fn get_qr_metadata(
        currency: &str,
        address: &str,
        amount: &str,
        payment_id: &str,
        external_id: &str,
    ) -> QrMetadata {
        Self::get_qr_metadata_with_environment(
            currency,
            address,
            amount,
            payment_id,
            external_id,
            None,
        )
    }

    /// Get QR code metadata for a payment with environment-specific network configuration
    pub fn get_qr_metadata_with_environment(
        currency: &str,
        address: &str,
        amount: &str,
        payment_id: &str,
        external_id: &str,
        environment: Option<&str>,
    ) -> QrMetadata {
        let payment_uri = Self::generate_uri_with_environment(
            currency,
            address,
            amount,
            payment_id,
            external_id,
            environment,
        )
        .unwrap_or_else(|_| address.to_string());

        let (network_config, setup_instructions) = if environment == Some("testnet") {
            // For testnet, get Base Sepolia configuration
            let testnet_config = NetworkConfigManager::get_network_config("base_sepolia");
            let testnet_instructions =
                NetworkConfigManager::get_network_setup_instructions("base_sepolia");
            (testnet_config, testnet_instructions)
        } else {
            // For mainnet or unspecified, use regular currency configuration
            let config = NetworkConfigManager::get_network_config(currency);
            let instructions = NetworkConfigManager::get_network_setup_instructions(currency);
            (config, instructions)
        };

        // Extract token information for debugging and display
        let (token_contract, token_decimals, amount_in_base_units) =
            Self::get_token_metadata_with_environment(currency, amount, environment);

        // Enhanced logging for QR metadata generation
        log::info!(
            "🔗 Generated QR metadata for {} payment: currency={}, amount={}, network={}, chain_id={:?}, token_contract={:?}, token_decimals={:?}, amount_in_base_units={:?}, environment={:?}", 
            currency.to_uppercase(),
            currency,
            amount,
            Self::get_network_name_with_environment(currency, environment),
            network_config.as_ref().map(|c| c.chain_id),
            token_contract,
            token_decimals,
            amount_in_base_units,
            environment
        );

        log::info!(
            "📱 Payment URI generated: {} (type: {}, wallet_support: {:?})",
            payment_uri,
            Self::get_uri_type_with_environment(currency, environment),
            Self::get_wallet_compatibility_with_environment(currency, environment)
        );

        QrMetadata {
            payment_uri,
            currency: currency.to_string(),
            amount: amount.to_string(),
            address: address.to_string(),
            network: Self::get_network_name_with_environment(currency, environment),
            uri_type: Self::get_uri_type_with_environment(currency, environment),
            chain_id: network_config.as_ref().map(|c| c.chain_id),
            network_config,
            setup_instructions,
            wallet_compatibility: Self::get_wallet_compatibility_with_environment(
                currency,
                environment,
            ),
            token_contract,
            token_decimals,
            amount_in_base_units,
        }
    }

    /// Get the network name for a currency
    fn get_network_name(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => "Bitcoin".to_string(),
            "ethereum" | "eth" => "Ethereum".to_string(),
            "usdt" | "tether" => "Ethereum (ERC-20)".to_string(),
            "usdt_bnb" | "usdt_bep20" | "tether_bnb" => "BNB Smart Chain (BEP-20)".to_string(),
            "usdc" | "usd-coin" => "Ethereum (ERC-20)".to_string(),
            "solana" | "sol" => "Solana".to_string(),
            "bnb" | "binancecoin" => "BNB Smart Chain".to_string(),
            _ => format!("{} Network", currency.to_uppercase()),
        }
    }

    /// Get the URI type/standard for a currency
    fn get_uri_type(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => "BIP21".to_string(),
            "ethereum" | "eth" => "EIP-681".to_string(),
            "usdt" | "tether" => "ERC-20".to_string(),
            "usdt_bnb" | "usdt_bep20" | "tether_bnb" => "BEP-20".to_string(),
            "usdc" | "usd-coin" => "ERC-20".to_string(),
            "solana" | "sol" => "Solana Pay".to_string(),
            "bnb" | "binancecoin" => "BSC".to_string(),
            "matic" | "polygon" => "Polygon".to_string(),
            "avax" | "avalanche" => "Avalanche".to_string(),
            _ => "Address Only".to_string(),
        }
    }

    /// Get wallet compatibility information for a currency
    fn get_wallet_compatibility(currency: &str) -> Option<super::network_config::WalletSupport> {
        NetworkConfigManager::get_network_config(currency).map(|config| config.wallet_support)
    }

    /// Get the network name for a currency with environment context
    fn get_network_name_with_environment(currency: &str, environment: Option<&str>) -> String {
        if environment == Some("testnet") {
            "Base Sepolia Testnet".to_string()
        } else {
            Self::get_network_name(currency)
        }
    }

    /// Get the URI type/standard for a currency with environment context
    fn get_uri_type_with_environment(currency: &str, environment: Option<&str>) -> String {
        if environment == Some("testnet") && currency.to_lowercase() == "eth" {
            "EIP-681 (Base Sepolia)".to_string()
        } else {
            Self::get_uri_type(currency)
        }
    }

    /// Get wallet compatibility information for a currency with environment context
    fn get_wallet_compatibility_with_environment(
        currency: &str,
        environment: Option<&str>,
    ) -> Option<super::network_config::WalletSupport> {
        if environment == Some("testnet") {
            NetworkConfigManager::get_network_config("base_sepolia")
                .map(|config| config.wallet_support)
        } else {
            Self::get_wallet_compatibility(currency)
        }
    }

    /// Get token metadata (contract address, decimals, amount in base units) for debugging
    fn get_token_metadata_with_environment(
        currency: &str,
        amount: &str,
        environment: Option<&str>,
    ) -> (Option<String>, Option<u8>, Option<String>) {
        let config_key = if environment == Some("testnet") {
            "base_sepolia" // For testnet, we use base_sepolia config
        } else {
            currency
        };

        if let Some(network_config) = NetworkConfigManager::get_network_config(config_key) {
            if let Some(token_contract) = &network_config.token_contract {
                // Calculate amount in base units for token transactions
                if let Ok(token_amount) = Decimal::from_str(amount) {
                    let multiplier = Decimal::from_str(&format!(
                        "1{}",
                        "0".repeat(token_contract.decimals as usize)
                    ))
                    .unwrap_or(Decimal::from(1));
                    let base_units = token_amount * multiplier;
                    return (
                        Some(token_contract.contract_address.clone()),
                        Some(token_contract.decimals),
                        Some(base_units.trunc().to_string()),
                    );
                } else {
                    return (
                        Some(token_contract.contract_address.clone()),
                        Some(token_contract.decimals),
                        None,
                    );
                }
            }
        }
        (None, None, None)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QrMetadata {
    pub payment_uri: String,
    pub currency: String,
    pub amount: String,
    pub address: String,
    pub network: String,
    pub uri_type: String,
    pub chain_id: Option<u64>,
    pub network_config: Option<super::network_config::NetworkConfig>,
    pub setup_instructions: Option<NetworkSetupInstructions>,
    pub wallet_compatibility: Option<super::network_config::WalletSupport>,
    // Token-specific information for better frontend display and debugging
    pub token_contract: Option<String>,
    pub token_decimals: Option<u8>,
    pub amount_in_base_units: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitcoin_uri_generation() {
        let uri = PaymentUriGenerator::generate_uri(
            "bitcoin",
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
            "0.001",
            "pay_123",
            "order_456",
        )
        .unwrap();

        assert!(uri.starts_with("bitcoin:bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh?"));
        assert!(uri.contains("amount=0.001"));
        assert!(uri.contains("label=Payment%20pay_123"));
        assert!(uri.contains("message=Order%20order_456"));
    }

    #[test]
    fn test_ethereum_uri_with_wei_conversion() {
        let uri = PaymentUriGenerator::generate_uri(
            "ethereum",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "0.1",
            "pay_123",
            "order_456",
        )
        .unwrap();

        assert!(uri.starts_with("ethereum:0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7@1"));
        assert!(uri.contains("value=100000000000000000")); // 0.1 ETH = 100000000000000000 Wei
    }

    #[test]
    fn test_solana_uri_generation() {
        let uri = PaymentUriGenerator::generate_uri(
            "solana",
            "7xKXtg2CW87d4WcSwEvgX9Rqh6MbQSNXdHjWVa7Bh4ce",
            "1.5",
            "pay_123",
            "order_456",
        )
        .unwrap();

        assert!(uri.starts_with("solana:7xKXtg2CW87d4WcSwEvgX9Rqh6MbQSNXdHjWVa7Bh4ce?"));
        assert!(uri.contains("amount=1.5"));
        assert!(uri.contains("label=Payment%20pay_123"));
    }

    #[test]
    fn test_erc20_token_transfer_uri() {
        let uri = PaymentUriGenerator::generate_uri(
            "usdt",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "100",
            "pay_123",
            "order_456",
        )
        .unwrap();

        // ERC-20 tokens should generate proper token transfer URIs
        // Format: ethereum:<token_contract>@<chainId>/transfer?address=<recipient>&uint256=<amount>
        assert!(uri.starts_with("ethereum:0xdAC17F958D2ee523a2206206994597C13D831ec7")); // USDT contract FIRST
        assert!(uri.contains("@1/transfer")); // Chain ID 1 for Ethereum
        assert!(uri.contains("address=0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7")); // Recipient in params
        assert!(uri.contains("uint256=100000000")); // 100 USDT = 100000000 (6 decimals)
    }

    #[test]
    fn test_bep20_token_transfer_uri() {
        let uri = PaymentUriGenerator::generate_uri(
            "usdt_bnb",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "100",
            "pay_123",
            "order_456",
        )
        .unwrap();

        // BEP-20 USDT should generate proper token transfer URIs with BSC chain ID and 6 decimals
        // Format: ethereum:<token_contract>@<chainId>/transfer?address=<recipient>&uint256=<amount>
        assert!(uri.starts_with("ethereum:0x55d398326f99059fF775485246999027B3197955")); // USDT contract FIRST
        assert!(uri.contains("@56/transfer")); // Chain ID 56 for BSC
        assert!(uri.contains("address=0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7")); // Recipient in params
        assert!(uri.contains("uint256=100000000")); // 100 USDT = 100000000 (6 decimals)
    }

    #[test]
    fn test_invalid_amount() {
        let result = PaymentUriGenerator::generate_uri(
            "bitcoin",
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
            "invalid",
            "pay_123",
            "order_456",
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid Bitcoin amount");
    }

    #[test]
    fn test_qr_metadata() {
        let metadata = PaymentUriGenerator::get_qr_metadata(
            "bitcoin",
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
            "0.001",
            "pay_123",
            "order_456",
        );

        assert_eq!(metadata.currency, "bitcoin");
        assert_eq!(metadata.amount, "0.001");
        assert_eq!(metadata.network, "Bitcoin");
        assert_eq!(metadata.uri_type, "BIP21");
        assert!(metadata.payment_uri.starts_with("bitcoin:"));
        assert_eq!(metadata.chain_id, Some(0)); // Bitcoin doesn't use chain IDs
    }

    #[test]
    fn test_bnb_uri_with_chain_id() {
        let uri = PaymentUriGenerator::generate_uri(
            "bnb",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "0.1",
            "pay_123",
            "order_456",
        )
        .unwrap();

        assert!(uri.starts_with("ethereum:0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7@56"));
        assert!(uri.contains("value=100000000000000000")); // 0.1 BNB in Wei
    }

    #[test]
    fn test_polygon_uri_with_chain_id() {
        let uri = PaymentUriGenerator::generate_uri(
            "matic",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "1.0",
            "pay_123",
            "order_456",
        )
        .unwrap();

        assert!(uri.starts_with("ethereum:0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7@137"));
        assert!(uri.contains("value=1000000000000000000")); // 1.0 MATIC in Wei
    }

    #[test]
    fn test_usdt_two_dollar_payment() {
        // Test $2 USDT payment specifically to reproduce the issue
        let uri = PaymentUriGenerator::generate_uri(
            "usdt",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "2",
            "pay_123",
            "order_456",
        )
        .unwrap();

        println!("Generated URI for $2 USDT: {}", uri);

        // $2 USDT should become 2000000 (2 * 10^6 = 2,000,000)
        // Format: ethereum:<token_contract>@<chainId>/transfer?address=<recipient>&uint256=<amount>
        assert!(uri.starts_with("ethereum:0xdAC17F958D2ee523a2206206994597C13D831ec7")); // USDT contract FIRST
        assert!(uri.contains("@1/transfer")); // Chain ID 1 for Ethereum
        assert!(uri.contains("address=0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7")); // Recipient in params
        assert!(uri.contains("uint256=2000000")); // 2 USDT = 2000000 (6 decimals)

        // Verify the complete URI structure
        let expected_uri = "ethereum:0xdAC17F958D2ee523a2206206994597C13D831ec7@1/transfer?address=0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7&uint256=2000000";
        assert_eq!(uri, expected_uri);
    }

    #[test]
    fn test_checksum_address_eip55() {
        // Test EIP-55 checksum functionality
        let test_address = "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7";
        let checksummed = PaymentUriGenerator::to_checksum_address(test_address);

        println!("Original: {}", test_address);
        println!("Checksummed: {}", checksummed);

        // Expected checksum (verified with web3 tools)
        assert_eq!(checksummed, "0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7");

        // Test USDT contract address checksum
        let usdt_contract = "0xdac17f958d2ee523a2206206994597c13d831ec7";
        let checksummed_usdt = PaymentUriGenerator::to_checksum_address(usdt_contract);

        println!("USDT Contract Original: {}", usdt_contract);
        println!("USDT Contract Checksummed: {}", checksummed_usdt);

        // USDT contract should be checksummed properly
        assert_eq!(
            checksummed_usdt,
            "0xdAC17F958D2ee523a2206206994597C13D831ec7"
        );
    }

    #[test]
    fn test_bnb_usdt_two_dollar_payment() {
        // Test $2 USDT BEP-20 payment specifically to debug BNB issue
        let uri = PaymentUriGenerator::generate_uri(
            "usdt_bnb",
            "0x742d35cc6634c0532925a3b8d0ed1ce827d8f2d7",
            "2",
            "pay_123",
            "order_456",
        )
        .unwrap();

        println!("Generated URI for $2 USDT BNB: {}", uri);

        // $2 USDT should become 2000000 (2 * 10^6 = 2,000,000) for BEP-20 too
        // Format: ethereum:<token_contract>@<chainId>/transfer?address=<recipient>&uint256=<amount>
        assert!(uri.starts_with("ethereum:0x55d398326f99059fF775485246999027B3197955")); // USDT BEP-20 contract FIRST
        assert!(uri.contains("@56/transfer")); // Chain ID 56 for BSC
        assert!(uri.contains("address=0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7")); // Recipient in params
        assert!(uri.contains("uint256=2000000")); // 2 USDT = 2000000 (6 decimals)

        // Verify the complete URI structure
        let expected_uri = "ethereum:0x55d398326f99059fF775485246999027B3197955@56/transfer?address=0x742D35cc6634C0532925A3B8d0ED1ce827D8F2D7&uint256=2000000";
        assert_eq!(uri, expected_uri);
    }
}
