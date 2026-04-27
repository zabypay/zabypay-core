use crate::shared::{
    entities::{merchant, payment_request, wallet, withdrawal_request},
    models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
    service::token_config_service::{GasSponsorConfig, TokenConfig, TokenConfigService},
    utils::{encryption::CryptoEncryption, errors::AppError},
};
use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use web3::{
    transports::Http,
    types::{Address, Bytes, CallRequest, TransactionParameters, U256},
    Web3,
};
// use hex; // Will be used for random tx generation fallback

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdtWalletCandidate {
    pub wallet_id: String,
    pub address: String,
    pub token_balance: U256,       // USDT balance in base units
    pub native_balance: U256,      // ETH/BNB balance for gas
    pub estimated_gas_cost: U256,  // Required gas cost
    pub transferable_amount: U256, // Token amount that can be sent
    pub needs_gas_topup: bool,
    pub gas_topup_amount: U256,
    pub private_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdtWalletUsage {
    pub wallet_id: String,
    pub address: String,
    pub token_sent: U256,
    pub gas_used: u64,
    pub actual_fee: U256,
    pub gas_topup_amount: U256,
    pub tx_hash: String,
    pub gas_topup_tx_hash: Option<String>,
    pub explorer_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdtWithdrawalResult {
    pub withdrawal_id: String,
    pub status: String,
    pub total_token_sent: U256,
    pub total_gas_used: U256,
    pub wallets_used: Vec<UsdtWalletUsage>,
    pub tx_hashes: Vec<String>,
    pub explorer_urls: Vec<String>,
    pub network: String,
    pub environment: String,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdtInsufficientBalanceError {
    pub requested: String,
    pub available: String,
    pub shortage: String,
    pub wallets: Vec<UsdtWalletBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdtWalletBreakdown {
    pub address: String,
    pub token_balance: String,
    pub native_balance: String,
    pub gas_needed: String,
    pub sendable: String,
    pub needs_gas_topup: bool,
}

pub struct UsdtWithdrawalService;

impl UsdtWithdrawalService {
    /// Execute USDT withdrawal with multi-wallet aggregation and gas top-up
    pub async fn execute_withdrawal(
        db: &DatabaseConnection,
        merchant_id: &str,
        currency: &str, // "USDT_ERC20" or "USDT_BEP20"
        environment: &str,
        to_address: &str,
        amount: &str,
        amount_type: &str,
        idempotency_key: &str,
    ) -> Result<UsdtWithdrawalResult, AppError> {
        log::info!(
            "🔄 Starting USDT withdrawal: {} {} to {} (env: {})",
            amount,
            currency,
            to_address,
            environment
        );

        // Validate encryption key is available
        TokenConfigService::validate_encryption_key()?;

        // Get token configuration
        let token_config = TokenConfigService::get_token_config(currency, environment)?;
        log::info!(
            " Token config: network={}, decimals={}, chain_id={}",
            token_config.network,
            token_config.decimals,
            token_config.chain_id
        );

        // Convert amount to base units using proper decimals
        let token_needed =
            TokenConfigService::amount_to_base_units(amount, token_config.decimals, amount_type)?;
        let token_needed_u256 = U256::from_dec_str(&token_needed)
            .map_err(|_| AppError::ValidationError("Amount too large".to_string()))?;

        log::info!(
            "💰 Amount conversion: {} {} = {} base units (decimals: {})",
            amount,
            currency,
            token_needed,
            token_config.decimals
        );

        // Setup Web3 connection
        let web3 = Self::create_web3_connection(&token_config.rpc_url)?;

        // Discover and evaluate wallets with token balances
        let mut wallet_candidates = Self::discover_token_wallets(
            db,
            merchant_id,
            currency,
            environment,
            &web3,
            &token_config,
        )
        .await?;

        if wallet_candidates.is_empty() {
            return Err(AppError::ValidationError(format!(
                "No {} deposit wallets found for merchant. Ensure you have received {} payments first.",
                currency, currency
            )));
        }

        // Estimate gas costs and calculate transferable amounts
        Self::evaluate_wallet_candidates(&mut wallet_candidates, &web3, &token_config).await?;

        // Sort wallets by transferable amount (highest first)
        wallet_candidates.sort_by(|a, b| b.transferable_amount.cmp(&a.transferable_amount));

        // Check total availability
        let total_available: U256 = wallet_candidates
            .iter()
            .map(|w| w.transferable_amount)
            .fold(U256::zero(), |acc, x| acc + x);

        if total_available < token_needed_u256 {
            let shortage = token_needed_u256 - total_available;
            return Err(Self::create_insufficient_balance_error(
                &token_needed,
                &total_available.to_string(),
                &shortage.to_string(),
                &wallet_candidates,
                &token_config,
            ));
        }

        log::info!(
            "✅ Sufficient balance: {} available, {} needed",
            total_available,
            token_needed_u256
        );

        // Select wallets to cover the amount
        let selected_wallets =
            Self::select_wallets_for_amount(&wallet_candidates, token_needed_u256)?;
        log::info!(
            "🎯 Selected {} wallets for withdrawal",
            selected_wallets.len()
        );

        // Create withdrawal record
        let withdrawal_id = uuid::Uuid::new_v4().to_string();
        Self::create_withdrawal_record(
            db,
            &withdrawal_id,
            merchant_id,
            currency,
            environment,
            to_address,
            amount,
            idempotency_key,
            &token_config,
        )
        .await?;

        // Execute gas top-ups and token transfers
        let result = Self::execute_transfers_with_gas_topup(
            &selected_wallets,
            token_needed_u256,
            to_address,
            &web3,
            &token_config,
            currency,
            environment,
            &withdrawal_id,
        )
        .await?;

        // Update withdrawal record with results
        Self::update_withdrawal_record(db, &withdrawal_id, &result).await?;

        log::info!(
            "✅ USDT withdrawal completed: {} tokens from {} wallets",
            result.total_token_sent,
            result.wallets_used.len()
        );

        Ok(result)
    }

    /// Create Web3 connection
    fn create_web3_connection(rpc_url: &str) -> Result<Web3<Http>, AppError> {
        let transport = Http::new(rpc_url).map_err(|e| {
            AppError::InternalServerError(format!("Failed to connect to RPC {}: {}", rpc_url, e))
        })?;
        Ok(Web3::new(transport))
    }

    /// Discover wallets with token balances
    async fn discover_token_wallets(
        db: &DatabaseConnection,
        merchant_id: &str,
        currency: &str,
        environment: &str,
        web3: &Web3<Http>,
        token_config: &TokenConfig,
    ) -> Result<Vec<UsdtWalletCandidate>, AppError> {
        log::info!(" Discovering {} wallets from payments...", currency);

        // Get merchant
        let merchant = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

        // Find payments for this token
        // USDT payments might be stored as "USDT" regardless of ERC-20/BEP-20
        let currency_filters = match currency.to_lowercase().as_str() {
            "usdt_erc20" => vec!["USDT".to_string(), "USDT_ERC20".to_string()],
            "usdt_bep20" => vec!["USDT".to_string(), "USDT_BEP20".to_string()],
            _ => vec![currency.to_uppercase()],
        };

        let mut payment_query = payment_request::Entity::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Currency.is_in(currency_filters))
            .filter(payment_request::Column::Status.eq("paid"));

        if environment != "mainnet" {
            payment_query =
                payment_query.filter(payment_request::Column::Environment.eq(environment));
        }

        let payments = payment_query.all(db).await?;
        log::info!(" Found {} paid {} payments", payments.len(), currency);

        if payments.is_empty() {
            return Ok(Vec::new());
        }

        // Get unique wallet addresses from payments
        let wallet_addresses: Vec<String> = payments
            .into_iter()
            .map(|p| p.wallet_address)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        log::info!(
            "🔑 Found {} unique deposit addresses",
            wallet_addresses.len()
        );

        // Get wallet records with private keys
        let currency_patterns = Self::get_wallet_currency_patterns(currency, environment);
        let mut wallets = Vec::new();

        for pattern in &currency_patterns {
            let pattern_wallets = wallet::Entity::find()
                .filter(wallet::Column::UserId.eq(&merchant.user_id))
                .filter(wallet::Column::Currency.like(format!("%{}%", pattern)))
                .filter(wallet::Column::Address.is_in(&wallet_addresses))
                .all(db)
                .await?;

            log::info!(
                " Found {} wallets matching pattern '{}'",
                pattern_wallets.len(),
                pattern
            );
            wallets.extend(pattern_wallets);
        }

        // Remove duplicates
        wallets.sort_by(|a, b| a.id.cmp(&b.id));
        wallets.dedup_by(|a, b| a.id == b.id);

        log::info!("💼 Total {} wallets to evaluate", wallets.len());

        // Convert to candidates with fresh balances
        let encryption = CryptoEncryption::new().map_err(|e| {
            AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
        })?;

        let mut candidates = Vec::new();
        for wallet in wallets {
            // Decrypt private key
            let private_key = encryption.decrypt(&wallet.private_key).map_err(|e| {
                AppError::InternalServerError(format!("Failed to decrypt private key: {}", e))
            })?;

            let wallet_address = Address::from_str(&wallet.address).map_err(|_| {
                AppError::ValidationError(format!("Invalid wallet address: {}", wallet.address))
            })?;

            // Get fresh on-chain balances
            let token_balance =
                Self::get_token_balance(web3, wallet_address, &token_config.contract_address)
                    .await?;
            let native_balance = web3
                .eth()
                .balance(wallet_address, None)
                .await
                .map_err(|e| {
                    AppError::ExternalServiceError(format!("Failed to get native balance: {}", e))
                })?;

            let token_display = TokenConfigService::base_units_to_amount(
                &token_balance.to_string(),
                token_config.decimals,
            )?;
            let native_display = Self::wei_to_eth_display(native_balance);
            log::info!(
                "💰 Wallet {}: {} {} tokens, {} {} native",
                wallet.address,
                token_display,
                currency,
                native_display,
                token_config.network.to_uppercase()
            );

            if token_balance > U256::zero() {
                candidates.push(UsdtWalletCandidate {
                    wallet_id: wallet.id,
                    address: wallet.address,
                    token_balance,
                    native_balance,
                    estimated_gas_cost: U256::zero(), // Will be set later
                    transferable_amount: U256::zero(), // Will be set later
                    needs_gas_topup: false,           // Will be determined later
                    gas_topup_amount: U256::zero(),
                    private_key,
                });
            }
        }

        Ok(candidates)
    }

    /// Evaluate wallet candidates and determine gas requirements
    async fn evaluate_wallet_candidates(
        candidates: &mut [UsdtWalletCandidate],
        web3: &Web3<Http>,
        token_config: &TokenConfig,
    ) -> Result<(), AppError> {
        log::info!(
            " Evaluating {} wallet candidates for gas requirements...",
            candidates.len()
        );

        // Get current gas price
        let gas_price = web3.eth().gas_price().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to get gas price: {}", e))
        })?;

        // Calculate gas cost with safety buffer
        let gas_limit = U256::from(token_config.gas_limit);
        let safety_multiplier = U256::from(110); // 10% safety buffer
        let gas_cost = (gas_price * gas_limit * safety_multiplier) / U256::from(100);

        log::info!(
            "⛽ Gas estimation: {} wei per transfer (limit: {}, price: {} gwei)",
            gas_cost,
            gas_limit,
            Self::wei_to_gwei_display(gas_price)
        );

        for candidate in candidates.iter_mut() {
            candidate.estimated_gas_cost = gas_cost;

            // For tokens, transferable amount is the token balance
            candidate.transferable_amount = candidate.token_balance;

            // Check if wallet needs gas top-up
            if candidate.native_balance < gas_cost {
                candidate.needs_gas_topup = true;
                candidate.gas_topup_amount =
                    gas_cost - candidate.native_balance + (gas_cost / U256::from(10)); // Add 10% extra buffer

                log::info!(
                    "⛽ Wallet {} needs gas top-up: {} wei",
                    candidate.address,
                    candidate.gas_topup_amount
                );
            } else {
                candidate.needs_gas_topup = false;
                candidate.gas_topup_amount = U256::zero();
                log::info!("✅ Wallet {} has sufficient gas", candidate.address);
            }

            let token_display = TokenConfigService::base_units_to_amount(
                &candidate.transferable_amount.to_string(),
                token_config.decimals,
            )?;
            log::info!(
                "💰 Wallet {}: {} tokens transferable",
                candidate.address,
                token_display
            );
        }

        Ok(())
    }

    /// Select wallets to cover the requested amount
    fn select_wallets_for_amount(
        candidates: &[UsdtWalletCandidate],
        needed_amount: U256,
    ) -> Result<Vec<&UsdtWalletCandidate>, AppError> {
        let mut selected = Vec::new();
        let mut remaining = needed_amount;

        for candidate in candidates {
            if remaining == U256::zero() {
                break;
            }

            if candidate.transferable_amount > U256::zero() {
                selected.push(candidate);
                remaining = remaining.saturating_sub(candidate.transferable_amount);
            }
        }

        if remaining > U256::zero() {
            return Err(AppError::ValidationError(
                "Insufficient transferable token balance across all wallets".to_string(),
            ));
        }

        Ok(selected)
    }

    /// Execute transfers with gas top-up
    async fn execute_transfers_with_gas_topup(
        selected_wallets: &[&UsdtWalletCandidate],
        token_needed: U256,
        to_address: &str,
        web3: &Web3<Http>,
        token_config: &TokenConfig,
        currency: &str,
        environment: &str,
        withdrawal_id: &str,
    ) -> Result<UsdtWithdrawalResult, AppError> {
        log::info!(
            " Executing transfers with gas top-up for {} wallets",
            selected_wallets.len()
        );

        let recipient_address = Address::from_str(to_address)
            .map_err(|_| AppError::ValidationError("Invalid recipient address".to_string()))?;

        let mut wallets_used = Vec::new();
        let mut tx_hashes = Vec::new();
        let mut explorer_urls = Vec::new();
        let mut remaining_needed = token_needed;
        let mut total_gas_used = U256::zero();

        // Get gas sponsor if needed
        let gas_sponsor = if selected_wallets.iter().any(|w| w.needs_gas_topup) {
            Some(TokenConfigService::get_gas_sponsor_config(
                &token_config.network,
                environment,
            )?)
        } else {
            None
        };

        for wallet in selected_wallets {
            if remaining_needed == U256::zero() {
                break;
            }

            let transfer_amount = std::cmp::min(wallet.transferable_amount, remaining_needed);
            if transfer_amount == U256::zero() {
                continue;
            }

            log::info!(
                "💸 Processing wallet {}: {} tokens",
                wallet.address,
                TokenConfigService::base_units_to_amount(
                    &transfer_amount.to_string(),
                    token_config.decimals
                )?
            );

            // Handle gas top-up if needed
            let gas_topup_tx_hash = if wallet.needs_gas_topup {
                match Self::execute_gas_topup(
                    wallet,
                    &gas_sponsor.as_ref().unwrap(),
                    web3,
                    token_config,
                )
                .await
                {
                    Ok(tx_hash) => {
                        log::info!("✅ Gas top-up successful: {}", tx_hash);
                        // Wait for top-up to be included
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        Some(tx_hash)
                    }
                    Err(e) => {
                        log::warn!(
                            " Gas top-up failed for {}: {}. Proceeding anyway.",
                            wallet.address,
                            e
                        );
                        None
                    }
                }
            } else {
                None
            };

            // Execute token transfer
            match Self::execute_token_transfer(
                wallet,
                transfer_amount,
                &recipient_address,
                web3,
                token_config,
            )
            .await
            {
                Ok((tx_hash, gas_used, actual_fee)) => {
                    let explorer_url = format!("{}/tx/{}", token_config.explorer_base_url, tx_hash);

                    wallets_used.push(UsdtWalletUsage {
                        wallet_id: wallet.wallet_id.clone(),
                        address: wallet.address.clone(),
                        token_sent: transfer_amount,
                        gas_used,
                        actual_fee,
                        gas_topup_amount: wallet.gas_topup_amount,
                        tx_hash: tx_hash.clone(),
                        gas_topup_tx_hash,
                        explorer_url: explorer_url.clone(),
                    });

                    tx_hashes.push(tx_hash);
                    explorer_urls.push(explorer_url);
                    total_gas_used += actual_fee;
                    remaining_needed = remaining_needed.saturating_sub(transfer_amount);

                    log::info!(
                        "✅ Transfer successful: {} tokens, gas: {} wei",
                        TokenConfigService::base_units_to_amount(
                            &transfer_amount.to_string(),
                            token_config.decimals
                        )?,
                        actual_fee
                    );
                }
                Err(e) => {
                    log::error!(" Transfer failed from {}: {}", wallet.address, e);
                    // Continue with next wallet instead of failing completely
                }
            }
        }

        if remaining_needed > U256::zero() {
            return Err(AppError::InternalServerError(format!(
                "Could not complete full transfer: {} tokens still needed",
                TokenConfigService::base_units_to_amount(
                    &remaining_needed.to_string(),
                    token_config.decimals
                )?
            )));
        }

        let total_sent = token_needed - remaining_needed;
        Ok(UsdtWithdrawalResult {
            withdrawal_id: withdrawal_id.to_string(),
            status: "processing".to_string(),
            total_token_sent: total_sent,
            total_gas_used,
            wallets_used,
            tx_hashes,
            explorer_urls,
            network: token_config.network.clone(),
            environment: environment.to_string(),
            currency: currency.to_string(),
        })
    }

    /// Execute gas top-up from sponsor wallet
    async fn execute_gas_topup(
        target_wallet: &UsdtWalletCandidate,
        sponsor_config: &GasSponsorConfig,
        web3: &Web3<Http>,
        token_config: &TokenConfig,
    ) -> Result<String, AppError> {
        log::info!(
            "⛽ Executing gas top-up: {} wei to {}",
            target_wallet.gas_topup_amount,
            target_wallet.address
        );

        let sponsor_address = Address::from_str(&sponsor_config.sponsor_address)
            .map_err(|_| AppError::ValidationError("Invalid sponsor address".to_string()))?;
        let target_address = Address::from_str(&target_wallet.address)
            .map_err(|_| AppError::ValidationError("Invalid target address".to_string()))?;

        // Check sponsor balance
        let sponsor_balance = web3
            .eth()
            .balance(sponsor_address, None)
            .await
            .map_err(|e| {
                AppError::ExternalServiceError(format!("Failed to get sponsor balance: {}", e))
            })?;

        let required_amount =
            target_wallet.gas_topup_amount + U256::from(21000u64 * 20_000_000_000u64); // topup + gas for topup tx
        if sponsor_balance < required_amount {
            return Err(AppError::InternalServerError(format!(
                "Sponsor wallet insufficient balance: {} < {} wei",
                sponsor_balance, required_amount
            )));
        }

        // Get gas price and nonce
        let gas_price = web3.eth().gas_price().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to get gas price: {}", e))
        })?;
        let nonce = web3
            .eth()
            .transaction_count(sponsor_address, None)
            .await
            .map_err(|e| {
                AppError::ExternalServiceError(format!("Failed to get sponsor nonce: {}", e))
            })?;

        // Build transaction
        let tx = TransactionParameters {
            to: Some(target_address),
            value: target_wallet.gas_topup_amount,
            gas: U256::from(21000),
            gas_price: Some(gas_price),
            nonce: Some(nonce),
            data: vec![].into(),
            chain_id: Some(token_config.chain_id),
            ..Default::default()
        };

        // Sign and send
        Self::sign_and_send_transaction(web3, tx, &sponsor_config.sponsor_private_key).await
    }

    /// Execute token transfer using ERC-20/BEP-20 transfer method
    async fn execute_token_transfer(
        wallet: &UsdtWalletCandidate,
        amount: U256,
        to_address: &Address,
        web3: &Web3<Http>,
        token_config: &TokenConfig,
    ) -> Result<(String, u64, U256), AppError> {
        log::info!(
            "🔄 Executing token transfer: {} units from {} to {}",
            amount,
            wallet.address,
            to_address
        );

        let from_address = Address::from_str(&wallet.address)
            .map_err(|_| AppError::ValidationError("Invalid sender address".to_string()))?;
        let contract_address = Address::from_str(&token_config.contract_address)
            .map_err(|_| AppError::ValidationError("Invalid contract address".to_string()))?;

        // Build ERC-20 transfer call data
        let call_data = Self::encode_transfer_call_data(to_address, amount)?;

        // Get gas price and nonce
        let gas_price = web3.eth().gas_price().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to get gas price: {}", e))
        })?;
        let nonce = web3
            .eth()
            .transaction_count(from_address, None)
            .await
            .map_err(|e| AppError::ExternalServiceError(format!("Failed to get nonce: {}", e)))?;

        // Build transaction
        let tx = TransactionParameters {
            to: Some(contract_address),
            value: U256::zero(), // No ETH/BNB value for token transfer
            gas: U256::from(token_config.gas_limit),
            gas_price: Some(gas_price),
            nonce: Some(nonce),
            data: call_data.into(),
            chain_id: Some(token_config.chain_id),
            ..Default::default()
        };

        // Sign and send
        let tx_hash = Self::sign_and_send_transaction(web3, tx, &wallet.private_key).await?;

        // For now, estimate actual gas usage (in production, wait for receipt)
        let estimated_gas_used = token_config.gas_limit;
        let actual_fee = gas_price * U256::from(estimated_gas_used);

        Ok((tx_hash, estimated_gas_used, actual_fee))
    }

    /// Encode ERC-20 transfer call data
    fn encode_transfer_call_data(to_address: &Address, amount: U256) -> Result<Vec<u8>, AppError> {
        // ERC-20 transfer function selector: 0xa9059cbb
        let function_selector = [0xa9, 0x05, 0x9c, 0xbb];

        // Encode recipient address (32 bytes, left-padded)
        let mut recipient_bytes = [0u8; 32];
        to_address
            .to_fixed_bytes()
            .iter()
            .enumerate()
            .for_each(|(i, &b)| {
                recipient_bytes[12 + i] = b;
            });

        // Encode amount (32 bytes, big-endian)
        let mut amount_bytes = [0u8; 32];
        amount.to_big_endian(&mut amount_bytes);

        // Construct call data
        let mut call_data = Vec::with_capacity(68); // 4 + 32 + 32
        call_data.extend_from_slice(&function_selector);
        call_data.extend_from_slice(&recipient_bytes);
        call_data.extend_from_slice(&amount_bytes);

        Ok(call_data)
    }

    /// Sign and send transaction
    async fn sign_and_send_transaction(
        web3: &Web3<Http>,
        tx: TransactionParameters,
        private_key_hex: &str,
    ) -> Result<String, AppError> {
        let key = web3::signing::SecretKey::from_str(private_key_hex)
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

        Ok(format!("{:?}", tx_hash))
    }

    /// Get token balance using ERC-20 balanceOf call
    async fn get_token_balance(
        web3: &Web3<Http>,
        wallet_address: Address,
        contract_address: &str,
    ) -> Result<U256, AppError> {
        let contract_addr = Address::from_str(contract_address)
            .map_err(|_| AppError::ValidationError("Invalid contract address".to_string()))?;

        // ERC-20 balanceOf function selector: 0x70a08231
        let function_selector = [0x70, 0xa0, 0x82, 0x31];

        // Encode wallet address (32 bytes, left-padded)
        let mut wallet_bytes = [0u8; 32];
        wallet_address
            .to_fixed_bytes()
            .iter()
            .enumerate()
            .for_each(|(i, &b)| {
                wallet_bytes[12 + i] = b;
            });

        // Construct call data
        let mut call_data = Vec::new();
        call_data.extend_from_slice(&function_selector);
        call_data.extend_from_slice(&wallet_bytes);

        let call_request = CallRequest {
            from: None,
            to: Some(contract_addr),
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            value: None,
            data: Some(Bytes(call_data)),
            access_list: None,
            transaction_type: None,
        };

        match web3.eth().call(call_request, None).await {
            Ok(result) => {
                if result.0.len() >= 32 {
                    let balance_bytes = &result.0[result.0.len() - 32..];
                    let balance = U256::from_big_endian(balance_bytes);
                    Ok(balance)
                } else {
                    Ok(U256::zero())
                }
            }
            Err(e) => {
                log::warn!("Failed to get token balance: {}", e);
                Ok(U256::zero())
            }
        }
    }

    /// Create insufficient balance error with detailed breakdown
    fn create_insufficient_balance_error(
        requested: &str,
        available: &str,
        shortage: &str,
        wallets: &[UsdtWalletCandidate],
        token_config: &TokenConfig,
    ) -> AppError {
        let wallet_breakdowns: Vec<UsdtWalletBreakdown> = wallets
            .iter()
            .map(|w| {
                let token_display = TokenConfigService::base_units_to_amount(
                    &w.token_balance.to_string(),
                    token_config.decimals,
                )
                .unwrap_or("0".to_string());
                let sendable_display = TokenConfigService::base_units_to_amount(
                    &w.transferable_amount.to_string(),
                    token_config.decimals,
                )
                .unwrap_or("0".to_string());

                UsdtWalletBreakdown {
                    address: w.address.clone(),
                    token_balance: token_display,
                    native_balance: Self::wei_to_eth_display(w.native_balance),
                    gas_needed: Self::wei_to_eth_display(w.estimated_gas_cost),
                    sendable: sendable_display,
                    needs_gas_topup: w.needs_gas_topup,
                }
            })
            .collect();

        let error_data = UsdtInsufficientBalanceError {
            requested: TokenConfigService::base_units_to_amount(requested, token_config.decimals)
                .unwrap_or(requested.to_string()),
            available: TokenConfigService::base_units_to_amount(available, token_config.decimals)
                .unwrap_or(available.to_string()),
            shortage: TokenConfigService::base_units_to_amount(shortage, token_config.decimals)
                .unwrap_or(shortage.to_string()),
            wallets: wallet_breakdowns,
        };

        AppError::ValidationError(format!(
            "USDT_{}_INSUFFICIENT: {}",
            if token_config.network == "ethereum" {
                "ERC20"
            } else {
                "BEP20"
            },
            serde_json::to_string(&error_data).unwrap_or_default()
        ))
    }

    /// Get wallet currency patterns for discovery
    fn get_wallet_currency_patterns(currency: &str, environment: &str) -> Vec<String> {
        match currency.to_lowercase().as_str() {
            "usdt_erc20" => vec![
                format!("usdt_{}", environment),
                format!("usdt_erc20_{}", environment),
            ],
            "usdt_bep20" => vec![
                format!("usdt_{}", environment),
                format!("usdt_bep20_{}", environment),
                format!("usdt_bnb_{}", environment),
            ],
            _ => vec![format!("{}_{}", currency.to_lowercase(), environment)],
        }
    }

    /// Create withdrawal record in database
    async fn create_withdrawal_record(
        db: &DatabaseConnection,
        withdrawal_id: &str,
        merchant_id: &str,
        currency: &str,
        environment: &str,
        to_address: &str,
        amount: &str,
        idempotency_key: &str,
        token_config: &TokenConfig,
    ) -> Result<(), AppError> {
        let withdrawal = withdrawal_request::ActiveModel {
            id: Set(withdrawal_id.to_string()),
            merchant_id: Set(merchant_id.to_string()),
            wallet_id: Set("multi_wallet".to_string()), // Multi-wallet indicator
            external_id: Set(withdrawal_id.to_string()),
            idempotency_key: Set(idempotency_key.to_string()),
            environment: Set(environment.to_string()),
            network: Set(currency.to_string()),
            currency: Set(currency.to_string()),
            to_address: Set(to_address.to_string()),
            amount: Set(amount.parse::<Decimal>().unwrap_or_default()),
            fee: Set(None), // Will be updated after execution
            net_amount: Set(amount.parse::<Decimal>().unwrap_or_default()),
            status: Set("processing".to_string()),
            required_confirmations: Set(token_config.required_confirmations as i32),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
            finality_required: Set(token_config.required_confirmations as i32),
            finality_reached_at: Set(None),
            replaced_by_tx: Set(None),
            reorg_depth: Set(0),
            lifecycle_state: Set(WithdrawalLifecycleState::Quoted.to_string()),
            withdrawal_funds_state: Set(WithdrawalFundsState::Unlocked.to_string()),
            ..Default::default()
        };

        withdrawal.insert(db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to create withdrawal record: {}", e))
        })?;

        Ok(())
    }

    /// Update withdrawal record with results
    async fn update_withdrawal_record(
        db: &DatabaseConnection,
        withdrawal_id: &str,
        result: &UsdtWithdrawalResult,
    ) -> Result<(), AppError> {
        use serde_json::json;

        let withdrawal = withdrawal_request::Entity::find_by_id(withdrawal_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("Withdrawal not found".to_string()))?;

        let metadata = json!({
            "tx_hashes": result.tx_hashes,
            "explorer_urls": result.explorer_urls,
            "wallets_used": result.wallets_used,
            "total_token_sent": result.total_token_sent.to_string(),
            "total_gas_used": result.total_gas_used.to_string(),
            "multi_wallet": true,
        });

        let mut active: withdrawal_request::ActiveModel = withdrawal.into();
        active.tx_hash = Set(result.tx_hashes.first().cloned());
        active.metadata = Set(Some(metadata.to_string()));
        active.broadcast_at = Set(Some(Utc::now().into()));
        active.updated_at = Set(Utc::now().into());

        active.update(db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to update withdrawal record: {}", e))
        })?;

        Ok(())
    }

    /// Helper: Convert Wei to ETH display
    fn wei_to_eth_display(wei: U256) -> String {
        let eth_value = wei.as_u128() as f64 / 1e18;
        format!("{:.6}", eth_value)
    }

    /// Helper: Convert Wei to Gwei display
    fn wei_to_gwei_display(wei: U256) -> String {
        let gwei_value = wei.as_u128() as f64 / 1e9;
        format!("{:.2}", gwei_value)
    }
}
