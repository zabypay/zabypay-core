use crate::shared::utils::errors::AppError;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;

/// Universal address monitor that works with all blockchain networks
pub struct UniversalAddressMonitor {
    network_configs: HashMap<String, NetworkMonitorConfig>,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct NetworkMonitorConfig {
    pub name: String,
    pub rpc_url: String,
    pub api_key: Option<String>,
    pub network_type: NetworkType,
    pub explorer_api: Option<String>,
    pub fallback_urls: Vec<String>,
    pub usdt_contract: Option<String>,
    pub chain_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum NetworkType {
    Ethereum, // Ethereum, BNB, Polygon, Avalanche
    Bitcoin,  // Bitcoin
    Solana,   // Solana
}

#[derive(Debug, Clone)]
pub struct DetectedTransaction {
    pub hash: String,
    pub amount: f64,
    pub currency: String,
    pub block_time: DateTime<Utc>,
    pub confirmations: u32,
    pub status: String,
    pub from_address: String,
    pub to_address: String,
    pub network: String,
}

impl UniversalAddressMonitor {
    pub fn new() -> Self {
        let mut network_configs = HashMap::new();

        // Get API keys from environment variables
        let etherscan_key = std::env::var("ETHERSCAN_API_KEY").ok();
        let bscscan_key = std::env::var("BSCSCAN_API_KEY").ok();
        let infura_key = std::env::var("INFURA_API_KEY")
            .unwrap_or_else(|_| "eef650a32682456db1cb76fa3f4e1206".to_string());

        // Ethereum Mainnet - prioritize Infura RPC over explorer APIs
        let (eth_api_key, eth_explorer_api) = if let Some(key) = etherscan_key.clone() {
            if !key.is_empty()
                && key != "YourEtherscanAPIKey"
                && key != "YOUR_ETHERSCAN_API_KEY_HERE"
            {
                log::info!("🔑 [ETH_CONFIG] Etherscan API available, but prioritizing Infura RPC");
                (Some(key), Some("https://api.etherscan.io/api".to_string()))
            } else {
                log::info!("✅ [ETH_CONFIG] Using Infura RPC for USDT detection (optimal)");
                (None, None)
            }
        } else {
            log::info!(
                "✅ [ETH_CONFIG] Using Infura RPC for USDT detection (no explorer API needed)"
            );
            (None, None)
        };

        network_configs.insert(
            "ethereum".to_string(),
            NetworkMonitorConfig {
                name: "ethereum".to_string(),
                rpc_url: format!("https://mainnet.infura.io/v3/{}", infura_key),
                api_key: eth_api_key,
                network_type: NetworkType::Ethereum,
                explorer_api: eth_explorer_api,
                fallback_urls: vec![
                    "https://cloudflare-eth.com".to_string(),
                    "https://rpc.ankr.com/eth".to_string(),
                    "https://eth-mainnet.public.blastapi.io".to_string(),
                ],
                usdt_contract: Some("0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string()), // USDT on Ethereum
                chain_id: Some(1),
            },
        );

        // BNB Smart Chain - prioritize reliable RPC over explorer APIs
        let (bnb_api_key, bnb_explorer_api) = if let Some(key) = bscscan_key.clone() {
            if !key.is_empty() && key != "YourBSCScanAPIKey" && key != "YOUR_BSCSCAN_API_KEY_HERE" {
                log::info!("🔑 [BNB_CONFIG] BSCScan API available, but prioritizing RPC");
                (Some(key), Some("https://api.bscscan.com/api".to_string()))
            } else {
                log::info!("✅ [BNB_CONFIG] Using reliable RPC for USDT detection (optimal)");
                (None, None)
            }
        } else {
            log::info!(
                "✅ [BNB_CONFIG] Using reliable RPC for USDT detection (no explorer API needed)"
            );
            (None, None)
        };

        network_configs.insert(
            "bnb".to_string(),
            NetworkMonitorConfig {
                name: "bnb".to_string(),
                rpc_url: "https://bsc-dataseed.binance.org/".to_string(),
                api_key: bnb_api_key,
                network_type: NetworkType::Ethereum,
                explorer_api: bnb_explorer_api,
                fallback_urls: vec![
                    "https://rpc.ankr.com/bsc".to_string(),
                    "https://bsc-mainnet.public.blastapi.io".to_string(),
                    "https://bsc-dataseed1.defibit.io".to_string(),
                    "https://bsc-dataseed1.ninicoin.io".to_string(),
                ],
                usdt_contract: Some("0x55d398326f99059fF775485246999027B3197955".to_string()), // USDT on BNB Chain
                chain_id: Some(56),
            },
        );

        // Polygon
        network_configs.insert(
            "polygon".to_string(),
            NetworkMonitorConfig {
                name: "polygon".to_string(),
                rpc_url: "https://polygon-rpc.com/".to_string(),
                api_key: None,
                network_type: NetworkType::Ethereum,
                explorer_api: Some("https://api.polygonscan.com/api".to_string()),
                fallback_urls: vec![
                    "https://rpc.ankr.com/polygon".to_string(),
                    "https://polygon-mainnet.public.blastapi.io".to_string(),
                ],
                usdt_contract: None,
                chain_id: Some(137),
            },
        );

        // Avalanche
        network_configs.insert(
            "avalanche".to_string(),
            NetworkMonitorConfig {
                name: "avalanche".to_string(),
                rpc_url: "https://api.avax.network/ext/bc/C/rpc".to_string(),
                api_key: None,
                network_type: NetworkType::Ethereum,
                explorer_api: Some("https://api.snowtrace.io/api".to_string()),
                fallback_urls: vec![
                    "https://rpc.ankr.com/avalanche".to_string(),
                    "https://avalanche-mainnet.public.blastapi.io".to_string(),
                ],
                usdt_contract: None,
                chain_id: Some(43114),
            },
        );

        // Bitcoin - Multiple reliable explorer APIs with fallbacks
        network_configs.insert(
            "bitcoin".to_string(),
            NetworkMonitorConfig {
                name: "bitcoin".to_string(),
                rpc_url: "https://blockstream.info/api".to_string(),
                api_key: None,
                network_type: NetworkType::Bitcoin,
                explorer_api: Some("https://blockstream.info/api".to_string()),
                fallback_urls: vec![
                    "https://mempool.space/api".to_string(),
                    "https://api.blockchair.com/bitcoin".to_string(),
                    "https://chain.api.btc.com/v3/".to_string(),
                ],
                usdt_contract: None,
                chain_id: None,
            },
        );

        // Solana Mainnet - only mainnet RPC endpoints
        network_configs.insert("solana".to_string(), NetworkMonitorConfig {
            name: "solana".to_string(),
            rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            api_key: None,
            network_type: NetworkType::Solana,
            explorer_api: None,
            fallback_urls: vec![
                "https://solana-mainnet.g.alchemy.com/v2/demo".to_string(),
                "https://rpc.ankr.com/solana".to_string(),
                "https://solana-api.projectserum.com".to_string(),
                "https://solana-mainnet.core.chainstack.com/56ad5ccb435b113dae7bde8b6855526ba6fdc95f".to_string(),
            ],
            usdt_contract: None,
            chain_id: None,
        });

        // Solana Testnet - testnet and devnet RPC endpoints only
        network_configs.insert(
            "solana_testnet".to_string(),
            NetworkMonitorConfig {
                name: "solana_testnet".to_string(),
                rpc_url: "https://api.testnet.solana.com".to_string(),
                api_key: None,
                network_type: NetworkType::Solana,
                explorer_api: None,
                fallback_urls: vec![
                    "https://api.devnet.solana.com".to_string(),
                    "https://solana-devnet.g.alchemy.com/v2/demo".to_string(),
                ],
                usdt_contract: None,
                chain_id: None,
            },
        );

        // Base Sepolia Testnet - Multiple reliable RPC endpoints
        network_configs.insert(
            "base_sepolia".to_string(),
            NetworkMonitorConfig {
                name: "base_sepolia".to_string(),
                rpc_url: "https://sepolia.base.org".to_string(), // Official Base Sepolia RPC endpoint
                api_key: None,
                network_type: NetworkType::Ethereum,
                explorer_api: None, // Base Sepolia API requires key, use RPC instead
                fallback_urls: vec![
                    "https://base-sepolia.public.blastapi.io".to_string(),
                    "https://base-sepolia.blockpi.network/v1/rpc/public".to_string(),
                    "https://base-sepolia-rpc.publicnode.com".to_string(),
                    "https://rpc.notadegen.com/base/sepolia".to_string(),
                    "https://base-sepolia.gateway.tenderly.co".to_string(),
                ],
                usdt_contract: None, // No USDT on testnet
                chain_id: Some(84532),
            },
        );

        Self {
            network_configs,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15)) // Reduced timeout for faster failover to fallback URLs
                .connect_timeout(std::time::Duration::from_secs(10)) // Connection timeout
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Get the correct decimal divisor for USDT based on network
    fn get_usdt_decimal_divisor(network_name: &str) -> f64 {
        match network_name {
            "bnb" => 1_000_000_000_000_000_000.0, // 18 decimals for BNB BEP-20 USDT
            "ethereum" => 1_000_000.0,            // 6 decimals for Ethereum ERC-20 USDT
            _ => 1_000_000.0,                     // Default to 6 decimals for other networks
        }
    }

    /// Get the correct USDT currency string based on network to distinguish BEP-20 from ERC-20
    fn get_usdt_currency_by_network(network_name: &str) -> String {
        match network_name {
            "bnb" => "USDT_BEP20".to_string(), // USDT on BNB Smart Chain (BEP-20)
            "ethereum" => "USDT".to_string(),  // USDT on Ethereum (ERC-20)
            "polygon" => "USDT_POLYGON".to_string(), // USDT on Polygon
            _ => "USDT".to_string(),           // Default to ERC-20 for unknown networks
        }
    }

    /// Check if address received expected payment for any network
    pub async fn check_payment_received(
        &self,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let network = self.get_network_for_currency(currency);

        if let Some(config) = self.network_configs.get(&network) {
            let env_indicator = if network.contains("testnet") {
                "TESTNET"
            } else {
                "MAINNET"
            };
            log::info!(
                " [{}] Checking {} payment for address {} on network {} (RPC: {})",
                env_indicator,
                currency.to_uppercase(),
                address,
                network,
                config.rpc_url
            );

            let result = match config.network_type {
                NetworkType::Ethereum => {
                    self.check_ethereum_payment(
                        config,
                        address,
                        expected_amount,
                        currency,
                        since_time,
                    )
                    .await
                }
                NetworkType::Bitcoin => {
                    self.check_bitcoin_payment(
                        config,
                        address,
                        expected_amount,
                        currency,
                        since_time,
                    )
                    .await
                }
                NetworkType::Solana => {
                    // Validate Solana address format
                    match Self::validate_solana_address(address) {
                        Ok(()) => {
                            log::debug!("✅ Valid Solana address: {}", address);
                            self.check_solana_payment(
                                config,
                                address,
                                expected_amount,
                                currency,
                                since_time,
                            )
                            .await
                        }
                        Err(e) => {
                            log::error!(" Invalid Solana address {}: {}", address, e);
                            Err(e)
                        }
                    }
                }
            };

            match result {
                Ok(tx) => Ok(tx),
                Err(e) => {
                    log::error!(
                        "Error checking {} payment for address {}: {}",
                        currency,
                        address,
                        e
                    );
                    Err(e)
                }
            }
        } else {
            Err(AppError::InternalServerError(format!(
                "Network {} not supported",
                network
            )))
        }
    }

    /// Check Ethereum-compatible networks (ETH, BNB, MATIC, AVAX) including USDT tokens
    async fn check_ethereum_payment(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        // Determine if this is a USDT token payment
        let is_usdt_payment = currency.to_lowercase().contains("usdt");

        log::info!(" [ETH_CHECK] Checking {} payment (USDT: {}) for address: {}, amount: {} on network: {}",
            currency.to_uppercase(), is_usdt_payment, address, expected_amount, config.name);

        // Add specific logging for BNB USDT
        if config.name == "bnb" && is_usdt_payment {
            log::info!(
                "🪙 [BNB_USDT_CHECK] Monitoring BNB BEP-20 USDT transaction with 18 decimals"
            );
            log::info!(
                "🪙 [BNB_USDT_CHECK] Contract: {:?}, Chain ID: {:?}",
                config.usdt_contract,
                config.chain_id
            );
        }

        if is_usdt_payment {
            // For USDT, check token transfers
            self.check_usdt_token_transfer(config, address, expected_amount, currency, since_time)
                .await
        } else {
            // For native currency, use existing logic
            if let Some(explorer_api) = &config.explorer_api {
                log::info!(
                    " Checking native {} payment via Explorer API: {}",
                    currency.to_uppercase(),
                    explorer_api
                );
                self.check_ethereum_via_explorer(
                    config,
                    explorer_api,
                    address,
                    expected_amount,
                    currency,
                    since_time,
                )
                .await
            } else {
                log::info!(
                    " Checking native {} payment via RPC: {}",
                    currency.to_uppercase(),
                    config.rpc_url
                );
                self.check_ethereum_via_rpc(config, address, expected_amount, currency, since_time)
                    .await
            }
        }
    }

    /// Check Ethereum via block explorer API (more reliable)
    async fn check_ethereum_via_explorer(
        &self,
        config: &NetworkMonitorConfig,
        explorer_api: &str,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            " [EXPLORER_CHECK] Checking {} via explorer API for address: {}, amount: {}",
            currency.to_uppercase(),
            address,
            expected_amount
        );

        // Build API key parameter if available
        let api_key_param = if let Some(api_key) = &config.api_key {
            format!("&apikey={}", api_key)
        } else {
            String::new()
        };

        // First get the current block number to calculate confirmations
        let block_url = format!(
            "{}?module=proxy&action=eth_blockNumber{}",
            explorer_api, api_key_param
        );
        let block_response = self.client.get(&block_url).send().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to get current block: {}", e))
        })?;

        let block_json: Value = block_response
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Parse block error: {}", e)))?;

        let current_block = if let Some(hex_block) = block_json["result"].as_str() {
            u64::from_str_radix(&hex_block[2..], 16).unwrap_or(0)
        } else {
            0
        };

        log::info!(
            "📦 [EXPLORER_CHECK] Current block height: {}",
            current_block
        );

        // Get recent native currency transactions for the address
        let url = format!(
            "{}?module=account&action=txlist&address={}&startblock=0&endblock=99999999&sort=desc&page=1&offset=20{}",
            explorer_api, address, api_key_param
        );

        log::info!(
            "🌐 [EXPLORER_CHECK] Fetching transactions (API key: {})",
            if config.api_key.is_some() {
                "provided"
            } else {
                "none"
            }
        );

        let response =
            self.client.get(&url).send().await.map_err(|e| {
                AppError::InternalServerError(format!("Explorer API failed: {}", e))
            })?;

        let json: Value = response
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Parse error: {}", e)))?;

        log::info!(
            " [EXPLORER_CHECK] Found {} transactions in response",
            json["result"].as_array().map(|a| a.len()).unwrap_or(0)
        );

        if let Some(result) = json["result"].as_array() {
            for tx in result {
                let tx_hash = tx["hash"].as_str().unwrap_or("unknown");
                let to_addr = tx["to"].as_str().unwrap_or("");
                let from_addr = tx["from"].as_str().unwrap_or("");

                log::debug!(
                    "🔎 [TX_CHECK] Examining tx {}: from={}, to={}",
                    tx_hash,
                    from_addr,
                    to_addr
                );

                // Check if transaction is to our address
                if to_addr.to_lowercase() != address.to_lowercase() {
                    log::debug!("  ↳ Skipping: wrong recipient (expected: {})", address);
                    continue;
                }

                // Check transaction time
                if let Some(timestamp) = tx["timeStamp"].as_str() {
                    if let Ok(ts) = timestamp.parse::<i64>() {
                        let tx_time = DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now);
                        if tx_time < since_time {
                            log::debug!(
                                "  ↳ Skipping: too old (tx: {}, required: {})",
                                tx_time.to_rfc3339(),
                                since_time.to_rfc3339()
                            );
                            continue;
                        }
                    }
                }

                // Convert value from Wei to currency unit
                if let Some(value_str) = tx["value"].as_str() {
                    if let Ok(value_wei) = u128::from_str(value_str) {
                        let value = value_wei as f64 / 1_000_000_000_000_000_000.0; // Wei to ETH/BNB/etc

                        log::info!(
                            "💰 [TX_AMOUNT] Transaction value: {} {} (expected: {} {})",
                            value,
                            currency.to_uppercase(),
                            expected_amount,
                            currency.to_uppercase()
                        );

                        // Check if amount matches (with tolerance)
                        let tolerance = expected_amount * 0.01; // 1% tolerance for better matching
                        if (value - expected_amount).abs() <= tolerance && value > 0.0 {
                            // Check if transaction is successful
                            let is_error = tx["isError"].as_str() == Some("1");
                            let receipt_status = tx["txreceipt_status"].as_str().unwrap_or("1");
                            let is_success = !is_error && receipt_status != "0";

                            log::info!("🎯 [TX_STATUS] Transaction status - isError: {}, receipt: {}, success: {}", 
                                is_error, receipt_status, is_success);

                            if is_success {
                                // Calculate confirmations from block heights
                                let confirmations = if current_block > 0 {
                                    if let Some(block_num_str) = tx["blockNumber"].as_str() {
                                        if let Ok(tx_block) = block_num_str.parse::<u64>() {
                                            let calc_confirmations =
                                                current_block.saturating_sub(tx_block) + 1;
                                            log::info!("📈 [CONFIRMATIONS] Block {} -> {}, confirmations: {}", 
                                                tx_block, current_block, calc_confirmations);
                                            calc_confirmations as u32
                                        } else {
                                            1
                                        }
                                    } else {
                                        1
                                    }
                                } else {
                                    1
                                };

                                log::info!("✅ [NATIVE_DETECTED] {} transaction {} detected with {} confirmations", 
                                    currency.to_uppercase(), tx_hash, confirmations);

                                return Ok(Some(DetectedTransaction {
                                    hash: tx["hash"].as_str().unwrap_or("").to_string(),
                                    amount: value,
                                    currency: currency.to_uppercase(),
                                    block_time: DateTime::from_timestamp(
                                        tx["timeStamp"]
                                            .as_str()
                                            .unwrap_or("0")
                                            .parse::<i64>()
                                            .unwrap_or(0),
                                        0,
                                    )
                                    .unwrap_or_else(Utc::now),
                                    confirmations,
                                    status: "confirmed".to_string(),
                                    from_address: tx["from"].as_str().unwrap_or("").to_string(),
                                    to_address: address.to_string(),
                                    network: config.name.clone(),
                                }));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check Ethereum via RPC (fallback) - Transaction history based detection
    async fn check_ethereum_via_rpc(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        // For native currencies, use balance detection
        log::info!(
            " [NATIVE_MONITOR] Checking native {} payment",
            serde_json::json!({
                "network": config.name.to_uppercase(),
                "currency": currency.to_uppercase(),
                "address": address,
                "expected_amount": expected_amount,
                "detector": "RPC_BALANCE",
                "since_time": since_time.to_rfc3339(),
            })
        );

        // Native currency balance check
        let balance_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBalance",
            "params": [address, "latest"],
            "id": 1
        });

        // Use RPC request with fallback for Ethereum-based networks
        let json = self
            .make_ethereum_rpc_request_with_fallback(config, &balance_payload)
            .await?;

        if let Some(balance_hex) = json["result"].as_str() {
            if let Ok(balance_wei) = u128::from_str_radix(&balance_hex[2..], 16) {
                let balance = balance_wei as f64 / 1_000_000_000_000_000_000.0; // Wei to native token

                log::info!(
                    " [DEBUG] Balance check: address {} has {} {}, expected {} {}",
                    address,
                    balance,
                    currency.to_uppercase(),
                    expected_amount,
                    currency.to_uppercase()
                );

                // Simplified balance-based detection - consistent with testnet behavior
                // Remove the restrictive max_reasonable_balance logic that prevents multiple payments
                let tolerance = expected_amount * 0.001; // 0.1% tolerance

                log::info!(
                    " [DEBUG] Balance check: {} >= {} (expected - tolerance) && balance > 0",
                    balance,
                    expected_amount - tolerance
                );

                // Simple condition: if balance >= expected amount (with tolerance) and balance > 0
                // This allows multiple payments to the same address to be detected properly
                if balance >= (expected_amount - tolerance) && balance > 0.0 {
                    log::info!(
                        "✅ [NATIVE_PAYMENT_DETECTED] {}",
                        serde_json::json!({
                            "network": config.name.to_uppercase(),
                            "currency": currency.to_uppercase(),
                            "address": address,
                            "balance": balance,
                            "expected_amount": expected_amount,
                            "detector": "RPC_BALANCE",
                            "confirmations": 1,
                            "status": "detected"
                        })
                    );

                    // Try to find the actual transaction hash by scanning recent blocks
                    log::info!("💡 [BALANCE_DETECTED] Payment detected via balance, attempting to find actual transaction hash...");
                    if let Some(actual_tx) = self
                        .find_actual_transaction_by_amount(
                            config,
                            address,
                            expected_amount,
                            currency,
                            since_time,
                        )
                        .await?
                    {
                        log::info!(
                            "🎯 [REAL_TX_FOUND] Found actual transaction: {}",
                            actual_tx.hash
                        );
                        return Ok(Some(actual_tx));
                    }

                    log::warn!(" [FALLBACK_TO_BALANCE] Could not find actual transaction hash, using balance detection as fallback");
                    return Ok(Some(DetectedTransaction {
                        hash: format!("balance_detected_{}", chrono::Utc::now().timestamp()),
                        amount: expected_amount, // Use expected amount for consistency
                        currency: currency.to_uppercase(),
                        block_time: Utc::now(),
                        confirmations: 10, // High confirmation count for balance-detected payments (already confirmed)
                        status: "confirmed".to_string(),
                        from_address: "unknown".to_string(),
                        to_address: address.to_string(),
                        network: config.name.clone(),
                    }));
                }

                log::debug!("⏳ [NATIVE_PAYMENT_PENDING] Balance {} {} does not meet payment detection criteria (expected {} {}, tolerance: {})", 
                    balance, currency.to_uppercase(), expected_amount, currency.to_uppercase(), tolerance);
            }
        }

        Ok(None)
    }

    /// Find the actual transaction hash by scanning recent blocks for matching amount
    async fn find_actual_transaction_by_amount(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            " [TX_SEARCH] Searching for actual transaction to {} with amount {}",
            address,
            expected_amount
        );

        // For EVM networks, try to find actual transaction using block scanning
        if matches!(config.network_type, NetworkType::Ethereum) {
            // Try explorer API first (more efficient)
            if let Some(explorer_api) = &config.explorer_api {
                if let Ok(Some(tx)) = self
                    .check_ethereum_via_explorer(
                        config,
                        explorer_api,
                        address,
                        expected_amount,
                        currency,
                        since_time,
                    )
                    .await
                {
                    return Ok(Some(tx));
                }
            }

            // Fallback to RPC scanning of recent blocks (last 50 blocks)
            if let Ok(Some(tx)) = self
                .scan_recent_blocks_for_transaction(
                    config,
                    address,
                    expected_amount,
                    currency,
                    since_time,
                )
                .await
            {
                return Ok(Some(tx));
            }
        }

        Ok(None)
    }

    /// Scan recent blocks for a transaction matching the expected amount
    async fn scan_recent_blocks_for_transaction(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            " [BLOCK_SCAN] Scanning recent blocks for transaction to {} with amount {}",
            address,
            expected_amount
        );

        // Get latest block number
        let latest_block_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });

        let json = self
            .make_ethereum_rpc_request_with_fallback(config, &latest_block_payload)
            .await?;
        let latest_block_hex = json["result"].as_str().ok_or_else(|| {
            AppError::InternalServerError("No latest block in response".to_string())
        })?;
        let latest_block = u64::from_str_radix(&latest_block_hex[2..], 16).map_err(|_| {
            AppError::InternalServerError("Invalid block number format".to_string())
        })?;

        // Scan last 50 blocks for the transaction
        let blocks_to_scan = 50;
        let start_block = latest_block.saturating_sub(blocks_to_scan);

        log::info!(
            " [BLOCK_SCAN] Scanning blocks {} to {} for transaction",
            start_block,
            latest_block
        );

        for block_num in (start_block..=latest_block).rev() {
            if let Ok(Some(tx)) = self
                .check_block_for_transaction(
                    config,
                    block_num,
                    address,
                    expected_amount,
                    since_time,
                )
                .await
            {
                log::info!(
                    "🎯 [REAL_TX_FOUND] Found matching transaction in block {}: {}",
                    block_num,
                    tx.hash
                );
                return Ok(Some(tx));
            }

            // Add small delay to avoid overwhelming the RPC
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        log::info!(
            " [BLOCK_SCAN] No matching transaction found in last {} blocks",
            blocks_to_scan
        );
        Ok(None)
    }

    /// Make Ethereum RPC request with fallback URLs and retry logic
    async fn make_ethereum_rpc_request_with_fallback(
        &self,
        config: &NetworkMonitorConfig,
        payload: &Value,
    ) -> Result<Value, AppError> {
        let mut urls = vec![config.rpc_url.clone()];
        urls.extend(config.fallback_urls.clone());

        let mut last_error = None;

        for (i, url) in urls.iter().enumerate() {
            log::info!(
                "🔧 Trying {} RPC endpoint {}/{}: {}",
                config.name.to_uppercase(),
                i + 1,
                urls.len(),
                url
            );

            match self.client.post(url).json(payload).send().await {
                Ok(response) => {
                    match response.json::<Value>().await {
                        Ok(json) => {
                            // Check for RPC errors
                            if let Some(error) = json.get("error") {
                                let error_msg = format!(
                                    "{} RPC error from {}: {}",
                                    config.name.to_uppercase(),
                                    url,
                                    error
                                        .get("message")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("Unknown error")
                                );
                                log::warn!(" {}", error_msg);
                                last_error = Some(AppError::ExternalServiceError(error_msg));
                                continue;
                            }

                            log::info!(
                                "✅ {} RPC success with endpoint: {}",
                                config.name.to_uppercase(),
                                url
                            );
                            return Ok(json);
                        }
                        Err(e) => {
                            let error_msg = format!(
                                "Failed to parse {} RPC response from {}: {}",
                                config.name.to_uppercase(),
                                url,
                                e
                            );
                            log::warn!(" {}", error_msg);
                            last_error = Some(AppError::InternalServerError(error_msg));
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!(
                        "{} RPC request failed to {}: {}",
                        config.name.to_uppercase(),
                        url,
                        e
                    );
                    log::warn!(" {}", error_msg);
                    last_error = Some(AppError::InternalServerError(error_msg));
                }
            }

            // Add exponential backoff delay between retries (but not on the last attempt)
            if i < urls.len() - 1 {
                let delay_ms = std::cmp::min(1000 * (2_u64.pow(i as u32)), 5000); // Max 5s delay
                log::info!(
                    "⏳ Waiting {}ms before trying next {} RPC endpoint...",
                    delay_ms,
                    config.name.to_uppercase()
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::InternalServerError(format!(
                "All {} RPC endpoints failed",
                config.name.to_uppercase()
            ))
        }))
    }

    /// Check recent transactions for Base Sepolia using RPC
    async fn check_recent_transactions_via_rpc(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        // Get the latest block number
        let latest_block_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });

        let json = self
            .make_ethereum_rpc_request_with_fallback(config, &latest_block_payload)
            .await?;

        let latest_block_hex = json["result"].as_str().ok_or_else(|| {
            AppError::InternalServerError("No latest block in response".to_string())
        })?;

        let latest_block = u64::from_str_radix(&latest_block_hex[2..], 16).map_err(|_| {
            AppError::InternalServerError("Invalid block number format".to_string())
        })?;

        // Check last 100 blocks for transactions to this address
        let blocks_to_check = std::cmp::min(100, latest_block);
        let start_block = latest_block.saturating_sub(blocks_to_check);

        log::info!(
            " Checking Base Sepolia blocks {} to {} for transactions to {}",
            start_block,
            latest_block,
            address
        );

        // Check recent blocks for transactions
        for block_num in (start_block..=latest_block).rev() {
            if let Ok(Some(tx)) = self
                .check_block_for_transaction(
                    config,
                    block_num,
                    address,
                    expected_amount,
                    since_time,
                )
                .await
            {
                log::info!(
                    "✅ Found matching transaction in block {}: {}",
                    block_num,
                    tx.hash
                );
                return Ok(Some(tx));
            }
        }

        log::info!(
            "No matching transactions found in recent {} blocks",
            blocks_to_check
        );
        Ok(None)
    }

    /// Check a specific block for transactions to the target address
    async fn check_block_for_transaction(
        &self,
        config: &NetworkMonitorConfig,
        block_number: u64,
        target_address: &str,
        expected_amount: f64,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let block_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBlockByNumber",
            "params": [format!("0x{:x}", block_number), true],
            "id": 1
        });

