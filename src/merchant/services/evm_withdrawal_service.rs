use crate::merchant::models::withdrawal::CreateWithdrawalRequest;
use crate::shared::{
    entities::{merchant, payment_request, wallet, withdrawal_request},
    models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
    utils::{encryption::CryptoEncryption, errors::AppError},
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use web3::{
    transports::Http,
    types::{Address, TransactionParameters, TransactionReceipt, H256, U256},
    Web3,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmWalletCandidate {
    pub wallet_id: String,
    pub address: String,
    pub balance_wei: U256,
    pub estimated_gas_cost: U256,
    pub transferable_wei: U256,
    pub private_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmWalletUsage {
    pub wallet_id: String,
    pub address: String,
    pub transferred_wei: U256,
    pub actual_gas_used: u64,
    pub effective_gas_price_wei: U256,
    pub actual_fee_wei: U256,
    pub tx_hash: String,
    pub explorer_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmWithdrawalResult {
    pub withdrawal_id: String,
    pub status: String, // "processing" initially, "confirmed" after required confirmations
    pub confirmations: u32,
    pub required_confirmations: u32,
    pub amount: String,    // Amount sent to recipient in ETH
    pub fee: String,       // Actual total fee in ETH
    pub net_debit: String, // amount + fee in ETH
    pub wallets_used: Vec<EvmWalletUsage>,
    pub tx_hashes: Vec<String>,
    pub explorer_urls: Vec<String>,
    pub network: String,
    pub environment: String,
}

pub struct EvmWithdrawalService;

impl EvmWithdrawalService {
    /// Get network configuration based on network and environment
    fn get_network_config(network: &str, environment: &str) -> Result<NetworkConfig, AppError> {
        let network_key = format!("{}_{}", network.to_lowercase(), environment);

        match network_key.as_str() {
            "eth_mainnet" => Ok(NetworkConfig {
                name: "ethereum".to_string(),
                rpc_url: std::env::var("ETH_MAINNET_RPC").unwrap_or_else(|_| {
                    "https://mainnet.infura.io/v3/eef650a32682456db1cb76fa3f4e1206".to_string()
                }),
                chain_id: 1,
                required_confirmations: 12, // As specified in the requirements
                decimals: 18,
                explorer_base_url: "https://etherscan.io".to_string(),
                gas_limit_transfer: 21000,
                gas_price_multiplier: 1.05, // 5% buffer
                static_gas_price_wei: Some("2000000000".to_string()), // 2 Gwei for mainnet (reduced for testing)
                max_fee_per_gas_gwei: None, // Will use EIP-1559 if supported
                max_priority_fee_per_gas_gwei: None,
            }),
            "eth_testnet" => Ok(NetworkConfig {
                name: "base_sepolia".to_string(),
                rpc_url: std::env::var("ETH_TESTNET_RPC")
                    .unwrap_or_else(|_| "https://sepolia.base.org".to_string()),
                chain_id: 84532, // Base Sepolia
                required_confirmations: 1,
                decimals: 18,
                explorer_base_url: "https://sepolia.etherscan.io".to_string(),
                gas_limit_transfer: 21000,
                gas_price_multiplier: 1.02, // 2% buffer for testnet
                static_gas_price_wei: Some("1000000000".to_string()), // 1 Gwei for testnet
                max_fee_per_gas_gwei: None,
                max_priority_fee_per_gas_gwei: None,
            }),
            "bnb_mainnet" | "bsc_mainnet" => Ok(NetworkConfig {
                name: "bnb".to_string(),
                rpc_url: std::env::var("BSC_MAINNET_RPC")
                    .unwrap_or_else(|_| "https://bsc-dataseed1.binance.org:443".to_string()),
                chain_id: 56,
                required_confirmations: 3,
                decimals: 18,
                explorer_base_url: "https://bscscan.com".to_string(),
                gas_limit_transfer: 21000,
                gas_price_multiplier: 1.05,
                static_gas_price_wei: Some("5000000000".to_string()), // 5 Gwei for BSC mainnet
                max_fee_per_gas_gwei: None,
                max_priority_fee_per_gas_gwei: None,
            }),
            "bnb_testnet" | "bsc_testnet" => Ok(NetworkConfig {
                name: "bnb_testnet".to_string(),
                rpc_url: std::env::var("BSC_TESTNET_RPC").unwrap_or_else(|_| {
                    "https://data-seed-prebsc-1-s1.binance.org:8545".to_string()
                }),
                chain_id: 97,
                required_confirmations: 1,
                decimals: 18,
                explorer_base_url: "https://testnet.bscscan.com".to_string(),
                gas_limit_transfer: 21000,
                gas_price_multiplier: 1.02,
                static_gas_price_wei: Some("1000000000".to_string()), // 1 Gwei for BSC testnet
                max_fee_per_gas_gwei: None,
                max_priority_fee_per_gas_gwei: None,
            }),
            _ => Err(AppError::ValidationError(format!(
                "Unsupported EVM network: {}_{}",
                network, environment
            ))),
        }
    }

    /// Execute ETH withdrawal with multi-wallet aggregation and proper lifecycle
    pub async fn execute_withdrawal(
        db: &DatabaseConnection,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
        idempotency_key: &str,
    ) -> Result<EvmWithdrawalResult, AppError> {
        log::info!(
            "🔄 Starting ETH withdrawal: {} ETH to {} (network: {}, env: {})",
            request.amount,
            request.to_address,
            request.network,
            request.environment
        );

        // Get network configuration
        let config = Self::get_network_config(&request.network, &request.environment)?;
        log::info!(
            " Network config: chain_id={}, confirmations={}, decimals={}",
            config.chain_id,
            config.required_confirmations,
            config.decimals
        );

        // Parse amount to Wei
        let amount_str = &request.amount;
        let amount_decimal: Decimal = amount_str
            .parse()
            .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

        if amount_decimal <= Decimal::ZERO {
            return Err(AppError::ValidationError(
                "Amount must be greater than zero".to_string(),
            ));
        }

        let amount_wei = Self::decimal_to_wei(amount_decimal, config.decimals)?;
        log::info!(
            "💰 Requested amount: {} ETH = {} Wei",
            amount_decimal,
            amount_wei
        );

        // Setup Web3 connection
        let web3 = Self::create_web3_connection(&config.rpc_url)?;

        // Get all deposit wallets with fresh balances (mirror BNB/SOL logic)
        let mut wallet_candidates = Self::fetch_deposit_wallets_with_balances(
            db,
            merchant_id,
            &request.network,
            &request.environment,
            &web3,
        )
        .await?;

        if wallet_candidates.is_empty() {
            log::error!(
                " No wallet candidates found for {} withdrawal",
                request.network.to_uppercase()
            );
            log::error!("   Merchant ID: {}", merchant_id);
            log::error!("   Network: {}", request.network);
            log::error!("   Environment: {}", request.environment);

            return Err(AppError::ValidationError(format!(
                "No {} deposit wallets found for merchant. Debug: merchant_id={}, network={}, environment={}. Check server logs for detailed wallet discovery process.",
                request.network.to_uppercase(), merchant_id, request.network, request.environment
            )));
        }

        // Estimate gas costs for each wallet
        Self::estimate_gas_costs_for_wallets(&mut wallet_candidates, &web3, &config).await?;

        // Calculate transferable amounts after gas costs
        for wallet in &mut wallet_candidates {
            wallet.transferable_wei = if wallet.balance_wei > wallet.estimated_gas_cost {
                wallet.balance_wei - wallet.estimated_gas_cost
            } else {
                U256::zero()
            };
        }

        // Sort by transferable amount (highest first)
        wallet_candidates.sort_by(|a, b| b.transferable_wei.cmp(&a.transferable_wei));

        // Check if we have enough total transferable balance
        let total_transferable: U256 = wallet_candidates
            .iter()
            .fold(U256::zero(), |acc, w| acc + w.transferable_wei);

        if total_transferable < amount_wei {
            let shortage_wei = amount_wei - total_transferable;
            let shortage_eth = Self::wei_to_decimal(shortage_wei, config.decimals)?;
            let available_eth = Self::wei_to_decimal(total_transferable, config.decimals)?;

            return Err(AppError::ValidationError(format!(
                "Insufficient {} balance after gas fees. Requested: {} {}, Available: {} {}, Shortage: {} {}",
                config.name.to_uppercase(), amount_decimal, request.network.to_uppercase(),
                available_eth, request.network.to_uppercase(),
                shortage_eth, request.network.to_uppercase()
            )));
        }

        // Select wallets to cover the amount (multi-wallet aggregation)
        let selected_wallets = Self::select_wallets_for_amount(&wallet_candidates, amount_wei)?;
        log::info!(
            "🎯 Selected {} wallets for withdrawal",
            selected_wallets.len()
        );

        // Create withdrawal record in "processing" status
        let withdrawal_id = uuid::Uuid::new_v4().to_string();
        let withdrawal_record = Self::create_withdrawal_record(
            db,
            &withdrawal_id,
            merchant_id,
            request,
            idempotency_key,
        )
        .await?;

        // Execute transactions from selected wallets
        let mut wallet_usages = Vec::new();
        let mut tx_hashes = Vec::new();
        let mut explorer_urls = Vec::new();
        let mut remaining_amount = amount_wei;

        for (i, wallet) in selected_wallets.iter().enumerate() {
            let amount_to_send = if i == selected_wallets.len() - 1 {
                // Last wallet sends exactly remaining amount
                remaining_amount
            } else {
                std::cmp::min(wallet.transferable_wei, remaining_amount)
            };

            if amount_to_send == U256::zero() {
                continue;
            }

            log::info!(
                "💸 Sending {} Wei from wallet {} ({}/{})",
                amount_to_send,
                wallet.address,
                i + 1,
                selected_wallets.len()
            );

            let usage = Self::execute_single_transaction(
                &web3,
                &config,
                wallet,
                &request.to_address,
                amount_to_send,
            )
            .await?;

            tx_hashes.push(usage.tx_hash.clone());
            explorer_urls.push(usage.explorer_url.clone());
            wallet_usages.push(usage);

            remaining_amount = remaining_amount.saturating_sub(amount_to_send);
            if remaining_amount == U256::zero() {
                break;
            }
        }

        // Calculate totals
        let total_fee_wei: U256 = wallet_usages
            .iter()
            .fold(U256::zero(), |acc, u| acc + u.actual_fee_wei);
        let total_amount_wei: U256 = wallet_usages
            .iter()
            .fold(U256::zero(), |acc, u| acc + u.transferred_wei);
        let net_debit_wei = total_amount_wei + total_fee_wei;

        let amount_eth = Self::wei_to_decimal(total_amount_wei, config.decimals)?;
        let fee_eth = Self::wei_to_decimal(total_fee_wei, config.decimals)?;
        let net_debit_eth = Self::wei_to_decimal(net_debit_wei, config.decimals)?;

        log::info!(
            "✅ ETH withdrawal completed: {} ETH sent, {} ETH fee, {} ETH total debit",
            amount_eth,
            fee_eth,
            net_debit_eth
        );

        // Update withdrawal record with transaction details
        Self::update_withdrawal_with_transactions(
            db,
            &withdrawal_record,
            &wallet_usages,
            total_amount_wei,
            total_fee_wei,
        )
        .await?;

        Ok(EvmWithdrawalResult {
            withdrawal_id,
            status: "processing".to_string(), // Important: start in processing status
            confirmations: 0,
            required_confirmations: config.required_confirmations,
            amount: amount_eth.to_string(),
            fee: fee_eth.to_string(),
            net_debit: net_debit_eth.to_string(),
            wallets_used: wallet_usages,
            tx_hashes,
            explorer_urls,
            network: request.network.clone(),
            environment: request.environment.clone(),
        })
    }

    /// Create Web3 connection
    fn create_web3_connection(rpc_url: &str) -> Result<Web3<Http>, AppError> {
        let transport = Http::new(rpc_url).map_err(|e| {
            AppError::InternalServerError(format!(
                "Failed to connect to ETH RPC {}: {}",
                rpc_url, e
            ))
        })?;
        Ok(Web3::new(transport))
    }

    /// Fetch deposit wallets with fresh on-chain balances
    async fn fetch_deposit_wallets_with_balances(
        db: &DatabaseConnection,
        merchant_id: &str,
        network: &str,
        environment: &str,
        web3: &Web3<Http>,
    ) -> Result<Vec<EvmWalletCandidate>, AppError> {
        log::info!(" Fetching deposit wallets with fresh balances...");

        // Get merchant's user_id
        let merchant = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

        let user_id = merchant.user_id.clone();

        // Find wallets that received payments (deposit wallets)
        // Note: Payment currency is stored as uppercase (ETH, BNB, etc.)
        log::info!(
            " Searching for payments: merchant_id={}, currency={}, status=paid",
            merchant_id,
            network.to_uppercase()
        );

        let mut payment_query = payment_request::Entity::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Currency.eq(network.to_uppercase()))
            .filter(payment_request::Column::Status.eq("paid"));

        // Add environment filter if it's not "mainnet" (mainnet might be implicit)
        if environment != "mainnet" {
            log::info!(" Adding environment filter: {}", environment);
            payment_query =
                payment_query.filter(payment_request::Column::Environment.eq(environment));
        } else {
            log::info!(" Mainnet environment - not filtering by environment");
        }

        let payments = payment_query.all(db).await?;
        log::info!(" Found {} total payments matching criteria", payments.len());

        // Debug: Show payment details
        for payment in &payments {
            log::info!(
                "💳 Payment: {} - {} {} to {} (env: {})",
                payment.id,
                payment.amount,
                payment.currency,
                payment.wallet_address,
                payment.environment
            );
        }

        let deposit_wallet_addresses: Vec<String> =
            payments.into_iter().map(|p| p.wallet_address).collect();

        log::info!(
            " Extracted {} unique deposit wallet addresses for {} payments",
            deposit_wallet_addresses.len(),
            network.to_uppercase()
        );

        if deposit_wallet_addresses.is_empty() {
            return Ok(Vec::new());
        }

        // Get wallet records for these addresses
        // Try multiple approaches to find wallets

        // Approach 1: Look for payment-funded wallets with currency pattern like "eth_mainnet_<payment_id>"
        let currency_pattern = format!("{}_{}_%", network.to_lowercase(), environment);
        log::info!(
            " Looking for wallets with currency pattern: {}",
            currency_pattern
        );

        let mut wallets = wallet::Entity::find()
            .filter(wallet::Column::UserId.eq(&user_id))
            .filter(wallet::Column::Currency.like(&currency_pattern))
            .filter(wallet::Column::Address.is_in(&deposit_wallet_addresses))
            .all(db)
            .await?;

        log::info!(
            " Found {} wallets matching pattern {} for specific addresses",
            wallets.len(),
            currency_pattern
        );

        // Approach 2: If no wallets found with pattern, look for any wallet with these addresses (broader search)
        if wallets.is_empty() {
            log::info!("🔄 No wallets found with pattern, trying broader search for addresses...");
            wallets = wallet::Entity::find()
                .filter(wallet::Column::UserId.eq(&user_id))
                .filter(wallet::Column::Address.is_in(&deposit_wallet_addresses))
                .all(db)
                .await?;

            log::info!(
                " Found {} wallets matching addresses (any currency)",
                wallets.len()
            );
        }

        // Approach 3: If still no wallets found, look for any ETH-related wallets for this user
        if wallets.is_empty() {
            log::info!(
                "🔄 No wallets found for specific addresses, looking for any ETH wallets..."
            );
            let eth_pattern = format!("{}_%", network.to_lowercase()); // eth_mainnet, eth_testnet, etc.
            wallets = wallet::Entity::find()
                .filter(wallet::Column::UserId.eq(&user_id))
                .filter(wallet::Column::Currency.like(&eth_pattern))
                .all(db)
                .await?;

            log::info!(
                " Found {} total ETH wallets for user (fallback)",
                wallets.len()
            );
        }

        // Debug: Show what wallets we found
        for wallet in &wallets {
            log::info!(
                "🔑 Wallet found: {} (currency: {}, address: {})",
                wallet.id,
                wallet.currency,
                wallet.address
            );
        }

        let mut candidates = Vec::new();
        let encryption = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;

        for wallet in wallets {
            // Decrypt private key
            let private_key = encryption.decrypt(&wallet.private_key).map_err(|e| {
                AppError::InternalServerError(format!("Failed to decrypt private key: {}", e))
            })?;

            // Get fresh on-chain balance
            let address = Address::from_str(&wallet.address).map_err(|_| {
                AppError::ValidationError(format!("Invalid wallet address: {}", wallet.address))
            })?;

            let balance_wei = web3.eth().balance(address, None).await.map_err(|e| {
                AppError::ExternalServiceError(format!(
                    "Failed to get balance for {}: {}",
                    wallet.address, e
                ))
            })?;

            let balance_eth = Self::wei_to_decimal(balance_wei, 18).unwrap_or(Decimal::ZERO);
            log::info!(
                "💰 Wallet {}: {} ETH ({} Wei)",
                wallet.address,
                balance_eth,
                balance_wei
            );

            candidates.push(EvmWalletCandidate {
                wallet_id: wallet.id,
                address: wallet.address,
                balance_wei,
                estimated_gas_cost: U256::zero(), // Will be set later
                transferable_wei: U256::zero(),   // Will be set later
                private_key,
            });
        }

        Ok(candidates)
    }

    /// Estimate gas costs for all wallet candidates
    async fn estimate_gas_costs_for_wallets(
        candidates: &mut [EvmWalletCandidate],
        web3: &Web3<Http>,
        config: &NetworkConfig,
    ) -> Result<(), AppError> {
        log::info!(
            "⛽ Estimating gas costs for {} wallets...",
            candidates.len()
        );

        // Use static gas price if configured, otherwise get dynamic gas price
        let base_gas_price = if let Some(ref static_price_str) = config.static_gas_price_wei {
            let static_price = static_price_str.parse::<u128>().map_err(|_| {
                AppError::ValidationError(format!("Invalid static gas price: {}", static_price_str))
            })?;
            U256::from(static_price)
        } else {
            web3.eth().gas_price().await.map_err(|e| {
                AppError::ExternalServiceError(format!("Failed to get gas price: {}", e))
            })?
        };

        let buffered_gas_price =
            U256::from((base_gas_price.as_u128() as f64 * config.gas_price_multiplier) as u128);
        log::info!(
            "⛽ Gas price - Base: {} Wei, Buffered: {} Wei ({})",
            base_gas_price,
            buffered_gas_price,
            if config.static_gas_price_wei.is_some() {
                "static"
            } else {
                "dynamic"
            }
        );

        for candidate in candidates {
            let gas_cost = buffered_gas_price * U256::from(config.gas_limit_transfer);
            candidate.estimated_gas_cost = gas_cost;

            let gas_eth = Self::wei_to_decimal(gas_cost, config.decimals).unwrap_or(Decimal::ZERO);
            log::debug!(
                "⛽ Wallet {}: estimated gas {} ETH",
                candidate.address,
                gas_eth
            );
        }

        Ok(())
    }

    /// Select wallets to cover the requested amount
    fn select_wallets_for_amount(
        candidates: &[EvmWalletCandidate],
        amount_wei: U256,
    ) -> Result<Vec<&EvmWalletCandidate>, AppError> {
        let mut selected = Vec::new();
        let mut remaining = amount_wei;

        for candidate in candidates {
            if remaining == U256::zero() {
                break;
            }

            if candidate.transferable_wei > U256::zero() {
                selected.push(candidate);
                remaining = remaining.saturating_sub(candidate.transferable_wei);
            }
        }

        if remaining > U256::zero() {
            return Err(AppError::ValidationError(
                "Not enough transferable balance across all wallets".to_string(),
            ));
        }

        Ok(selected)
    }

    /// Execute a single transaction from one wallet
    async fn execute_single_transaction(
        web3: &Web3<Http>,
        config: &NetworkConfig,
        wallet: &EvmWalletCandidate,
        to_address: &str,
        amount_wei: U256,
    ) -> Result<EvmWalletUsage, AppError> {
        log::info!(
            " Executing transaction: {} Wei from {} to {}",
            amount_wei,
            wallet.address,
            to_address
        );

        let from_address = Address::from_str(&wallet.address)
            .map_err(|_| AppError::ValidationError("Invalid sender address".to_string()))?;
        let to_address = Address::from_str(to_address)
            .map_err(|_| AppError::ValidationError("Invalid destination address".to_string()))?;

        // Use static gas price if configured, otherwise get dynamic gas price for transaction
        let gas_price = if let Some(ref static_price_str) = config.static_gas_price_wei {
            let static_price = static_price_str.parse::<u128>().map_err(|_| {
                AppError::ValidationError(format!("Invalid static gas price: {}", static_price_str))
            })?;
            U256::from(static_price)
        } else {
            web3.eth().gas_price().await.map_err(|e| {
                AppError::ExternalServiceError(format!("Failed to get gas price: {}", e))
            })?
        };

        let buffered_gas_price =
            U256::from((gas_price.as_u128() as f64 * config.gas_price_multiplier) as u128);

        // Get nonce
        let nonce = web3
            .eth()
            .transaction_count(from_address, None)
            .await
            .map_err(|e| AppError::ExternalServiceError(format!("Failed to get nonce: {}", e)))?;

        // Build transaction
        let tx = TransactionParameters {
            to: Some(to_address),
            value: amount_wei,
            gas: U256::from(config.gas_limit_transfer),
            gas_price: Some(buffered_gas_price),
            nonce: Some(nonce),
            data: vec![].into(),
            chain_id: Some(config.chain_id),
            ..Default::default()
        };

        // Sign and send transaction
        let key = web3::signing::SecretKey::from_str(&wallet.private_key)
            .map_err(|e| AppError::InternalServerError(format!("Invalid private key: {}", e)))?;
        let signed = web3
            .accounts()
            .sign_transaction(tx, &key)
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to sign transaction: {}", e))
            })?;

        let tx_hash = web3
            .eth()
            .send_raw_transaction(signed.raw_transaction)
            .await
            .map_err(|e| {
                AppError::ExternalServiceError(format!("Failed to send transaction: {}", e))
            })?;

        let tx_hash_str = format!("{:?}", tx_hash);
        log::info!("✅ Transaction sent: {}", tx_hash_str);

        // Wait a moment for transaction to be included
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Get transaction receipt to get actual gas used and effective gas price
        let receipt = web3.eth().transaction_receipt(tx_hash).await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to get transaction receipt: {}", e))
        })?;

        let (actual_gas_used, effective_gas_price, actual_fee) = match receipt {
            Some(receipt) => {
                let gas_used = receipt
                    .gas_used
                    .unwrap_or(U256::from(config.gas_limit_transfer))
                    .as_u64();
                let effective_price = receipt.effective_gas_price.unwrap_or(buffered_gas_price);
                let actual_fee = effective_price * U256::from(gas_used);
                (gas_used, effective_price, actual_fee)
            }
            None => {
                log::warn!(" Transaction receipt not available yet, using estimates");
                (
                    config.gas_limit_transfer,
                    buffered_gas_price,
                    buffered_gas_price * U256::from(config.gas_limit_transfer),
                )
            }
        };

        let explorer_url = format!("{}/tx/{}", config.explorer_base_url, tx_hash_str);

        Ok(EvmWalletUsage {
            wallet_id: wallet.wallet_id.clone(),
            address: wallet.address.clone(),
            transferred_wei: amount_wei,
            actual_gas_used,
            effective_gas_price_wei: effective_gas_price,
            actual_fee_wei: actual_fee,
            tx_hash: tx_hash_str,
            explorer_url,
        })
    }

    /// Create withdrawal record in database
    async fn create_withdrawal_record(
        db: &DatabaseConnection,
        withdrawal_id: &str,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
        idempotency_key: &str,
    ) -> Result<withdrawal_request::Model, AppError> {
        let config = Self::get_network_config(&request.network, &request.environment)?;

        let new_withdrawal = withdrawal_request::ActiveModel {
            id: Set(withdrawal_id.to_string()),
            merchant_id: Set(merchant_id.to_string()),
            external_id: Set(request
                .external_id
                .clone()
                .unwrap_or_else(|| withdrawal_id.to_string())),
            amount: Set(request.amount.parse::<Decimal>().unwrap_or_default()),
            currency: Set(request.network.clone()),
            network: Set(request.network.clone()),
            environment: Set(request.environment.clone()),
            to_address: Set(request.to_address.clone()),
            status: Set("processing".to_string()), // Important: start in processing
            metadata: Set(Some(
                serde_json::json!({
                    "idempotency_key": idempotency_key,
                    "initial_status": "processing",
                    "confirmations": 0
                })
                .to_string(),
            )),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
            finality_required: Set(config.required_confirmations as i32),
            finality_reached_at: Set(None),
            replaced_by_tx: Set(None),
            reorg_depth: Set(0),
            lifecycle_state: Set(WithdrawalLifecycleState::Quoted.to_string()),
            withdrawal_funds_state: Set(WithdrawalFundsState::Unlocked.to_string()),
            ..Default::default()
        };

        let withdrawal = new_withdrawal
            .insert(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to create withdrawal: {}", e)))?;

        Ok(withdrawal)
    }

    /// Update withdrawal record with transaction details
    async fn update_withdrawal_with_transactions(
        db: &DatabaseConnection,
        withdrawal: &withdrawal_request::Model,
        usages: &[EvmWalletUsage],
        total_amount_wei: U256,
        total_fee_wei: U256,
    ) -> Result<(), AppError> {
        let tx_hashes: Vec<String> = usages.iter().map(|u| u.tx_hash.clone()).collect();
        let explorer_urls: Vec<String> = usages.iter().map(|u| u.explorer_url.clone()).collect();

        let mut metadata = if let Some(meta_str) = &withdrawal.metadata {
            serde_json::from_str::<serde_json::Value>(meta_str).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        metadata["tx_hashes"] = serde_json::json!(tx_hashes);
        metadata["explorer_urls"] = serde_json::json!(explorer_urls);
        metadata["total_amount_wei"] = serde_json::json!(total_amount_wei.to_string());
        metadata["total_fee_wei"] = serde_json::json!(total_fee_wei.to_string());
        metadata["wallets_used"] = serde_json::json!(usages);
        metadata["transaction_sent_at"] = serde_json::json!(Utc::now().to_rfc3339());

        let mut active_withdrawal: withdrawal_request::ActiveModel = withdrawal.clone().into();
        active_withdrawal.metadata =
            Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));
        active_withdrawal.updated_at = Set(Utc::now().into());

        active_withdrawal
            .update(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to update withdrawal: {}", e)))?;

        Ok(())
    }

    /// Convert decimal to wei
    fn decimal_to_wei(amount: Decimal, decimals: u8) -> Result<U256, AppError> {
        let scale_factor = Decimal::from(10_u64.pow(decimals as u32));
        let scaled_amount = amount * scale_factor;

        let amount_str = scaled_amount.to_string();
        let (integer_part, _) = if let Some(pos) = amount_str.find('.') {
            (&amount_str[..pos], &amount_str[pos + 1..])
        } else {
            (amount_str.as_str(), "")
        };

        U256::from_dec_str(integer_part)
            .map_err(|_| AppError::ValidationError("Amount too large".to_string()))
    }

    /// Convert wei to decimal
    pub fn wei_to_decimal(wei: U256, decimals: u8) -> Result<Decimal, AppError> {
        let wei_str = wei.to_string();
        let scale_factor = Decimal::from(10_u64.pow(decimals as u32));
        let wei_decimal = wei_str
            .parse::<Decimal>()
            .map_err(|_| AppError::ValidationError("Wei amount too large".to_string()))?;
        Ok(wei_decimal / scale_factor)
    }

    /// Monitor ETH withdrawal transactions and update status when confirmed
    pub async fn monitor_withdrawal_transactions(
        app_state: &crate::shared::AppState,
        withdrawal_id: &str,
    ) -> Result<bool, AppError> {
        let db = &app_state.db;
        log::info!(" Monitoring ETH withdrawal {}", withdrawal_id);

        // Get withdrawal record
        let withdrawal = withdrawal_request::Entity::find_by_id(withdrawal_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("Withdrawal not found".to_string()))?;

        if withdrawal.status == "confirmed" {
            return Ok(true); // Already confirmed
        }

        if withdrawal.status != "processing" {
            return Ok(false); // Not ready for monitoring
        }

        // Parse metadata to get transaction details
        let metadata = if let Some(meta_str) = &withdrawal.metadata {
            serde_json::from_str::<serde_json::Value>(meta_str).unwrap_or_default()
        } else {
            return Ok(false);
        };

        let tx_hashes = metadata["tx_hashes"]
            .as_array()
            .ok_or(AppError::ValidationError(
                "No transaction hashes found".to_string(),
            ))?;

        if tx_hashes.is_empty() {
            return Ok(false);
        }

        // Get network configuration
        let config = Self::get_network_config(&withdrawal.currency, &withdrawal.environment)?;
        let web3 = Self::create_web3_connection(&config.rpc_url)?;

        // Check confirmations for all transactions
        let mut all_confirmed = true;
        let mut total_confirmations = 0u32;

        // Get current block number
        let current_block = web3.eth().block_number().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to get current block: {}", e))
        })?;

        for tx_hash_val in tx_hashes {
            if let Some(tx_hash_str) = tx_hash_val.as_str() {
                let tx_hash = H256::from_str(&tx_hash_str).map_err(|_| {
                    AppError::ValidationError(format!("Invalid transaction hash: {}", tx_hash_str))
                })?;

                // Get transaction receipt
                if let Some(receipt) =
                    web3.eth().transaction_receipt(tx_hash).await.map_err(|e| {
                        AppError::ExternalServiceError(format!(
                            "Failed to get receipt for {}: {}",
                            tx_hash_str, e
                        ))
                    })?
                {
                    if let Some(block_number) = receipt.block_number {
                        let confirmations =
                            (current_block.as_u64() - block_number.as_u64() + 1) as u32;
                        total_confirmations += confirmations;

                        log::info!(
                            " TX {}: {} confirmations (need {})",
                            tx_hash_str,
                            confirmations,
                            config.required_confirmations
                        );

                        if confirmations < config.required_confirmations {
                            all_confirmed = false;
                        }
                    } else {
                        all_confirmed = false; // Transaction not yet in a block
                    }
                } else {
                    all_confirmed = false; // Receipt not available yet
                }
            }
        }

        let avg_confirmations = if tx_hashes.len() > 0 {
            total_confirmations / tx_hashes.len() as u32
        } else {
            0
        };

        // Update confirmation count in database
        let mut updated_metadata = metadata.clone();
        updated_metadata["confirmations"] = serde_json::json!(avg_confirmations);
        updated_metadata["last_checked_at"] = serde_json::json!(Utc::now().to_rfc3339());

        // Extract values before moving withdrawal
        let merchant_id = withdrawal.merchant_id.clone();
        let environment = withdrawal.environment.clone();

        let mut active_withdrawal: withdrawal_request::ActiveModel = withdrawal.into();
        active_withdrawal.blockchain_confirmations = Set(Some(avg_confirmations as i32));
        active_withdrawal.metadata = Set(Some(
            serde_json::to_string(&updated_metadata).unwrap_or_default(),
        ));
        active_withdrawal.updated_at = Set(Utc::now().into());

        // If all transactions are confirmed, mark withdrawal as confirmed
        if all_confirmed {
            log::info!(
                "✅ ETH withdrawal {} confirmed with {} confirmations",
                withdrawal_id,
                avg_confirmations
            );
            active_withdrawal.status = Set("confirmed".to_string());
            active_withdrawal.confirmed_at = Set(Some(Utc::now().into()));
            updated_metadata["confirmed_at"] = serde_json::json!(Utc::now().to_rfc3339());
            active_withdrawal.metadata = Set(Some(
                serde_json::to_string(&updated_metadata).unwrap_or_default(),
            ));

            // Trigger balance sync to update aggregated balances and USD calculations
            if let Err(e) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(
                app_state,
                &merchant_id,
                &environment,
            ).await {
                log::warn!("Failed to sync balances after EVM withdrawal confirmation: {}", e);
                // Don't fail the withdrawal confirmation if balance sync fails
            }
        }

        active_withdrawal
            .update(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to update withdrawal: {}", e)))?;

        Ok(all_confirmed)
    }
}

#[derive(Debug, Clone)]
struct NetworkConfig {
    name: String,
    rpc_url: String,
    chain_id: u64,
    required_confirmations: u32,
    decimals: u8,
    explorer_base_url: String,
    gas_limit_transfer: u64,
    gas_price_multiplier: f64,
    static_gas_price_wei: Option<String>, // Static gas price in Wei string format
    max_fee_per_gas_gwei: Option<u64>,
    max_priority_fee_per_gas_gwei: Option<u64>,
}