        let json = self
            .make_ethereum_rpc_request_with_fallback(config, &block_payload)
            .await?;

        if let Some(block) = json["result"].as_object() {
            // Check block timestamp
            if let Some(timestamp_hex) = block.get("timestamp").and_then(|t| t.as_str()) {
                if let Ok(timestamp) = u64::from_str_radix(&timestamp_hex[2..], 16) {
                    let block_time =
                        DateTime::from_timestamp(timestamp as i64, 0).unwrap_or_else(Utc::now);
                    if block_time < since_time {
                        return Ok(None); // Block is too old
                    }
                }
            }

            // Check transactions in this block
            if let Some(transactions) = block.get("transactions").and_then(|txs| txs.as_array()) {
                for tx in transactions {
                    if let Some(tx_obj) = tx.as_object() {
                        // Check if transaction is to our target address
                        if let Some(to_addr) = tx_obj.get("to").and_then(|t| t.as_str()) {
                            if to_addr.to_lowercase() == target_address.to_lowercase() {
                                // Check transaction value
                                if let Some(value_hex) =
                                    tx_obj.get("value").and_then(|v| v.as_str())
                                {
                                    if let Ok(value_wei) = u128::from_str_radix(&value_hex[2..], 16)
                                    {
                                        let value_eth =
                                            value_wei as f64 / 1_000_000_000_000_000_000.0;

                                        // Check if amount matches with tolerance
                                        let tolerance = expected_amount * 0.001;
                                        if (value_eth - expected_amount).abs() <= tolerance {
                                            let tx_hash = tx_obj
                                                .get("hash")
                                                .and_then(|h| h.as_str())
                                                .unwrap_or("unknown")
                                                .to_string();

                                            let block_time = if let Some(timestamp_hex) =
                                                block.get("timestamp").and_then(|t| t.as_str())
                                            {
                                                if let Ok(timestamp) =
                                                    u64::from_str_radix(&timestamp_hex[2..], 16)
                                                {
                                                    DateTime::from_timestamp(timestamp as i64, 0)
                                                        .unwrap_or_else(Utc::now)
                                                } else {
                                                    Utc::now()
                                                }
                                            } else {
                                                Utc::now()
                                            };

                                            return Ok(Some(DetectedTransaction {
                                                hash: tx_hash,
                                                amount: value_eth,
                                                currency: "ETH".to_string(),
                                                block_time,
                                                confirmations: 1, // Assume confirmed if in block
                                                status: "confirmed".to_string(),
                                                from_address: tx_obj
                                                    .get("from")
                                                    .and_then(|f| f.as_str())
                                                    .unwrap_or("unknown")
                                                    .to_string(),
                                                to_address: target_address.to_string(),
                                                network: config.name.clone(),
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check Bitcoin payments with multiple explorer fallbacks
    async fn check_bitcoin_payment(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            " [BTC_CHECK] Checking Bitcoin payment for address: {}, amount: {} BTC",
            address,
            expected_amount
        );

        // Try primary explorer first, then fallbacks
        let mut explorers = vec![config.rpc_url.clone()];
        explorers.extend(config.fallback_urls.clone());

        for (i, explorer_url) in explorers.iter().enumerate() {
            log::debug!(
                "🔗 [BTC_EXPLORER] Trying explorer {}/{}: {}",
                i + 1,
                explorers.len(),
                explorer_url
            );

            match self
                .check_bitcoin_via_explorer(
                    explorer_url,
                    address,
                    expected_amount,
                    currency,
                    since_time,
                )
                .await
            {
                Ok(Some(tx)) => {
                    log::info!(
                        "✅ [BTC_SUCCESS] Transaction found via explorer: {}",
                        explorer_url
                    );
                    return Ok(Some(tx));
                }
                Ok(None) => {
                    log::debug!(
                        " [BTC_NO_TX] No matching transaction found via: {}",
                        explorer_url
                    );
                    continue;
                }
                Err(e) => {
                    log::warn!(" [BTC_ERROR] Explorer {} failed: {}", explorer_url, e);
                    continue;
                }
            }
        }

        log::info!(
            " [BTC_NOT_FOUND] No Bitcoin transaction found for address: {}",
            address
        );
        Ok(None)
    }

    /// Check Bitcoin transactions via specific explorer API
    async fn check_bitcoin_via_explorer(
        &self,
        explorer_url: &str,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        // Handle different explorer API formats
        let api_url = if explorer_url.contains("blockstream.info") {
            format!("{}/address/{}/txs", explorer_url, address)
        } else if explorer_url.contains("mempool.space") {
            format!("{}/address/{}/txs", explorer_url, address)
        } else if explorer_url.contains("blockchair.com") {
            format!("{}/dashboards/address/{}?limit=100", explorer_url, address)
        } else {
            format!("{}/address/{}/txs", explorer_url, address) // Default format
        };

        log::debug!("🌐 [BTC_API] Requesting: {}", api_url);

        let response = self.client.get(&api_url).send().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Bitcoin API request failed: {}", e))
        })?;

        let response_text = response.text().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to read response: {}", e))
        })?;

        log::debug!(
            "📡 [BTC_RESPONSE] Raw response length: {} bytes",
            response_text.len()
        );

        // Parse response based on explorer type
        if explorer_url.contains("blockchair.com") {
            self.parse_blockchair_response(
                &response_text,
                address,
                expected_amount,
                currency,
                since_time,
            )
            .await
        } else {
            self.parse_blockstream_response(
                &response_text,
                address,
                expected_amount,
                currency,
                since_time,
            )
            .await
        }
    }

    /// Parse Blockstream/Mempool.space API response
    async fn parse_blockstream_response(
        &self,
        response_text: &str,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let transactions: Vec<Value> = serde_json::from_str(response_text).map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to parse Bitcoin API response: {}", e))
        })?;

        // Get current block height for confirmation calculation
        let current_height = self.get_bitcoin_block_height().await.unwrap_or(0);

        for tx in transactions {
            // Check transaction time
            let tx_time = if let Some(block_time) = tx["status"]["block_time"].as_i64() {
                DateTime::from_timestamp(block_time, 0).unwrap_or_else(Utc::now)
            } else {
                continue; // Skip unconfirmed transactions without block time
            };

            if tx_time < since_time {
                continue;
            }

            let tx_hash = tx["txid"].as_str().unwrap_or("").to_string();
            let is_confirmed = tx["status"]["confirmed"].as_bool().unwrap_or(false);
            let block_height = tx["status"]["block_height"].as_u64().unwrap_or(0);

            // Calculate confirmations
            let confirmations = if is_confirmed && current_height > 0 && block_height > 0 {
                (current_height.saturating_sub(block_height) + 1) as u32
            } else {
                0
            };

            log::debug!(
                " [BTC_TX] Checking tx: {}, block: {}, confirmations: {}",
                tx_hash,
                block_height,
                confirmations
            );

            // Check outputs for our address
            if let Some(vout) = tx["vout"].as_array() {
                for output in vout {
                    if let Some(script_address) = output["scriptpubkey_address"].as_str() {
                        if script_address == address {
                            if let Some(value_sats) = output["value"].as_u64() {
                                let value_btc = value_sats as f64 / 100_000_000.0;

                                log::info!(
                                    "💰 [BTC_AMOUNT] Found output: {} BTC to {}, expected: {} BTC",
                                    value_btc,
                                    address,
                                    expected_amount
                                );

                                // Check if amount matches with tolerance
                                let tolerance = expected_amount * 0.01; // 1% tolerance
                                if (value_btc - expected_amount).abs() <= tolerance
                                    && value_btc > 0.0
                                {
                                    log::info!(
                                        "🎯 [BTC_MATCH] Amount matches! confirmations: {}",
                                        confirmations
                                    );

                                    return Ok(Some(DetectedTransaction {
                                        hash: tx_hash,
                                        amount: value_btc,
                                        currency: currency.to_uppercase(),
                                        block_time: tx_time,
                                        confirmations,
                                        status: if confirmations >= 6 {
                                            "confirmed"
                                        } else {
                                            "pending"
                                        }
                                        .to_string(),
                                        from_address: "bitcoin_sender".to_string(),
                                        to_address: address.to_string(),
                                        network: "bitcoin".to_string(),
                                    }));
                                } else {
                                    log::debug!(" [BTC_AMOUNT_MISMATCH] Amount {} BTC doesn't match expected {} BTC", 
                                        value_btc, expected_amount);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Parse Blockchair API response
    async fn parse_blockchair_response(
        &self,
        response_text: &str,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let json: Value = serde_json::from_str(response_text).map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to parse Blockchair response: {}", e))
        })?;

        if let Some(data) = json["data"].get(address) {
            if let Some(transactions) = data["transactions"].as_array() {
                for tx_hash in transactions {
                    if let Some(tx_hash_str) = tx_hash.as_str() {
                        // This would require additional API calls to get transaction details
                        // For now, fallback to other explorers
                        log::debug!(
                            "🔗 [BLOCKCHAIR] Found tx: {} (details require additional API call)",
                            tx_hash_str
                        );
                    }
                }
            }
        }

        Ok(None)
    }

    /// Get current Bitcoin block height
    async fn get_bitcoin_block_height(&self) -> Result<u64, AppError> {
        let url = "https://blockstream.info/api/blocks/tip/height";

        match self.client.get(url).send().await {
            Ok(response) => match response.text().await {
                Ok(height_str) => height_str.trim().parse::<u64>().map_err(|_| {
                    AppError::ExternalServiceError("Invalid block height format".to_string())
                }),
                Err(_) => Err(AppError::ExternalServiceError(
                    "Failed to read block height response".to_string(),
                )),
            },
            Err(_) => Err(AppError::ExternalServiceError(
                "Failed to fetch block height".to_string(),
            )),
        }
    }

    /// Verify a specific transaction hash for USDT transfer (for debugging/confirmation)
    pub async fn verify_usdt_transaction(
        &self,
        tx_hash: &str,
        expected_to_address: &str,
        expected_amount: f64,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            " [VERIFY_USDT] Verifying transaction {} for {} USDT to {}",
            tx_hash,
            expected_amount,
            expected_to_address
        );

        // Try both Ethereum and BNB networks
        for network_name in ["ethereum", "bnb"] {
            if let Some(config) = self.network_configs.get(network_name) {
                log::info!(
                    " [VERIFY_{}] Checking transaction on {} network",
                    network_name.to_uppercase(),
                    network_name
                );

                if let Ok(Some(tx)) = self
                    .verify_transaction_on_network(
                        config,
                        tx_hash,
                        expected_to_address,
                        expected_amount,
                    )
                    .await
                {
                    return Ok(Some(tx));
                }
            }
        }

        Ok(None)
    }

    /// Verify transaction on a specific network
    async fn verify_transaction_on_network(
        &self,
        config: &NetworkMonitorConfig,
        tx_hash: &str,
        expected_to_address: &str,
        expected_amount: f64,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionReceipt",
            "params": [tx_hash],
            "id": 1
        });

        let json = self
            .make_ethereum_rpc_request_with_fallback(config, &payload)
            .await?;

        if let Some(receipt) = json["result"].as_object() {
            if let Some(logs) = receipt["logs"].as_array() {
                log::info!("📋 [VERIFY_USDT] Transaction has {} logs", logs.len());

                for (i, log_entry) in logs.iter().enumerate() {
                    log::debug!(
                        " [VERIFY_LOG_{}] {}",
                        i,
                        serde_json::to_string(log_entry).unwrap_or_default()
                    );

                    // Check if this is a Transfer event for USDT contract
                    if let Some(address) = log_entry["address"].as_str() {
                        let contract_address = config
                            .usdt_contract
                            .as_deref()
                            .unwrap_or("0xdac17f958d2ee523a2206206994597c13d831ec7");
                        if address.to_lowercase() == contract_address.to_lowercase() {
                            // USDT contract for this network
                            if let Some(topics) = log_entry["topics"].as_array() {
                                if topics.len() >= 3 {
                                    // topics[0] = Transfer signature
                                    // topics[1] = from address
                                    // topics[2] = to address
                                    if let Some(to_topic) = topics[2].as_str() {
                                        let to_address =
                                            format!("0x{}", &to_topic[26..].to_lowercase());

                                        log::info!(
                                            "📧 [VERIFY_TO] Log to address: {} (expected: {})",
                                            to_address,
                                            expected_to_address.to_lowercase()
                                        );

                                        if to_address == expected_to_address.to_lowercase() {
                                            if let Some(data) = log_entry["data"].as_str() {
                                                if data.len() >= 66 {
                                                    if let Ok(amount_raw) =
                                                        u128::from_str_radix(&data[2..], 16)
                                                    {
                                                        let decimal_divisor =
                                                            Self::get_usdt_decimal_divisor(
                                                                &config.name,
                                                            );
                                                        let token_amount =
                                                            amount_raw as f64 / decimal_divisor;

                                                        log::info!("💰 [VERIFY_AMOUNT] Found {} USDT (expected: {}) using {} decimals (divisor: {}, raw: {})",
                                                            token_amount, expected_amount, if config.name == "bnb" { "18" } else { "6" }, decimal_divisor, amount_raw);

                                                        return Ok(Some(DetectedTransaction {
                                                            hash: tx_hash.to_string(),
                                                            amount: token_amount,
                                                            currency:
                                                                Self::get_usdt_currency_by_network(
                                                                    &config.name,
                                                                ),
                                                            block_time: Utc::now(),
                                                            confirmations: 999, // Mark as verified
                                                            status: "confirmed".to_string(),
                                                            from_address: format!(
                                                                "0x{}",
                                                                &topics[1].as_str().unwrap_or("")
                                                                    [26..]
                                                            ),
                                                            to_address: expected_to_address
                                                                .to_string(),
                                                            network: "ethereum".to_string(),
                                                        }));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check USDT token transfers (ERC-20/BEP-20)
    async fn check_usdt_token_transfer(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!("🪙 [USDT_CHECK] Checking USDT token transfer for address: {}, amount: {} USDT on network: {}", address, expected_amount, config.name);

        let contract_address = match &config.usdt_contract {
            Some(contract) => contract,
            None => {
                log::warn!(
                    " [USDT_CONFIG] No USDT contract configured for network: {}",
                    config.name
                );
                return Ok(None);
            }
        };

        log::info!(
            "📝 [USDT_CONTRACT] Using USDT contract: {} on {}",
            contract_address,
            config.name
        );

        // Prioritize RPC detection (more reliable and doesn't need API keys)
        log::debug!("🔗 [USDT_RPC] Checking USDT via RPC (primary method)");
        match self
            .check_usdt_via_rpc(
                config,
                contract_address,
                address,
                expected_amount,
                currency,
                since_time,
            )
            .await
        {
            Ok(Some(tx)) => return Ok(Some(tx)),
            Ok(None) => log::debug!(" [USDT_RPC] No USDT transaction found via RPC"),
            Err(e) => log::warn!(" [USDT_RPC] RPC check failed: {}", e),
        }

        // Fallback to explorer API if available
        if let Some(explorer_api) = &config.explorer_api {
            log::debug!(" [USDT_EXPLORER] Checking USDT via explorer API (fallback)");
            self.check_usdt_via_explorer(
                config,
                explorer_api,
                contract_address,
                address,
                expected_amount,
                currency,
                since_time,
            )
            .await
        } else {
            Ok(None)
        }
    }

    /// Check USDT token transfers via explorer API (tokentx endpoint)
    async fn check_usdt_via_explorer(
        &self,
        config: &NetworkMonitorConfig,
        explorer_api: &str,
        contract_address: &str,
        to_address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let api_key_param = if let Some(api_key) = &config.api_key {
            format!("&apikey={}", api_key)
        } else {
            String::new()
        };

        // Use tokentx API to get token transfers to the address
        let url = format!(
            "{}?module=account&action=tokentx&contractaddress={}&address={}&page=1&offset=100&sort=desc{}",
            explorer_api, contract_address, to_address, api_key_param
        );

        log::debug!("🌐 [USDT_API] Requesting: {}", url);

        let response = self.client.get(&url).send().await.map_err(|e| {
            AppError::ExternalServiceError(format!("USDT explorer API failed: {}", e))
        })?;

        let json: Value = response.json().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to parse USDT response: {}", e))
        })?;

        if json["status"] != "1" {
            let message = json["message"].as_str().unwrap_or("Unknown error");
            log::warn!(" [USDT_API] Explorer API error: {}", message);
            return Ok(None);
        }

        let empty_vec = vec![];
        let transfers = json["result"].as_array().unwrap_or(&empty_vec);
        log::debug!(
            "📋 [USDT_TRANSFERS] Found {} token transfers",
            transfers.len()
        );

        // Get current block for confirmation calculation
        let current_block = self.get_current_block_number(config).await.unwrap_or(0);

        for transfer in transfers {
            // Check timestamp
            let timestamp = transfer["timeStamp"]
                .as_str()
                .and_then(|ts| ts.parse::<i64>().ok())
                .map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now))
                .unwrap_or_else(Utc::now);

            if timestamp < since_time {
                continue;
            }

            // Check if this is a transfer TO our address
            let to = transfer["to"].as_str().unwrap_or("");
            if to.to_lowercase() != to_address.to_lowercase() {
                continue;
            }

            let tx_hash = transfer["hash"].as_str().unwrap_or("").to_string();
            let block_number = transfer["blockNumber"]
                .as_str()
                .and_then(|bn| bn.parse::<u64>().ok())
                .unwrap_or(0);

            // Calculate confirmations
            let confirmations = if current_block > 0 && block_number > 0 {
                (current_block.saturating_sub(block_number) + 1) as u32
            } else {
                0
            };

            // Parse token amount (USDT has 6 decimals)
            let value_str = transfer["value"].as_str().unwrap_or("0");
            let token_amount = if let Ok(value_raw) = value_str.parse::<u128>() {
                value_raw as f64 / 1_000_000.0 // Convert from 6 decimal places
            } else {
                continue;
            };

            log::info!(
                "💰 [USDT_TRANSFER] Found transfer: {} USDT to {}, expected: {} USDT",
                token_amount,
                to_address,
                expected_amount
            );

            // Check if amount matches with tolerance
            let tolerance = expected_amount * 0.01; // 1% tolerance
            if (token_amount - expected_amount).abs() <= tolerance && token_amount > 0.0 {
                log::info!(
                    "🎯 [USDT_MATCH] USDT amount matches! confirmations: {}",
                    confirmations
                );

                let required_confirmations = match config.name.as_str() {
                    "ethereum" => 3,
                    "bnb" => 3,
                    _ => 3,
                };

                return Ok(Some(DetectedTransaction {
                    hash: tx_hash,
                    amount: token_amount,
                    currency: Self::get_usdt_currency_by_network(&config.name),
                    block_time: timestamp,
                    confirmations,
                    status: if confirmations >= required_confirmations {
                        "confirmed"
                    } else {
                        "pending"
                    }
                    .to_string(),
                    from_address: transfer["from"].as_str().unwrap_or("unknown").to_string(),
                    to_address: to_address.to_string(),
                    network: config.name.clone(),
                }));
            } else {
                log::debug!(
                    " [USDT_AMOUNT_MISMATCH] Amount {} USDT doesn't match expected {} USDT",
                    token_amount,
                    expected_amount
                );
            }
        }

        Ok(None)
    }

    /// Check a specific transaction hash for USDT transfer to target address
    pub async fn verify_transaction_for_payment(
        &self,
        tx_hash: &str,
        to_address: &str,
        expected_amount: f64,
        currency: &str,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        let network = self.get_network_for_currency(currency);

        if let Some(config) = self.network_configs.get(&network) {
            log::info!(
                " [TX_VERIFY] Verifying transaction {} for {} USDT to {}",
                tx_hash,
                expected_amount,
                to_address
            );

            // Get transaction receipt
            let payload = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionReceipt",
                "params": [tx_hash],
                "id": 1
            });

            let mut urls = vec![config.rpc_url.clone()];
            urls.extend(config.fallback_urls.clone());

            for rpc_url in urls {
                match self.client.post(&rpc_url).json(&payload).send().await {
                    Ok(response) => {
                        match response.json::<Value>().await {
                            Ok(json) => {
                                if let Some(error) = json.get("error") {
                                    log::warn!(" [TX_VERIFY] RPC error: {}", error);
                                    continue;
                                }

                                if let Some(receipt) = json["result"].as_object() {
                                    if let Some(logs) = receipt["logs"].as_array() {
                                        log::info!(
                                            "📋 [TX_VERIFY] Found {} logs in transaction receipt",
                                            logs.len()
                                        );

                                        // Check for USDT transfer to our address
                                        let transfer_topic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
                                        let normalized_address = if to_address.starts_with("0x") {
                                            to_address[2..].to_lowercase()
                                        } else {
                                            to_address.to_lowercase()
                                        };

                                        for log_entry in logs {
                                            if let Some(topics) = log_entry["topics"].as_array() {
                                                if topics.len() >= 3
                                                    && topics[0].as_str() == Some(transfer_topic)
                                                    && topics[2]
                                                        .as_str()
                                                        .map(|s| {
                                                            s.to_lowercase()
                                                                .ends_with(&normalized_address)
                                                        })
                                                        .unwrap_or(false)
                                                {
                                                    // Parse amount from data field
                                                    if let Some(data) = log_entry["data"].as_str() {
                                                        if data.len() >= 66 {
                                                            if let Ok(amount_raw) =
                                                                u128::from_str_radix(&data[2..], 16)
                                                            {
                                                                let decimal_divisor =
                                                                    Self::get_usdt_decimal_divisor(
                                                                        &config.name,
                                                                    );
                                                                let token_amount = amount_raw
                                                                    as f64
                                                                    / decimal_divisor;

                                                                log::info!("💰 [TX_VERIFY] Found USDT transfer: {} USDT (expected: {} USDT)", token_amount, expected_amount);

                                                                // Check amount with tolerance
                                                                let tolerance = (expected_amount
                                                                    * 0.05)
                                                                    .max(0.01);
                                                                let amount_matches = (token_amount
                                                                    - expected_amount)
                                                                    .abs()
                                                                    <= tolerance;

                                                                if amount_matches {
                                                                    // Get block info for confirmations
                                                                    let current_block = self
                                                                        .get_current_block_number(
                                                                            config,
                                                                        )
                                                                        .await
                                                                        .unwrap_or(0);
                                                                    let block_number = receipt
                                                                        ["blockNumber"]
                                                                        .as_str()
                                                                        .and_then(|bn| {
                                                                            u64::from_str_radix(
                                                                                &bn[2..],
                                                                                16,
                                                                            )
                                                                            .ok()
                                                                        })
                                                                        .unwrap_or(0);

                                                                    let confirmations =
                                                                        if current_block > 0
                                                                            && block_number > 0
                                                                        {
                                                                            (current_block
                                                                                .saturating_sub(
                                                                                    block_number,
                                                                                )
                                                                                + 1)
                                                                                as u32
                                                                        } else {
                                                                            0
                                                                        };

                                                                    let block_time = self
                                                                        .get_block_timestamp(
                                                                            config,
                                                                            block_number,
                                                                        )
                                                                        .await
                                                                        .unwrap_or_else(|_| {
                                                                            Utc::now()
                                                                        });

                                                                    log::info!("✅ [TX_VERIFY] Transaction verified! {} confirmations", confirmations);

                                                                    return Ok(Some(DetectedTransaction {
                                                                        hash: tx_hash.to_string(),
                                                                        amount: token_amount,
                                                                        currency: Self::get_usdt_currency_by_network(&config.name),
                                                                        block_time,
                                                                        confirmations,
                                                                        status: "confirmed".to_string(),
                                                                        from_address: "unknown".to_string(), // Could parse from topics[1] if needed
                                                                        to_address: to_address.to_string(),
                                                                        network: config.name.clone(),
                                                                    }));
                                                                } else {
                                                                    log::info!(" [TX_VERIFY] Amount mismatch: got {} USDT, expected {} USDT", token_amount, expected_amount);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    log::info!(" [TX_VERIFY] No matching USDT transfer found in transaction");
                                    return Ok(None);
                                } else {
                                    log::warn!(
                                        " [TX_VERIFY] No receipt found for transaction {}",
                                        tx_hash
                                    );
                                    return Ok(None);
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    " [TX_VERIFY] Failed to parse response from {}: {}",
                                    rpc_url,
                                    e
                                );
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!(" [TX_VERIFY] Request failed to {}: {}", rpc_url, e);
                        continue;
                    }
                }
            }

            Err(AppError::ExternalServiceError(
                "All RPC endpoints failed for transaction verification".to_string(),
            ))
        } else {
            Err(AppError::InternalServerError(format!(
                "Network {} not supported",
                network
            )))
        }
    }

    /// Check USDT token transfers via RPC (eth_getLogs)
    async fn check_usdt_via_rpc(
        &self,
        config: &NetworkMonitorConfig,
        contract_address: &str,
        to_address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            "🔗 [USDT_RPC] Checking USDT transfers via RPC for {} USDT to address {}",
            expected_amount,
            to_address
        );

        // Transfer event signature: Transfer(address,address,uint256)
        let transfer_topic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

        // Encode the 'to' address as a topic (pad to 32 bytes) - normalize address
        let normalized_address = if to_address.starts_with("0x") {
            to_address[2..].to_lowercase()
        } else {
            to_address.to_lowercase()
        };
        let to_topic = format!("0x000000000000000000000000{}", normalized_address);

        log::info!(
            "🎯 [USDT_FILTER] Searching for transfers to topic: {} (from address: {})",
            to_topic,
            to_address
        );

        // Calculate from_block based on since_time - BNB Smart Chain has strict limits
        let current_block = self.get_current_block_number(config).await.unwrap_or(0);

        // BNB Smart Chain has strict eth_getLogs limits, use smaller range
        let blocks_to_search = match config.name.as_str() {
            "bnb" => {
                log::info!(
                    " [BNB_RPC] Using limited block range for BNB Smart Chain (last 100 blocks)"
                );
                if current_block > 100 {
                    100
                } else {
                    current_block
                } // Much smaller range for BNB
            }
            _ => {
                // Other networks can handle larger ranges
                if current_block > 5000 {
                    5000
                } else {
                    current_block
                }
            }
        };

        let from_block = if current_block > blocks_to_search {
            format!("0x{:x}", current_block - blocks_to_search)
        } else {
            "earliest".to_string()
        };

        log::info!(
            " [USDT_BLOCKS] Searching blocks from {} to latest (current: {})",
            from_block,
            current_block
        );

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getLogs",
            "params": [{
                "address": contract_address,
                "topics": [
                    transfer_topic,
                    null,
                    to_topic
                ],
                "fromBlock": from_block,
                "toBlock": "latest"
            }],
            "id": 1
        });

        let mut urls = vec![config.rpc_url.clone()];
        urls.extend(config.fallback_urls.clone());

        for rpc_url in urls {
            log::debug!("🌐 [USDT_RPC] Trying RPC: {}", rpc_url);

            match self.client.post(&rpc_url).json(&payload).send().await {
                Ok(response) => {
                    match response.json::<Value>().await {
                        Ok(json) => {
                            if let Some(error) = json.get("error") {
                                log::warn!(" [USDT_RPC] RPC error: {}", error);
                                continue;
                            }

                            if let Some(logs) = json["result"].as_array() {
                                log::info!(
                                    "📋 [USDT_LOGS] Found {} transfer logs matching our filter",
                                    logs.len()
                                );

                                for (i, log_entry) in logs.iter().enumerate() {
                                    log::debug!(
                                        " [USDT_LOG_{}] Processing log entry: {}",
                                        i,
                                        serde_json::to_string(log_entry).unwrap_or_default()
                                    );

                                    // Parse the log entry for USDT transfer
                                    if let Some(tx_hash) = log_entry["transactionHash"].as_str() {
                                        if let Some(block_number_hex) =
                                            log_entry["blockNumber"].as_str()
                                        {
                                            if let Some(data) = log_entry["data"].as_str() {
                                                // Parse the transfer amount from the data field
                                                if data.len() >= 66 {
                                                    // 0x + 64 hex chars
                                                    if let Ok(amount_raw) =
                                                        u128::from_str_radix(&data[2..], 16)
                                                    {
                                                        let decimal_divisor =
                                                            Self::get_usdt_decimal_divisor(
                                                                &config.name,
                                                            );
                                                        let token_amount =
                                                            amount_raw as f64 / decimal_divisor;

                                                        log::info!("💰 [USDT_LOG_{}] Tx: {}, Amount: {} USDT (expected: {} USDT) using {} decimals (divisor: {}, raw: {})",
                                                            i, tx_hash, token_amount, expected_amount, if config.name == "bnb" { "18" } else { "6" }, decimal_divisor, amount_raw);

                                                        // Check amount matching with generous tolerance for USDT payments
                                                        let tolerance =
                                                            (expected_amount * 0.05).max(0.01); // 5% or min 0.01 USDT
                                                        let amount_matches =
                                                            (token_amount - expected_amount).abs()
                                                                <= tolerance
                                                                && token_amount > 0.0;

                                                        log::info!("🎯 [USDT_MATCH_{}] Amount check: {} ± {} = {} (matches: {})", 
                                                            i, expected_amount, tolerance, token_amount, amount_matches);

                                                        if amount_matches {
                                                            // Get block number for confirmations
                                                            let block_number = u64::from_str_radix(
                                                                &block_number_hex[2..],
                                                                16,
                                                            )
                                                            .unwrap_or(0);
                                                            let confirmations = if current_block > 0
                                                                && block_number > 0
                                                            {
                                                                (current_block
                                                                    .saturating_sub(block_number)
                                                                    + 1)
                                                                    as u32
                                                            } else {
                                                                1 // Default for pending
                                                            };

                                                            log::info!("✅ [USDT_RPC_MATCH] USDT payment detected! Tx: {}, Amount: {} USDT, Confirmations: {}", 
                                                                tx_hash, token_amount, confirmations);

                                                            let required_confirmations =
                                                                match config.name.as_str() {
                                                                    "ethereum" => 12, // ETH mainnet
                                                                    "bnb" => 15,      // BSC
                                                                    _ => 12,
                                                                };

                                                            // Get transaction timestamp
                                                            let block_time = self
                                                                .get_block_timestamp(
                                                                    config,
                                                                    block_number,
                                                                )
                                                                .await
                                                                .unwrap_or_else(|_| Utc::now());

                                                            return Ok(Some(DetectedTransaction {
                                                                hash: tx_hash.to_string(),
                                                                amount: token_amount,
                                                                currency: Self::get_usdt_currency_by_network(&config.name),
                                                                block_time,
                                                                confirmations,
                                                                status: if confirmations >= required_confirmations { "confirmed" } else { "pending" }.to_string(),
                                                                from_address: self.extract_from_address_from_log(log_entry).unwrap_or_else(|| "unknown".to_string()),
                                                                to_address: to_address.to_string(),
                                                                network: config.name.clone(),
                                                            }));
                                                        } else {
                                                            log::debug!(" [USDT_MISMATCH_{}] Amount {} USDT doesn't match expected {} USDT (tolerance: {})", 
                                                                i, token_amount, expected_amount, tolerance);
                                                        }
                                                    } else {
                                                        log::warn!(" [USDT_PARSE_{}] Failed to parse amount from data: {}", i, data);
                                                    }
                                                } else {
                                                    log::warn!(" [USDT_DATA_{}] Invalid data length: {} (expected >= 66)", i, data.len());
                                                }
                                            } else {
                                                log::warn!(" [USDT_MISSING_{}] Missing data field in log", i);
                                            }
                                        } else {
                                            log::warn!(
                                                " [USDT_BLOCK_{}] Missing blockNumber in log",
                                                i
                                            );
                                        }
                                    } else {
                                        log::warn!(
                                            " [USDT_HASH_{}] Missing transactionHash in log",
                                            i
                                        );
                                    }
                                }
                            } else {
                                log::info!("📭 [USDT_LOGS] No logs found in result");
                            }

                            return Ok(None); // No matching transfer found
                        }
                        Err(e) => {
                            log::warn!(
                                " [USDT_RPC] Failed to parse response from {}: {}",
                                rpc_url,
                                e
                            );
                            continue;
                        }
                    }
                }
                Err(e) => {
                    log::warn!(" [USDT_RPC] Failed to connect to {}: {}", rpc_url, e);
                    continue;
                }
            }
        }

        Ok(None)
    }

    /// Get block timestamp for transaction
    async fn get_block_timestamp(
        &self,
        config: &NetworkMonitorConfig,
        block_number: u64,
    ) -> Result<DateTime<Utc>, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBlockByNumber",
            "params": [format!("0x{:x}", block_number), false],
            "id": 1
        });

        let json = self
            .make_ethereum_rpc_request_with_fallback(config, &payload)
            .await?;

        if let Some(timestamp_hex) = json["result"]["timestamp"].as_str() {
            if let Ok(timestamp) = u64::from_str_radix(&timestamp_hex[2..], 16) {
                return Ok(DateTime::from_timestamp(timestamp as i64, 0).unwrap_or_else(Utc::now));
            }
        }

        Ok(Utc::now())
    }

    /// Extract from address from Transfer log topics
    fn extract_from_address_from_log(&self, log_entry: &Value) -> Option<String> {
        if let Some(topics) = log_entry["topics"].as_array() {
            if topics.len() >= 2 {
                if let Some(from_topic) = topics[1].as_str() {
                    // Remove leading zeros and add 0x prefix
                    let address = format!("0x{}", &from_topic[26..]);
                    return Some(address);
                }
            }
        }
        None
    }

    /// Get current block number for confirmation calculations
    async fn get_current_block_number(
        &self,
        config: &NetworkMonitorConfig,
    ) -> Result<u64, AppError> {
        if let Some(explorer_api) = &config.explorer_api {
            let api_key_param = if let Some(api_key) = &config.api_key {
                format!("&apikey={}", api_key)
            } else {
                String::new()
            };

            let url = format!(
                "{}?module=proxy&action=eth_blockNumber{}",
                explorer_api, api_key_param
            );

            match self.client.get(&url).send().await {
                Ok(response) => match response.json::<Value>().await {
                    Ok(json) => {
                        if let Some(hex_block) = json["result"].as_str() {
                            return u64::from_str_radix(&hex_block[2..], 16).map_err(|_| {
                                AppError::ExternalServiceError(
                                    "Invalid block number format".to_string(),
                                )
                            });
                        }
                    }
                    Err(_) => {}
                },
                Err(_) => {}
            }
        }

        // Fallback to RPC
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });

        match self
            .client
            .post(&config.rpc_url)
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => match response.json::<Value>().await {
                Ok(json) => {
                    if let Some(hex_block) = json["result"].as_str() {
                        u64::from_str_radix(&hex_block[2..], 16).map_err(|_| {
                            AppError::ExternalServiceError(
                                "Invalid block number format".to_string(),
                            )
                        })
                    } else {
                        Err(AppError::ExternalServiceError(
                            "No result in RPC response".to_string(),
                        ))
                    }
                }
                Err(e) => Err(AppError::ExternalServiceError(format!(
                    "Failed to parse RPC response: {}",
                    e
                ))),
            },
            Err(e) => Err(AppError::ExternalServiceError(format!(
                "RPC request failed: {}",
                e
            ))),
        }
    }

    /// Make RPC request with fallback URLs
    async fn make_rpc_request_with_fallback(
        &self,
        config: &NetworkMonitorConfig,
        payload: &Value,
    ) -> Result<Value, AppError> {
        let mut urls = vec![config.rpc_url.clone()];
        urls.extend(config.fallback_urls.clone());

        let mut last_error = None;

        for (i, url) in urls.iter().enumerate() {
            log::debug!(
                "Trying Solana RPC endpoint {}/{}: {}",
                i + 1,
                urls.len(),
                url
            );

            match self.client.post(url).json(payload).send().await {
                Ok(response) => {
                    match response.text().await {
                        Ok(response_text) => {
                            log::debug!("Solana RPC response from {}: {}", url, response_text);

                            match serde_json::from_str::<Value>(&response_text) {
                                Ok(json) => {
                                    // Check for RPC errors
                                    if let Some(error) = json.get("error") {
                                        let error_code = error
                                            .get("code")
                                            .and_then(|c| c.as_i64())
                                            .unwrap_or(-1);
                                        let error_message = error
                                            .get("message")
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("Unknown error");
                                        let error_data = error
                                            .get("data")
                                            .and_then(|d| d.as_str())
                                            .unwrap_or("");

                                        let error_msg = format!(
                                            "Solana RPC error from {}: {} (code: {}, data: {})",
                                            url, error_message, error_code, error_data
                                        );

                                        // Log detailed error for debugging invalid params
                                        if error_code == -32602 {
                                            log::error!("🚫 [RPC_INVALID_PARAMS] {}", error_msg);
                                            log::error!(
                                                "🔧 [DEBUG] Request payload: {}",
                                                serde_json::to_string_pretty(payload)
                                                    .unwrap_or_default()
                                            );
                                        } else {
                                            log::warn!("  [RPC_ERROR] {}", error_msg);
                                        }

                                        last_error =
                                            Some(AppError::ExternalServiceError(error_msg));
                                        continue;
                                    }

                                    log::info!("✅ Solana RPC success with endpoint: {}", url);
                                    return Ok(json);
                                }
                                Err(e) => {
                                    let error_msg =
                                        format!("Failed to parse response from {}: {}", url, e);
                                    log::warn!("{}", error_msg);
                                    last_error = Some(AppError::InternalServerError(error_msg));
                                }
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to read response from {}: {}", url, e);
                            log::warn!("{}", error_msg);
                            last_error = Some(AppError::InternalServerError(error_msg));
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("Request failed to {}: {}", url, e);
                    log::warn!("{}", error_msg);
                    last_error = Some(AppError::InternalServerError(error_msg));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::InternalServerError("All Solana RPC endpoints failed".to_string())
        }))
    }

    /// Check Solana payments
    async fn check_solana_payment(
        &self,
        config: &NetworkMonitorConfig,
        address: &str,
        expected_amount: f64,
        currency: &str,
        since_time: DateTime<Utc>,
    ) -> Result<Option<DetectedTransaction>, AppError> {
        log::info!(
            " [SOL_CHECK] Checking {} {} payment for address {} (since {})",
            expected_amount,
            currency.to_uppercase(),
            address,
            since_time.to_rfc3339()
        );

        // Get signatures for the address with proper commitment
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignaturesForAddress",
            "params": [
                address,
                {
                    "limit": 50,
                    "commitment": "confirmed"
                }
            ]
        });

        let json = self
            .make_rpc_request_with_fallback(config, &payload)
            .await?;

        // Get result (error handling is done in make_rpc_request_with_fallback)
        let result = json.get("result").ok_or_else(|| {
            AppError::InternalServerError("Missing result in Solana response".to_string())
        })?;

        if result.is_null() {
            // No transactions found for this address (valid response)
            log::debug!("No transactions found for Solana address: {}", address);
            return Ok(None);
        }

        let signatures = result.as_array().ok_or_else(|| {
            AppError::InternalServerError(format!(
                "Expected array in Solana response, got: {}",
                result
            ))
        })?;

        for (idx, sig_info) in signatures.iter().enumerate() {
            if let Some(signature) = sig_info["signature"].as_str() {
                // Check signature timestamp first for early filtering
                let sig_block_time = sig_info["blockTime"]
                    .as_i64()
                    .and_then(|ts| DateTime::from_timestamp(ts, 0))
                    .unwrap_or_else(Utc::now);

                if sig_block_time < since_time {
                    log::debug!(
                        "⏭️  [SOL_SIG] Skipping old signature {} ({})",
                        &signature[0..8],
                        sig_block_time.to_rfc3339()
                    );
                    continue;
                }

                log::debug!(
                    "🔎 [SOL_SIG] Checking signature {}/{}: {} ({})",
                    idx + 1,
                    signatures.len(),
                    &signature[0..8],
                    sig_block_time.to_rfc3339()
                );

                // Get transaction details
                match self.get_solana_transaction_details(config, signature).await {
                    Ok(tx) => {
                        if tx.block_time >= since_time && tx.status == "confirmed" {
                            // Check if amount matches with tolerance
                            let tolerance = expected_amount * 0.01; // 1% tolerance for Solana
                            let amount_diff = (tx.amount - expected_amount).abs();

                            log::info!("💰 [SOL_MATCH] Found tx {} - Amount: {} SOL (expected: {} SOL, diff: {}, tolerance: {})", 
                                &signature[0..8], tx.amount, expected_amount, amount_diff, tolerance);

                            if amount_diff <= tolerance && tx.amount > 0.0 {
                                log::info!(
                                    "✅ [SOL_DETECTED] Payment detected! Tx: {}, Amount: {} SOL",
                                    signature,
                                    tx.amount
                                );

                                return Ok(Some(DetectedTransaction {
                                    hash: signature.to_string(),
                                    amount: tx.amount,
                                    currency: currency.to_uppercase(),
                                    block_time: tx.block_time,
                                    confirmations: tx.confirmations,
                                    status: tx.status,
                                    from_address: tx.from_address,
                                    to_address: address.to_string(),
                                    network: config.name.clone(),
                                }));
                            } else {
                                log::debug!(" [SOL_NOMATCH] Amount mismatch - got {} SOL, expected {} SOL", 
                                    tx.amount, expected_amount);
                            }
                        } else {
                            log::debug!(
                                "⏳ [SOL_PENDING] Transaction {} not confirmed or too old",
                                &signature[0..8]
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "  [SOL_ERROR] Failed to get details for signature {}: {}",
                            &signature[0..8],
                            e
                        );
                        continue;
                    }
                }
            }
        }

        Ok(None)
    }

    async fn get_solana_transaction_details(
        &self,
        config: &NetworkMonitorConfig,
        signature: &str,
    ) -> Result<DetectedTransaction, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [
                signature,
                {
                    "encoding": "json",
                    "maxSupportedTransactionVersion": 0,
                    "commitment": "confirmed"
                }
            ]
        });

        let json = self
            .make_rpc_request_with_fallback(config, &payload)
            .await?;

        let result = json.get("result").ok_or_else(|| {
            AppError::InternalServerError(
                "Missing result in Solana transaction response".to_string(),
            )
        })?;

        if result.is_null() {
            return Err(AppError::NotFound("Transaction not found".to_string()));
        }

        let transaction = &result["transaction"];
        let meta = &result["meta"];

        // Get account keys from transaction
        let empty_account_keys = vec![];
        let account_keys = transaction["message"]["accountKeys"]
            .as_array()
            .unwrap_or(&empty_account_keys);

        let empty_vec = vec![];
        let pre_balances = meta["preBalances"].as_array().unwrap_or(&empty_vec);
        let post_balances = meta["postBalances"].as_array().unwrap_or(&empty_vec);

        // Calculate the maximum balance change (find the recipient)
        let mut max_balance_change = 0i64;
        let mut from_address = "unknown".to_string();
        let mut to_address = "unknown".to_string();

        for i in 0..pre_balances.len().min(post_balances.len()) {
            let pre = pre_balances[i].as_i64().unwrap_or(0);
            let post = post_balances[i].as_i64().unwrap_or(0);
            let balance_change = post - pre;

            if i < account_keys.len() {
                let account = account_keys[i].as_str().unwrap_or("unknown");

                // Track the largest positive balance change (recipient)
                if balance_change > max_balance_change {
                    max_balance_change = balance_change;
                    to_address = account.to_string();
                }

                // Track negative balance changes (sender, excluding fees)
                if balance_change < 0 && from_address == "unknown" {
                    from_address = account.to_string();
                }
            }
        }

        let amount_lamports = max_balance_change;
        let amount_sol = amount_lamports as f64 / 1_000_000_000.0;

        let block_time = result["blockTime"]
            .as_i64()
            .map(|t| DateTime::from_timestamp(t, 0))
            .flatten()
            .unwrap_or_else(Utc::now);

        // Get confirmation count - use confirmed if transaction succeeded
        let confirmations = if meta["err"].is_null() {
            32 // Assume sufficient confirmations for successful transactions
        } else {
            0
        };

        log::debug!(
            " [SOL_TX] {} - Amount: {} SOL, From: {}, To: {}, Confirmations: {}",
            signature,
            amount_sol,
            from_address,
            to_address,
            confirmations
        );

        Ok(DetectedTransaction {
            hash: signature.to_string(),
            amount: amount_sol,
            currency: "SOL".to_string(),
            block_time,
            confirmations,
            status: if meta["err"].is_null() {
                "confirmed"
            } else {
                "failed"
            }
            .to_string(),
            from_address,
            to_address,
            network: config.name.clone(),
        })
    }

    /// Map currency to network
    fn get_network_for_currency(&self, currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => "bitcoin".to_string(),
            "ethereum" | "eth" => "ethereum".to_string(),
            "solana" | "sol" => "solana".to_string(),
            "solana_testnet" => "solana_testnet".to_string(), // Handle Solana testnet explicitly
            "bnb" | "binancecoin" => "bnb".to_string(),
            "matic" | "polygon" => "polygon".to_string(),
            "avax" | "avalanche" => "avalanche".to_string(),
            "base_sepolia" => "base_sepolia".to_string(), // Handle Base Sepolia explicitly
            "usdt" | "usdc" | "usdt_erc20" | "usdt_ethereum" => "ethereum".to_string(), // ERC-20 tokens on Ethereum
            "usdt_bnb" | "usdt_bep20" | "tether_bnb" | "usdt_bsc" | "USDT_BNB" | "USDT_BEP20"
            | "TETHER_BNB" => "bnb".to_string(), // BEP-20 tokens on BNB Chain
            _ => "ethereum".to_string(), // Default fallback
        }
    }

    /// Validate Solana address format
    fn validate_solana_address(address: &str) -> Result<(), AppError> {
        // Check length (Solana addresses are typically 32-44 characters)
        if address.len() < 32 || address.len() > 44 {
            return Err(AppError::ValidationError(format!(
                "Invalid Solana address length: {} (expected 32-44 characters)",
                address.len()
            )));
        }

        // Check if it's valid base58
        match bs58::decode(address).into_vec() {
            Ok(bytes) => {
                // Solana public keys are exactly 32 bytes
                if bytes.len() != 32 {
                    return Err(AppError::ValidationError(format!(
                        "Invalid Solana address: decoded length {} (expected 32 bytes)",
                        bytes.len()
                    )));
                }
            }
            Err(_) => {
                return Err(AppError::ValidationError(format!(
                    "Invalid Solana address: not valid base58 encoding: {}",
                    address
                )));
            }
        }

        // Check for invalid prefixes (like "Sol...")
        if address.starts_with("Sol") || address.starts_with("sol") {
            return Err(AppError::ValidationError(format!(
                "Invalid Solana address: contains invalid prefix 'Sol': {}",
                address
            )));
        }

        Ok(())
    }
}
