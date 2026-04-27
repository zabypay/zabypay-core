use actix_web::{web, Error, HttpMessage, HttpRequest, HttpResponse};
use validator::{Validate, ValidationErrors};

use crate::shared::service::solana_wallet::{
    estimate_transfer_fee, get_rent_exemption_amount, get_sol_balance,
    get_solana_keypair_from_mnemonic, transfer_sol_with_confirmation,
};
use crate::shared::service::{
    token_config_service::TokenConfigService, usd_conversion_service::UsdConversionService,
    usdt_withdrawal_service::UsdtWithdrawalService,
};
use crate::{
    merchant::{
        models::withdrawal::{
            BalanceQuery, BalancesResponse, CreateWithdrawalRequest, FeeEstimateRequest,
            MaxWithdrawableRequest, MaxWithdrawableResponse, MultiWalletBalanceResponse,
            MultiWalletWithdrawalRequest, MultiWalletWithdrawalResponse, WithdrawalListQuery,
            WithdrawalPreview, WithdrawalResponse,
        },
        services::{
            BtcMultiWalletService, EvmMultiWalletService, MultiWalletProcessor, MultiWalletService,
            SolanaMultiWalletService, WithdrawalService,
        },
    },
    shared::models::{auth::AuthContext, temp_auth::UserClaims},
    shared::{
        entities::{merchant, payment_request, prelude::*, wallet, withdrawal_request},
        models::withdrawal_state::{WithdrawalFundsState, WithdrawalLifecycleState},
        service::withdrawal_processor::WithdrawalProcessor,
        utils::errors::AppError,
        AppState,
    },
};
use bip39::{Language, Mnemonic, Seed};
use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;
use std::str::FromStr as DecimalFromStr;

#[derive(Debug, Clone)]
pub struct MerchantAuth {
    pub merchant_id: String,
    pub user_id: String,
}

impl actix_web::FromRequest for MerchantAuth {
    type Error = AppError;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        let req = req.clone();

        Box::pin(async move {
            let claims = req
                .extensions()
                .get::<UserClaims>()
                .cloned()
                .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

            let app_state = req
                .app_data::<web::Data<AppState>>()
                .ok_or_else(|| AppError::DatabaseError("App state not found".to_string()))?;

            let merchants = Merchant::find()
                .filter(merchant::Column::UserId.eq(&claims.id))
                .all(&app_state.db)
                .await
                .map_err(|e| AppError::DatabaseError(format!("Failed to find merchants: {}", e)))?;

            if merchants.is_empty() {
                return Err(AppError::Unauthorized(
                    "No merchants found for user".to_string(),
                ));
            }

            let merchant = &merchants[0];

            Ok(MerchantAuth {
                merchant_id: merchant.id.clone(),
                user_id: claims.id,
            })
        })
    }
}

pub async fn get_balances(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<BalanceQuery>,
) -> Result<HttpResponse, AppError> {
    log::info!("get_balances called with query: {:?}", query);

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!("User claims found: user_id={}", claims.id);

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            log::error!("Database error finding merchants: {:?}", e);
            AppError::DatabaseError(format!("Failed to find merchants: {}", e))
        })?;

    let environment = query.environment.as_deref().unwrap_or("testnet");
    log::info!("Environment: {}", environment);

    if !matches!(environment, "mainnet" | "testnet") {
        log::warn!("Invalid environment: {}", environment);
        return Err(AppError::ValidationError(
            "Environment must be 'mainnet' or 'testnet'".to_string(),
        ));
    }

    if merchants.is_empty() {
        log::warn!("No merchants found for user {}", claims.id);
        return Ok(HttpResponse::Ok().json(serde_json::json!({
            "environment": environment,
            "balances": [],
            "total_usd_value": "0.00",
            "last_updated": chrono::Utc::now(),
            "message": "No merchant account found. Please complete your merchant setup.",
            "setup_required": true
        })));
    }

    let merchant = &merchants[0];
    log::info!("Using merchant {} for user {}", merchant.id, claims.id);

    let balances = WithdrawalService::get_balances(&app_state, &merchant.id, environment)
        .await
        .map_err(|e| {
            log::error!("WithdrawalService::get_balances failed: {:?}", e);
            e
        })?;

    log::info!(
        "Successfully retrieved balances with {} balance entries",
        balances.balances.len()
    );
    Ok(HttpResponse::Ok().json(balances))
}

pub async fn create_withdrawal(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<CreateWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    log::info!("create_withdrawal called with body: {:?}", body);

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!(
        "User claims found: user_id={}, merchant_id={:?}",
        claims.id,
        claims.merchant_id
    );

    if TokenConfigService::is_supported_token(&body.network) {
        return handle_usdt_withdrawal(app_state, claims, body).await;
    }

    body.validate().map_err(|e| {
        log::error!("Validation error: {:?}", e);
        AppError::ValidationError(format!("Invalid request: {}", e))
    })?;

    log::info!("Request validation passed");

    let merchant = match &claims.merchant_id {
        Some(merchant_id) => {
            log::info!("Using merchant_id from JWT: {}", merchant_id);
            let merchant = Merchant::find()
                .filter(merchant::Column::Id.eq(merchant_id))
                .one(&app_state.db)
                .await
                .map_err(|e| {
                    log::error!("Database error finding merchant {}: {:?}", merchant_id, e);
                    AppError::DatabaseError(format!("Failed to find merchant: {}", e))
                })?;

            match merchant {
                Some(m) => m,
                None => {
                    log::warn!("Merchant {} not found in database", merchant_id);
                    return Err(AppError::BadRequest("Merchant account not found. Please complete your merchant setup to create withdrawals.".to_string()));
                }
            }
        }
        None => {
            log::warn!(
                "No merchant_id in JWT claims, falling back to user lookup for user {}",
                claims.id
            );

            let merchants = Merchant::find()
                .filter(merchant::Column::UserId.eq(&claims.id))
                .all(&app_state.db)
                .await
                .map_err(|e| {
                    log::error!(
                        "Database error finding merchants for user {}: {:?}",
                        claims.id,
                        e
                    );
                    AppError::DatabaseError(format!("Failed to find merchants: {}", e))
                })?;

            if merchants.is_empty() {
                log::warn!("No merchants found for user {}", claims.id);
                return Err(AppError::BadRequest("No merchant account found. Please complete your merchant setup to create withdrawals.".to_string()));
            }

            let merchant = &merchants[0];
            log::info!(
                "Using first merchant {} for user {}",
                merchant.id,
                claims.id
            );
            merchant.clone()
        }
    };

    log::info!("Using merchant {} for withdrawal creation", merchant.id);
    log::info!(
        "DEBUG: Merchant details - id: {}, user_id: {}",
        merchant.id,
        merchant.user_id
    );

    let merchant_exists = Merchant::find()
        .filter(merchant::Column::Id.eq(&merchant.id))
        .one(&app_state.db)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to verify merchant: {}", e)))?;

    if merchant_exists.is_none() {
        log::error!("CRITICAL: Merchant {} not found in database!", merchant.id);
        return Err(AppError::DatabaseError(
            "Merchant validation failed - merchant not found".to_string(),
        ));
    }
    log::info!("   Merchant {} verified in database", merchant.id);

    let request_body = body.into_inner();

    let currency_pattern = format!(
        "{}_{}",
        request_body.network.to_lowercase(),
        request_body.environment.to_lowercase()
    );

    log::info!(
        " Searching for ACTIVE wallets (with paid payments) with currency pattern: {}",
        currency_pattern
    );

    let mut wallets = wallet::Entity::find()
        .filter(wallet::Column::UserId.eq(&claims.id))
        .filter(wallet::Column::Currency.contains(&currency_pattern))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            log::error!("Database error finding wallets: {:?}", e);
            AppError::DatabaseError(format!("Failed to find wallets: {}", e))
        })?;

    log::info!(
        " Found {} existing wallets for network {} in {} environment",
        wallets.len(),
        request_body.network,
        request_body.environment
    );

    if wallets.is_empty() {
        log::info!(
            "No wallets found, creating new {} wallet for {} environment",
            request_body.network.to_uppercase(),
            request_body.environment
        );

        use crate::merchant::models::merchant::MerchantEnvironmentType;
        use crate::merchant::services::wallet_generation_service::WalletGenerationService;

        let env_type = if request_body.environment == "mainnet" {
            MerchantEnvironmentType::Mainnet
        } else {
            MerchantEnvironmentType::Testnet
        };

        match WalletGenerationService::ensure_wallet_for_payment(
            &app_state.db,
            &claims.id,
            &request_body.network,
            Some(env_type),
        )
        .await
        {
            Ok(wallet_address) => {
                log::info!(
                    "   Created new wallet: {} for {}",
                    wallet_address,
                    request_body.network
                );

                let new_wallet = wallet::Entity::find()
                    .filter(wallet::Column::UserId.eq(&claims.id))
                    .filter(wallet::Column::Address.eq(&wallet_address))
                    .one(&app_state.db)
                    .await
                    .map_err(|e| {
                        AppError::DatabaseError(format!("Failed to fetch new wallet: {}", e))
                    })?
                    .ok_or_else(|| {
                        AppError::InternalServerError(
                            "New wallet not found after creation".to_string(),
                        )
                    })?;

                wallets.push(new_wallet);
            }
            Err(e) => {
                log::error!(" Failed to create new wallet: {:?}", e);
                return Err(AppError::InternalServerError(format!(
                    "Failed to create wallet: {}",
                    e
                )));
            }
        }
    }

    println!("{:#?} Wallets \n", wallets);

    if wallets.is_empty() {
        log::warn!(
            "No wallets found for user {} with network {} and environment {}",
            claims.id,
            request_body.network,
            request_body.environment
        );
        return Err(AppError::BadRequest(format!(
            "No {} wallets found for {} environment. Please create a wallet first.",
            request_body.network.to_uppercase(),
            request_body.environment
        )));
    }

    match request_body.network.to_lowercase().as_str() {
        "eth" | "ethereum" => {
            log::info!(
                "🔷 Processing Ethereum withdrawal - going directly to multi-wallet aggregation"
            );
            return MerchantAuth::try_evm_multi_wallet_withdrawal_fallback(
                &app_state,
                &merchant.id,
                &request_body,
            )
            .await;
        }
        "usdt_erc20" | "usdt" => {
            log::warn!("USDT withdrawals are disabled in this build");
            return Err(AppError::ValidationError(
                "USDT is not enabled in this build. Supported networks: ETH, BNB, SOL.".to_string(),
            ));
        }
        "bnb" | "bsc" => {
            log::info!(
                "🟡 Processing BNB/BSC withdrawal - going directly to multi-wallet aggregation"
            );
            return MerchantAuth::try_evm_multi_wallet_withdrawal_fallback(
                &app_state,
                &merchant.id,
                &request_body,
            )
            .await;
        }
        "usdt_bep20" | "usdt_bnb" => {
            log::warn!("USDT (BEP-20) withdrawals are disabled in this build");
            return Err(AppError::ValidationError(
                "USDT is not enabled in this build. Supported networks: ETH, BNB, SOL.".to_string(),
            ));
        }
        "btc" | "bitcoin" => {
            log::warn!("Bitcoin withdrawals are disabled in this build");
            return Err(AppError::ValidationError(
                "Bitcoin is not enabled in this build. Supported networks: ETH, BNB, SOL."
                    .to_string(),
            ));
        }
        "sol" | "solana" => {
            log::info!("🟣 Processing Solana withdrawal - continuing with existing logic");
        }
        _ => {
            log::warn!("Unsupported network: {}", request_body.network);
            return Err(AppError::ValidationError(format!(
                "Unsupported network: {}. Supported networks: ETH, BNB, SOL (native currencies only).",
                request_body.network
            )));
        }
    }

    log::info!(
        "   Found {} ACTIVE wallet(s) with paid payments for network {} in {} environment",
        wallets.len(),
        request_body.network,
        request_body.environment
    );

    if wallets.len() > 0 {
        log::info!("💡 OPTIMIZATION: Filtered out empty/unused wallets - only processing {} wallets with actual balances", wallets.len());
    }

    let crypto = crate::shared::utils::encryption::CryptoEncryption::new().map_err(|e| {
        AppError::InternalServerError(format!("Failed to initialize encryption: {}", e))
    })?;

    let mut wallet_details = Vec::new();
    let mut successful_withdrawal = false;

    for (wallet_index, wallet) in wallets.iter().enumerate() {
        println!(
            "🔐 Processing wallet {}/{}: {} ({})",
            wallet_index + 1,
            wallets.len(),
            wallet.address,
            wallet.currency
        );

        let decrypted_mnemonic = if !wallet.mnemonic.is_empty() {
            match crypto.decrypt_mnemonic(&wallet.mnemonic) {
                Ok(mnemonic) => {
                    log::info!(
                        "   Successfully decrypted mnemonic for wallet {}",
                        wallet.address
                    );
                    Some(mnemonic)
                }
                Err(e) => {
                    log::error!(
                        " Failed to decrypt mnemonic for wallet {}: {}",
                        wallet.address,
                        e
                    );
                    None
                }
            }
        } else {
            log::warn!("  No mnemonic stored for wallet {}", wallet.address);
            None
        };

        println!("{:?} decrypted_mnemonic\n", decrypted_mnemonic);

        let decrypted_private_key = if !wallet.private_key.is_empty() {
            match crypto.decrypt_private_key(&wallet.private_key) {
                Ok(private_key) => {
                    log::info!(
                        "Successfully decrypted private key for wallet {}",
                        wallet.address
                    );
                    Some(private_key)
                }
                Err(e) => {
                    log::error!(
                        "Failed to decrypt private key for wallet {}: {}",
                        wallet.address,
                        e
                    );
                    None
                }
            }
        } else {
            log::warn!("No private key stored for wallet {}", wallet.address);
            None
        };
        println!("{:?} decrypted_private_key\n", decrypted_private_key);

        if decrypted_mnemonic.is_none() || decrypted_private_key.is_none() {
            log::warn!("Wallet {} missing credentials (mnemonic: {}, private_key: {}) - skipping to next wallet",
                      wallet.address,
                      decrypted_mnemonic.is_some(),
                      decrypted_private_key.is_some());
            continue;
        }

        if let (Some(mnemonic), Some(_private_key)) = (&decrypted_mnemonic, &decrypted_private_key)
        {
            log::info!("Starting Solana withdrawal transaction...");

            let amount_str = &request_body.amount;
            let amount_sol: f64 = amount_str
                .parse()
                .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;
            let amount_lamports = (amount_sol * 1_000_000_000.0) as u64;

            log::info!(
                "💸 Withdrawal details: {} SOL ({} lamports) to {}",
                amount_sol,
                amount_lamports,
                request_body.to_address
            );

            let sender_keypair = get_solana_keypair_from_mnemonic(mnemonic, 0);
            let sender_address = sender_keypair.pubkey().to_string();

            if sender_address != wallet.address {
                log::error!(
                    "Derived address {} doesn't match wallet address {}",
                    sender_address,
                    wallet.address
                );
                return Err(AppError::InternalServerError(
                    "Wallet address mismatch - security check failed".to_string(),
                ));
            }

            log::info!(
                "Address verification passed: {:?}\n decrypted_private_key: {:?}",
                sender_address,
                decrypted_private_key
            );

            let mainnet_rpc = "https://api.mainnet-beta.solana.com";
            let balance_result = get_sol_balance(&wallet.address, Some(mainnet_rpc)).await;

            let current_balance_lamports = match balance_result {
                Ok(balance) => balance,
                Err(e) => {
                    log::error!("Failed to check wallet balance: {}", e);
                    return Err(AppError::InternalServerError(format!(
                        "Could not verify wallet balance: {}",
                        e
                    )));
                }
            };

            println!("current_balance_lamports {:?}", current_balance_lamports);

            let current_balance_sol = current_balance_lamports as f64 / 1_000_000_000.0;
            println!(
                "💰 Current wallet balance: {} SOL ({} lamports)",
                current_balance_sol, current_balance_lamports
            );

            let estimated_fee = match estimate_transfer_fee(
                &sender_keypair.pubkey(),
                &request_body.to_address,
                amount_lamports,
                Some(mainnet_rpc),
            )
            .await
            {
                Ok(fee) => fee,
                Err(e) => {
                    log::error!("Failed to estimate fee: {}", e);
                    5000_u64
                }
            };

            let fee_sol = estimated_fee as f64 / 1_000_000_000.0;
            let total_cost_lamports = amount_lamports + estimated_fee;
            let total_cost_sol = total_cost_lamports as f64 / 1_000_000_000.0;

            println!("Transaction cost analysis:");
            println!(
                "Transfer: {} SOL ({} lamports)",
                amount_sol, amount_lamports
            );
            println!("   Fee: {} SOL ({} lamports)", fee_sol, estimated_fee);
            println!(
                "   Total: {} SOL ({} lamports)",
                total_cost_sol, total_cost_lamports
            );
            println!("   Remaining: {} SOL", current_balance_sol - total_cost_sol);

            let rent_minimum_lamports = get_rent_exemption_amount(Some(mainnet_rpc))
                .await
                .unwrap_or(890880);
            let rent_minimum_sol = rent_minimum_lamports as f64 / 1_000_000_000.0;

            println!(
                "🔒 Rent exemption minimum: {} SOL ({} lamports)",
                rent_minimum_sol, rent_minimum_lamports
            );

            let transferable_lamports = current_balance_lamports
                .saturating_sub(estimated_fee)
                .saturating_sub(rent_minimum_lamports);
            let transferable_sol = transferable_lamports as f64 / 1_000_000_000.0;

            println!(
                "💸 Transferable amount: {} SOL ({} lamports)",
                transferable_sol, transferable_lamports
            );
            println!(
                " Breakdown: {} SOL balance - {} SOL fee - {} SOL rent = {} SOL transferable",
                current_balance_sol, fee_sol, rent_minimum_sol, transferable_sol
            );

            if transferable_lamports == 0 || transferable_sol <= 0.0 {
                log::warn!("Wallet {} has insufficient balance (0 SOL transferable) - checking next wallet...", wallet.address);
                log::info!("Wallet balance ({} SOL) is too small after fees and rent, moving to next wallet", current_balance_sol);
                continue;
            }

            let final_amount_lamports;
            let final_amount_sol;

            if amount_lamports > transferable_lamports {
                final_amount_lamports = transferable_lamports;
                final_amount_sol = transferable_sol;

                log::info!(
                    "Auto-adjusting withdrawal: Requested {} SOL exceeds transferable {} SOL",
                    amount_sol,
                    transferable_sol
                );
                log::info!(
                    "Using maximum transferable amount: {} SOL",
                    final_amount_sol
                );
            } else {
                final_amount_lamports = amount_lamports;
                final_amount_sol = amount_sol;
            }

            println!(
                "Transfer approved: {} SOL from {} SOL transferable (using {} SOL)",
                final_amount_sol, transferable_sol, final_amount_sol
            );

            let withdrawal_id = uuid::Uuid::new_v4().to_string();

            log::info!("Using merchant_id {} for withdrawal record", merchant.id);

            let withdrawal_record = withdrawal_request::ActiveModel {
                id: sea_orm::Set(withdrawal_id.clone()),
                merchant_id: sea_orm::Set(merchant.id.clone()),
                wallet_id: sea_orm::Set(wallet.id.clone()),
                external_id: sea_orm::Set(withdrawal_id.clone()),
                idempotency_key: sea_orm::Set(request_body.idempotency_key.clone()),
                environment: sea_orm::Set(request_body.environment.clone()),
                network: sea_orm::Set(request_body.network.clone()),
                currency: sea_orm::Set("solana".to_string()),
                to_address: sea_orm::Set(request_body.to_address.clone()),
                amount: sea_orm::Set(Decimal::from_f64(final_amount_sol).unwrap_or_default()),
                fee: sea_orm::Set(Some(Decimal::from_f64(fee_sol).unwrap_or_default())),
                net_amount: sea_orm::Set(Decimal::from_f64(final_amount_sol).unwrap_or_default()),
                status: sea_orm::Set("processing".to_string()),
                required_confirmations: sea_orm::Set(32i32),
                created_at: sea_orm::Set(chrono::Utc::now().into()),
                updated_at: sea_orm::Set(chrono::Utc::now().into()),
                finality_required: sea_orm::Set(32),
                finality_reached_at: sea_orm::Set(None),
                replaced_by_tx: sea_orm::Set(None),
                reorg_depth: sea_orm::Set(0),
                lifecycle_state: sea_orm::Set(WithdrawalLifecycleState::Quoted.to_string()),
                withdrawal_funds_state: sea_orm::Set(WithdrawalFundsState::Unlocked.to_string()),
                ..Default::default()
            };

            let saved_withdrawal = withdrawal_record.insert(&app_state.db).await.map_err(|e| {
                log::error!("Failed to create withdrawal record: {}", e);
                AppError::DatabaseError(format!("Failed to create withdrawal record: {}", e))
            })?;

            log::info!(
                "Withdrawal {} created in database - executing Solana transfer on MAINNET...",
                withdrawal_id
            );

            let transfer_result = transfer_sol_with_confirmation(
                &sender_keypair,
                &request_body.to_address,
                final_amount_lamports,
                Some(mainnet_rpc),
            )
            .await;

            match transfer_result {
                Ok(tx_signature) => {
                    log::info!("Solana transfer successful!");
                    println!("Transaction signature: {}", tx_signature);

                    let mut withdrawal_update: withdrawal_request::ActiveModel =
                        saved_withdrawal.into();
                    withdrawal_update.status = sea_orm::Set("confirmed".to_string());
                    withdrawal_update.tx_hash = sea_orm::Set(Some(tx_signature.clone()));
                    withdrawal_update.blockchain_confirmations = sea_orm::Set(Some(1));
                    withdrawal_update.confirmed_at = sea_orm::Set(Some(chrono::Utc::now().into()));
                    withdrawal_update.processed_at = sea_orm::Set(Some(chrono::Utc::now().into()));
                    withdrawal_update.broadcast_at = sea_orm::Set(Some(chrono::Utc::now().into()));
                    withdrawal_update.updated_at = sea_orm::Set(chrono::Utc::now().into());

                    println!("withdrawal_update {:?}\n ", withdrawal_update);

                    let _final_withdrawal =
                        withdrawal_update.update(&app_state.db).await.map_err(|e| {
                            log::error!("Failed to update withdrawal record: {}", e);
                            AppError::DatabaseError(format!("Failed to update withdrawal: {}", e))
                        })?;

                    log::info!("Syncing balances after successful withdrawal");
                    if let Err(sync_error) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(&app_state, &merchant.id, &request_body.environment).await {
                        log::warn!("Balance sync failed after withdrawal: {:?}", sync_error);
                    }

                    let new_balance_lamports = get_sol_balance(&wallet.address, Some(mainnet_rpc))
                        .await
                        .unwrap_or(current_balance_lamports);
                    let new_balance = new_balance_lamports as f64 / 1_000_000_000.0;

                    log::info!("New wallet balance: {} SOL", new_balance);

                    let response = serde_json::json!({
                        "id": withdrawal_id,
                        "merchant_id": merchant.id,
                        "external_id": withdrawal_id.clone(),
                        "idempotency_key": request_body.idempotency_key,
                        "environment": request_body.environment,
                        "network": request_body.network,
                        "currency": "solana",
                        "to_address": request_body.to_address,
                        "amount": final_amount_sol.to_string(),
                        "fee": fee_sol.to_string(),
                        "net_amount": final_amount_sol.to_string(),
                        "status": "confirmed",
                        "tx_hash": tx_signature,
                        "blockchain_confirmations": 1,
                        "required_confirmations": 32,
                        "confirmations": 1,
                        "explorer_url": format!("https://explorer.solana.com/tx/{}?cluster=mainnet", tx_signature),
                        "created_at": chrono::Utc::now().to_rfc3339(),
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                        "processed_at": chrono::Utc::now().to_rfc3339(),
                        "confirmed_at": chrono::Utc::now().to_rfc3339(),
                        "broadcast_at": chrono::Utc::now().to_rfc3339(),
                        "wallet_used": {
                            "address": wallet.address,
                            "previous_balance": format!("{} SOL", current_balance_sol),
                            "new_balance": format!("{} SOL", new_balance)
                        }
                    });

                    successful_withdrawal = true;
                    return Ok(HttpResponse::Ok().json(response));
                }
                Err(e) => {
                    log::error!("Solana transfer failed: {}", e);

                    let mut withdrawal_update: withdrawal_request::ActiveModel =
                        saved_withdrawal.into();
                    withdrawal_update.status = sea_orm::Set("failed".to_string());
                    withdrawal_update.failure_reason = sea_orm::Set(Some(e.to_string()));
                    withdrawal_update.failed_at = sea_orm::Set(Some(chrono::Utc::now().into()));
                    withdrawal_update.updated_at = sea_orm::Set(chrono::Utc::now().into());

                    let _failed_withdrawal =
                        withdrawal_update.update(&app_state.db).await.map_err(|e| {
                            log::error!("Failed to update failed withdrawal record: {}", e);
                            AppError::DatabaseError(format!(
                                "Failed to update failed withdrawal: {}",
                                e
                            ))
                        })?;

                    log::warn!(
                        "Transfer failed from wallet {}, trying next wallet if available...",
                        wallet.address
                    );
                    continue;
                }
            }
        }

        wallet_details.push(serde_json::json!({
            "wallet_id": wallet.id,
            "address": wallet.address,
            "public_key": wallet.public_key,
            "currency": wallet.currency,
            "has_mnemonic": decrypted_mnemonic.is_some(),
            "has_private_key": decrypted_private_key.is_some(),
            "created_at": wallet.created_at
        }));
    }

    if successful_withdrawal {
        log::error!("Unexpected state: successful_withdrawal flag set but didn't return");
    }

    log::warn!(
        "No single wallet had sufficient balance for withdrawal of {} {}",
        request_body.amount,
        request_body.network.to_uppercase()
    );
    log::info!("🔄 Attempting multi-wallet withdrawal as fallback to aggregate balances from all {} wallets", wallets.len());

    if request_body.network == "sol" && request_body.environment == "mainnet" {
        log::info!("💎 Using Solana-specific multi-wallet aggregation for SOL mainnet withdrawal");
        return MerchantAuth::try_solana_multi_wallet_withdrawal_fallback(
            &app_state,
            &merchant.id,
            &request_body,
        )
        .await;
    }

    if (request_body.network == "bnb"
        || request_body.network == "bsc"
        || request_body.network == "eth"
        || request_body.network == "ethereum")
        && request_body.environment == "mainnet"
    {
        log::info!(
            "⚡ Using EVM-specific multi-wallet aggregation for {} mainnet withdrawal",
            request_body.network.to_uppercase()
        );
        return MerchantAuth::try_evm_multi_wallet_withdrawal_fallback(
            &app_state,
            &merchant.id,
            &request_body,
        )
        .await;
    }

    if request_body.network == "btc" && request_body.environment == "mainnet" {
        log::info!("₿ Using Bitcoin-specific multi-wallet aggregation for BTC mainnet withdrawal");
        return MerchantAuth::try_btc_multi_wallet_withdrawal_fallback(
            &app_state,
            &merchant.id,
            &request_body,
        )
        .await;
    }

    return MerchantAuth::try_multi_wallet_withdrawal_fallback(
        &app_state,
        &merchant.id,
        &request_body,
    )
    .await;
}

pub async fn list_withdrawals(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<WithdrawalListQuery>,
) -> Result<HttpResponse, AppError> {
    log::info!("list_withdrawals called with query: {:?}", query);

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!(
        "User claims found: user_id={}, merchant_id={:?}",
        claims.id,
        claims.merchant_id
    );

    let merchant_id = match &claims.merchant_id {
        Some(mid) => mid.clone(),
        None => {
            log::warn!(
                "No merchant_id in JWT claims, falling back to user lookup for user {}",
                claims.id
            );

            let merchants = Merchant::find()
                .filter(merchant::Column::UserId.eq(&claims.id))
                .all(&app_state.db)
                .await
                .map_err(|e| {
                    log::error!(
                        "Database error finding merchants for user {}: {:?}",
                        claims.id,
                        e
                    );
                    AppError::DatabaseError(format!("Failed to find merchants: {}", e))
                })?;

            if merchants.is_empty() {
                log::warn!("No merchants found for user {}", claims.id);

                let empty_response = serde_json::json!({
                    "withdrawals": [],
                    "pagination": {
                        "page": 1,
                        "limit": 50,
                        "total": 0,
                        "total_pages": 0,
                        "has_next": false,
                        "has_prev": false
                    },
                    "summary": {
                        "total_count": 0,
                        "pending_count": 0,
                        "processing_count": 0,
                        "confirmed_count": 0,
                        "failed_count": 0,
                        "total_amount_usd": "0.00",
                        "total_fees_usd": "0.00"
                    }
                });

                return Ok(HttpResponse::Ok()
            .insert_header(("x-merchant-setup-required", "true"))
            .insert_header(("x-merchant-message", "No merchant account found. Please complete your merchant setup to start making withdrawals."))
            .json(empty_response));
            }

            let merchant = &merchants[0];
            log::info!(
                "Using first merchant {} for user {}",
                merchant.id,
                claims.id
            );
            merchant.id.clone()
        }
    };

    log::info!("Using merchant {} for user {}", merchant_id, claims.id);

    let withdrawals =
        WithdrawalService::list_withdrawals(&app_state, &merchant_id, query.into_inner())
            .await
            .map_err(|e| {
                log::error!("WithdrawalService::list_withdrawals failed: {:?}", e);
                e
            })?;

    log::info!(
        "Successfully retrieved {} withdrawals",
        withdrawals.withdrawals.len()
    );
    Ok(HttpResponse::Ok().json(withdrawals))
}

/// GET /api/v1/merchant/fee-estimate?network=&amount=&environment=
pub async fn estimate_fee(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<FeeEstimateRequest>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

    query.validate()?;

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if merchants.is_empty() {
        return Err(AppError::BadRequest("No merchant account found. Please complete your merchant setup to estimate withdrawal fees.".to_string()));
    }

    let merchant = &merchants[0];

    let fee_estimate =
        WithdrawalService::estimate_fee(&app_state, &merchant.id, &query.into_inner()).await?;

    Ok(HttpResponse::Ok().json(fee_estimate))
}

/// GET /api/v1/merchant/max-withdrawable?network=&environment=
pub async fn get_max_withdrawable(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<MaxWithdrawableRequest>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if merchants.is_empty() {
        return Err(AppError::BadRequest(
            "No merchant account found. Please complete your merchant setup.".to_string(),
        ));
    }

    let merchant = &merchants[0];

    let max_withdrawable = WithdrawalService::get_max_withdrawable_amount(
        &app_state,
        &merchant.id,
        &query.into_inner(),
    )
    .await?;

    Ok(HttpResponse::Ok().json(max_withdrawable))
}

pub async fn get_withdrawal(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;
    let withdrawal_id = path.into_inner();

    if withdrawal_id == "undefined" || withdrawal_id.is_empty() {
        log::warn!(" Invalid withdrawal ID received: '{}'", withdrawal_id);
        return Err(AppError::ValidationError(
            "Invalid withdrawal ID. Please provide a valid withdrawal ID instead of 'undefined'."
                .to_string(),
        ));
    }

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if merchants.is_empty() {
        return Err(AppError::BadRequest("No merchant account found. Please complete your merchant setup to access withdrawal details.".to_string()));
    }

    let merchant_ids: Vec<String> = merchants.into_iter().map(|m| m.id).collect();

    let withdrawal = withdrawal_request::Entity::find()
        .filter(withdrawal_request::Column::Id.eq(&withdrawal_id))
        .filter(withdrawal_request::Column::MerchantId.is_in(merchant_ids))
        .one(&app_state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

    let response =
        crate::merchant::services::WithdrawalService::withdrawal_to_response(&withdrawal);
    Ok(HttpResponse::Ok().json(response))
}

/// POST /api/v1/merchant/withdrawals/{id}/cancel
pub async fn cancel_withdrawal(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;
    let withdrawal_id = path.into_inner();

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if merchants.is_empty() {
        return Err(AppError::BadRequest(
            "No merchant account found. Please complete your merchant setup to cancel withdrawals."
                .to_string(),
        ));
    }

    let merchant_ids: Vec<String> = merchants.into_iter().map(|m| m.id).collect();

    let withdrawal = withdrawal_request::Entity::find()
        .filter(withdrawal_request::Column::Id.eq(&withdrawal_id))
        .filter(withdrawal_request::Column::MerchantId.is_in(merchant_ids))
        .one(&app_state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Withdrawal not found".to_string()))?;

    if !withdrawal.can_be_cancelled() {
        return Err(AppError::ValidationError(
            "Withdrawal cannot be cancelled".to_string(),
        ));
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Withdrawal cancellation requested",
        "withdrawal_id": withdrawal_id
    })))
}

pub async fn debug_wallets(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

    let all_wallets = wallet::Entity::find()
        .filter(wallet::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to find wallets: {}", e)))?;

    let mainnet_sol_wallets = wallet::Entity::find()
        .filter(wallet::Column::UserId.eq(&claims.id))
        .filter(wallet::Column::Currency.contains("sol_mainnet"))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            AppError::DatabaseError(format!("Failed to find sol mainnet wallets: {}", e))
        })?;

    let response = serde_json::json!({
        "user_id": claims.id,
        "total_wallets": all_wallets.len(),
        "mainnet_sol_wallets": mainnet_sol_wallets.len(),
        "all_wallets": all_wallets.iter().map(|w| {
            serde_json::json!({
                "address": w.address,
                "currency": w.currency,
                "created_at": w.created_at
            })
        }).collect::<Vec<_>>(),
        "mainnet_sol_details": mainnet_sol_wallets.iter().map(|w| {
            serde_json::json!({
                "address": w.address,
                "currency": w.currency,
                "created_at": w.created_at
            })
        }).collect::<Vec<_>>(),
        "looking_for_wallet": "ijAiubWuiGYz8RLP8AWtscMfmG3mb2yBTB1hK6sYxjr"
    });

    Ok(HttpResponse::Ok().json(response))
}

async fn handle_usdt_withdrawal(
    app_state: web::Data<AppState>,
    claims: UserClaims,
    request_body: web::Json<CreateWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    log::info!(
        "🪙 Handling USDT token withdrawal: {} {}",
        request_body.amount,
        request_body.network
    );

    let merchant_id = claims
        .merchant_id
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("Merchant ID required for withdrawals".to_string()))?;

    if let Some(existing) =
        check_existing_withdrawal(&app_state.db, merchant_id, &request_body.idempotency_key).await?
    {
        log::info!(
            "Returning existing withdrawal for idempotency key: {}",
            request_body.idempotency_key
        );
        return Ok(HttpResponse::Ok().json(existing));
    }

    let (final_amount, amount_type) = if request_body.is_usd_withdrawal() {
        log::info!(
            "💱 Converting USD amount {} to crypto for {}",
            request_body.amount,
            request_body.network
        );

        let crypto_amount = UsdConversionService::convert_usd_to_crypto(
            &request_body.amount,
            &request_body.network,
            &request_body.environment,
        )
        .await?;

        log::info!(
            "💱 USD conversion: ${} → {} {}",
            request_body.amount,
            crypto_amount,
            request_body.network.to_uppercase()
        );
        (crypto_amount, "usd")
    } else {
        (request_body.amount.clone(), "crypto")
    };

    match UsdtWithdrawalService::execute_withdrawal(
        &app_state.db,
        merchant_id,
        &request_body.network,
        &request_body.environment,
        &request_body.to_address,
        &final_amount,
        "crypto",
        &request_body.idempotency_key,
    )
    .await
    {
        Ok(result) => {
            log::info!(
                "   USDT withdrawal successful: {} tokens from {} wallets",
                result.total_token_sent,
                result.wallets_used.len()
            );

            let mut response = convert_usdt_result_to_response(&result);

            if amount_type == "usd" {
                response["amount_type"] = serde_json::Value::String("usd".to_string());
                response["requested_usd_amount"] =
                    serde_json::Value::String(request_body.amount.clone());
                response["usd_equivalent"] = serde_json::Value::String(request_body.amount.clone());
            } else {
                response["amount_type"] = serde_json::Value::String("crypto".to_string());
            }

            Ok(HttpResponse::Ok().json(response))
        }
        Err(AppError::ValidationError(msg)) if msg.starts_with("USDT_") => {
            log::warn!("USDT withdrawal insufficient balance: {}", msg);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "insufficient_balance",
                "message": msg,
                "code": if msg.contains("ERC20") { "USDT_ERC20_INSUFFICIENT" } else { "USDT_BEP20_INSUFFICIENT" }
            })))
        }
        Err(AppError::InternalServerError(msg)) if msg.contains("CRYPTO_ENCRYPTION_KEY") => {
            log::error!("Encryption key missing: {}", msg);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "crypto_key_missing",
                "message": "CRYPTO_ENCRYPTION_KEY environment variable is not set",
                "code": "CRYPTO_KEY_MISSING"
            })))
        }
        Err(e) => {
            log::error!(" USDT withdrawal failed: {:?}", e);
            Err(e)
        }
    }
}

async fn check_existing_withdrawal(
    db: &sea_orm::DatabaseConnection,
    merchant_id: &str,
    idempotency_key: &str,
) -> Result<Option<serde_json::Value>, AppError> {
    use crate::shared::entities::withdrawal_request;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    match withdrawal_request::Entity::find()
        .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
        .filter(withdrawal_request::Column::IdempotencyKey.eq(idempotency_key))
        .one(db)
        .await
    {
        Ok(Some(existing)) => {
            let response = serde_json::json!({
                "id": existing.id,
                "status": existing.status,
                "amount": existing.amount.to_string(),
                "currency": existing.currency,
                "network": existing.network,
                "to_address": existing.to_address,
                "created_at": existing.created_at,
                "tx_hash": existing.tx_hash,
                "idempotency_key": existing.idempotency_key
            });
            Ok(Some(response))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(AppError::DatabaseError(format!(
            "Failed to check existing withdrawal: {}",
            e
        ))),
    }
}

fn convert_usdt_result_to_response(
    result: &crate::shared::service::usdt_withdrawal_service::UsdtWithdrawalResult,
) -> serde_json::Map<String, serde_json::Value> {
    use crate::shared::service::token_config_service::TokenConfigService;

    let token_config = TokenConfigService::get_token_config(&result.currency, &result.environment)
        .unwrap_or_else(
            |_| crate::shared::service::token_config_service::TokenConfig {
                network: result.network.clone(),
                contract_address: "".to_string(),
                decimals: 18,
                chain_id: 1,
                rpc_url: "".to_string(),
                explorer_base_url: "".to_string(),
                gas_limit: 60000,
                required_confirmations: 12,
            },
        );

    let amount_display = TokenConfigService::base_units_to_amount(
        &result.total_token_sent.to_string(),
        token_config.decimals,
    )
    .unwrap_or("0".to_string());

    let fee_display =
        TokenConfigService::base_units_to_amount(&result.total_gas_used.to_string(), 18)
            .unwrap_or("0".to_string());

    let mut response = serde_json::Map::new();
    response.insert(
        "id".to_string(),
        serde_json::Value::String(result.withdrawal_id.clone()),
    );
    response.insert(
        "status".to_string(),
        serde_json::Value::String(result.status.clone()),
    );
    response.insert(
        "amount".to_string(),
        serde_json::Value::String(amount_display),
    );
    response.insert(
        "currency".to_string(),
        serde_json::Value::String(result.currency.clone()),
    );
    response.insert(
        "network".to_string(),
        serde_json::Value::String(result.network.clone()),
    );
    response.insert(
        "environment".to_string(),
        serde_json::Value::String(result.environment.clone()),
    );
    response.insert(
        "tx_hashes".to_string(),
        serde_json::Value::Array(
            result
                .tx_hashes
                .iter()
                .map(|h| serde_json::Value::String(h.clone()))
                .collect(),
        ),
    );
    response.insert(
        "explorer_urls".to_string(),
        serde_json::Value::Array(
            result
                .explorer_urls
                .iter()
                .map(|u| serde_json::Value::String(u.clone()))
                .collect(),
        ),
    );
    response.insert(
        "wallets_used".to_string(),
        serde_json::to_value(&result.wallets_used).unwrap_or_default(),
    );
    response.insert(
        "total_fee".to_string(),
        serde_json::Value::String(fee_display),
    );
    response.insert(
        "created_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );
    response.insert("multi_wallet".to_string(), serde_json::Value::Bool(true));
    response
}

/// TEST: GET /api/v1/merchant/test/mainnet-balances - Test mainnet balance sync without auth
pub async fn test_mainnet_balance_sync(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    log::info!("Testing mainnet balance sync - finding first merchant");

    let first_merchant = Merchant::find().one(&app_state.db).await?;

    match first_merchant {
        Some(merchant) => {
            log::info!(
                "Testing mainnet balance sync for merchant {} (user_id: {})",
                merchant.id,
                merchant.user_id
            );

            match crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(&app_state, &merchant.id, "mainnet").await {
                Ok(_) => {
                    log::info!("Mainnet balance sync completed successfully");

                    match crate::merchant::services::WithdrawalService::get_balances(&app_state, &merchant.id, "mainnet").await {
                        Ok(balances) => {
                            Ok(HttpResponse::Ok().json(serde_json::json!({
                                "message": "Mainnet balance sync test completed successfully",
                                "merchant_id": merchant.id,
                                "user_id": merchant.user_id,
                                "status": "success",
                                "balances": balances
                            })))
                        },
                        Err(e) => {
                            log::error!("Get mainnet balances failed: {:?}", e);
                            Ok(HttpResponse::Ok().json(serde_json::json!({
                                "message": "Mainnet balance sync succeeded but get_balances failed",
                                "merchant_id": merchant.id,
                                "status": "partial_success",
                                "error": format!("{:?}", e)
                            })))
                        }
                    }
                },
                Err(e) => {
                    log::error!("Mainnet balance sync failed: {:?}", e);
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "message": "Mainnet balance sync test failed",
                        "merchant_id": merchant.id,
                        "status": "error",
                        "error": format!("{:?}", e)
                    })))
                }
            }
        }
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "message": "No merchants found in database",
            "status": "no_merchants"
        }))),
    }
}

/// TEST: GET /api/v1/merchant/test/balance-sync - Test balance sync without auth
pub async fn test_balance_sync(app_state: web::Data<AppState>) -> Result<HttpResponse, AppError> {
    log::info!("Testing balance sync - finding first merchant");

    let first_merchant = Merchant::find().one(&app_state.db).await?;

    match first_merchant {
        Some(merchant) => {
            log::info!(
                "Testing balance sync for merchant {} (user_id: {})",
                merchant.id,
                merchant.user_id
            );

            match crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments(&app_state, &merchant.id).await {
                Ok(_) => {
                    log::info!("Balance sync completed successfully");

                    match crate::merchant::services::WithdrawalService::get_balances(&app_state, &merchant.id, "testnet").await {
                        Ok(balances) => {
                            Ok(HttpResponse::Ok().json(serde_json::json!({
                                "message": "Balance sync test completed successfully",
                                "merchant_id": merchant.id,
                                "user_id": merchant.user_id,
                                "status": "success",
                                "balances": balances
                            })))
                        },
                        Err(e) => {
                            log::error!("Get balances failed: {:?}", e);
                            Ok(HttpResponse::Ok().json(serde_json::json!({
                                "message": "Balance sync succeeded but get_balances failed",
                                "merchant_id": merchant.id,
                                "status": "partial_success",
                                "error": format!("{:?}", e)
                            })))
                        }
                    }
                },
                Err(e) => {
                    log::error!("Balance sync failed: {:?}", e);
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "message": "Balance sync test failed",
                        "merchant_id": merchant.id,
                        "status": "error",
                        "error": format!("{:?}", e)
                    })))
                }
            }
        }
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "message": "No merchants found in database",
            "status": "no_merchants"
        }))),
    }
}

/// TEST: GET /api/v1/merchant/test/withdrawal-list - Test withdrawal list without auth
pub async fn test_withdrawal_list(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    log::info!("Testing withdrawal list - finding first merchant");

    let first_merchant = Merchant::find().one(&app_state.db).await?;

    match first_merchant {
        Some(merchant) => {
            log::info!(
                "Testing withdrawal list for merchant {} (user_id: {})",
                merchant.id,
                merchant.user_id
            );

            let query = WithdrawalListQuery {
                status: None,
                network: None,
                environment: Some("testnet".to_string()),
                from_date: None,
                to_date: None,
                page: Some(1),
                limit: Some(50),
            };

            match crate::merchant::services::WithdrawalService::list_withdrawals(
                &app_state,
                &merchant.id,
                query,
            )
            .await
            {
                Ok(result) => Ok(HttpResponse::Ok().json(serde_json::json!({
                    "message": "Withdrawal list test completed successfully",
                    "merchant_id": merchant.id,
                    "user_id": merchant.user_id,
                    "status": "success",
                    "result": result
                }))),
                Err(e) => {
                    log::error!("Withdrawal list test failed: {:?}", e);
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "message": "Withdrawal list test failed",
                        "merchant_id": merchant.id,
                        "status": "error",
                        "error": format!("{:?}", e)
                    })))
                }
            }
        }
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "message": "No merchants found in database",
            "status": "no_merchants"
        }))),
    }
}

/// TEST: GET /api/v1/merchant/test/payments - Check payment data by environment without auth
pub async fn test_payments_by_environment(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    log::info!("Testing payment data by environment - finding first merchant");

    let first_merchant = Merchant::find().one(&app_state.db).await?;

    match first_merchant {
        Some(merchant) => {
            log::info!(
                "Checking payment data for merchant {} (user_id: {})",
                merchant.id,
                merchant.user_id
            );

            let all_payments = crate::shared::entities::payment_request::Entity::find()
                .filter(
                    crate::shared::entities::payment_request::Column::MerchantId.eq(&merchant.id),
                )
                .all(&app_state.db)
                .await?;

            let mainnet_payments = crate::shared::entities::payment_request::Entity::find()
                .filter(
                    crate::shared::entities::payment_request::Column::MerchantId.eq(&merchant.id),
                )
                .filter(crate::shared::entities::payment_request::Column::Environment.eq("mainnet"))
                .all(&app_state.db)
                .await?;

            let testnet_payments = crate::shared::entities::payment_request::Entity::find()
                .filter(
                    crate::shared::entities::payment_request::Column::MerchantId.eq(&merchant.id),
                )
                .filter(crate::shared::entities::payment_request::Column::Environment.eq("testnet"))
                .all(&app_state.db)
                .await?;

            let paid_mainnet: Vec<_> = mainnet_payments
                .iter()
                .filter(|p| p.status == "paid")
                .collect();

            let paid_testnet: Vec<_> = testnet_payments
                .iter()
                .filter(|p| p.status == "paid")
                .collect();

            Ok(HttpResponse::Ok().json(serde_json::json!({
                "merchant_id": merchant.id,
                "user_id": merchant.user_id,
                "total_payments": all_payments.len(),
                "mainnet_payments": mainnet_payments.len(),
                "testnet_payments": testnet_payments.len(),
                "paid_mainnet_payments": paid_mainnet.len(),
                "paid_testnet_payments": paid_testnet.len(),
                "mainnet_samples": paid_mainnet.iter().take(3).map(|p| serde_json::json!({
                    "id": p.id,
                    "currency": p.currency,
                    "amount": p.amount.to_string(),
                    "status": p.status,
                    "environment": p.environment,
                    "created_at": p.created_at
                })).collect::<Vec<_>>(),
                "testnet_samples": paid_testnet.iter().take(3).map(|p| serde_json::json!({
                    "id": p.id,
                    "currency": p.currency,
                    "amount": p.amount.to_string(),
                    "status": p.status,
                    "environment": p.environment,
                    "created_at": p.created_at
                })).collect::<Vec<_>>()
            })))
        }
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "message": "No merchants found in database",
            "status": "no_merchants"
        }))),
    }
}

/// DEBUG: GET /api/v1/merchant/debug/payments - Check payment data
pub async fn debug_payments(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if merchants.is_empty() {
        return Err(AppError::NotFound(
            "No merchants found for user".to_string(),
        ));
    }

    let merchant = &merchants[0];

    let payments = crate::shared::entities::payment_request::Entity::find()
        .filter(crate::shared::entities::payment_request::Column::MerchantId.eq(&merchant.id))
        .all(&app_state.db)
        .await?;

    let paid_payments: Vec<_> = payments.iter().filter(|p| p.status == "paid").collect();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "merchant_id": merchant.id,
        "user_id": merchant.user_id,
        "total_payments": payments.len(),
        "paid_payments": paid_payments.len(),
        "payments": payments.iter().take(5).map(|p| serde_json::json!({
            "id": p.id,
            "currency": p.currency,
            "amount": p.amount.to_string(),
            "status": p.status,
            "created_at": p.created_at
        })).collect::<Vec<_>>()
    })))
}

/// DEBUG: POST /api/v1/merchant/debug/process-withdrawal/{withdrawal_id} - Manually trigger withdrawal processing
pub async fn debug_process_withdrawal(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let withdrawal_id = path.into_inner();

    log::info!("DEBUG: Manually processing withdrawal {}", withdrawal_id);

    let withdrawal = withdrawal_request::Entity::find()
        .filter(withdrawal_request::Column::Id.eq(&withdrawal_id))
        .one(&app_state.db)
        .await?;

    match withdrawal {
        Some(w) => {
            log::info!(
                "DEBUG: Found withdrawal {} with status {}",
                withdrawal_id,
                w.status
            );

            match WithdrawalProcessor::process_withdrawal(&app_state, &withdrawal_id).await {
                Ok(tx_hash) => {
                    log::info!(
                        "DEBUG: Successfully processed withdrawal {} - tx_hash: {}",
                        withdrawal_id,
                        tx_hash
                    );
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "message": "Withdrawal processed successfully",
                        "withdrawal_id": withdrawal_id,
                        "tx_hash": tx_hash,
                        "status": "success"
                    })))
                }
                Err(e) => {
                    log::error!(
                        "DEBUG: Failed to process withdrawal {}: {:?}",
                        withdrawal_id,
                        e
                    );
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "message": "Withdrawal processing failed",
                        "withdrawal_id": withdrawal_id,
                        "error": format!("{:?}", e),
                        "status": "error"
                    })))
                }
            }
        }
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "message": "Withdrawal not found",
            "withdrawal_id": withdrawal_id,
            "status": "not_found"
        }))),
    }
}

pub async fn get_multi_wallet_balances(
    app_state: web::Data<AppState>,
    merchant: MerchantAuth,
    query: web::Query<BalanceQuery>,
) -> Result<HttpResponse, AppError> {
    let environment = query.environment.as_deref().unwrap_or("mainnet");

    log::info!(
        " Getting multi-wallet balances for merchant {} in {}",
        merchant.merchant_id,
        environment
    );

    match MultiWalletService::get_aggregated_balances(
        &app_state.db,
        &merchant.merchant_id,
        environment,
    )
    .await
    {
        Ok(balances) => {
            let total_usd: rust_decimal::Decimal = balances
                .iter()
                .filter_map(|b| b.total_usd_value.parse::<rust_decimal::Decimal>().ok())
                .sum();

            let total_wallets: u64 = balances.iter().map(|b| b.wallet_count).sum();

            let response = serde_json::json!({
                "environment": environment,
                "aggregated_balances": balances,
                "total_usd_value": total_usd.to_string(),
                "total_wallets": total_wallets,
                "last_updated": chrono::Utc::now()
            });

            Ok(HttpResponse::Ok().json(response))
        }
        Err(e) => {
            log::error!("Failed to get multi-wallet balances: {:?}", e);
            Err(e)
        }
    }
}

pub async fn preview_withdrawal_plan(
    app_state: web::Data<AppState>,
    merchant: MerchantAuth,
    withdrawal_data: web::Json<CreateWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    let request = withdrawal_data.into_inner();

    if let Err(validation_errors) = request.validate() {
        return Err(AppError::ValidationError(format!(
            "{:?}",
            validation_errors
        )));
    }

    log::info!(
        "🔮 Previewing withdrawal plan for merchant {}: {} {} to {}",
        merchant.merchant_id,
        request.amount,
        request.network,
        request.to_address
    );

    let target_amount = if request.is_max_withdrawal() {
        rust_decimal::Decimal::MAX
    } else {
        match request.validate_amount_for_network() {
            Ok(amount) => amount,
            Err(e) => {
                return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Invalid amount",
                    "details": e
                })));
            }
        }
    };

    let currency = request.get_currency();
    let strategy = request
        .wallet_selection_strategy
        .as_deref()
        .unwrap_or("optimal");

    match MultiWalletService::select_wallets_for_withdrawal(
        &app_state.db,
        &merchant.merchant_id,
        &request.environment,
        currency,
        target_amount,
        strategy,
    )
    .await
    {
        Ok(plan) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "preview": true,
            "plan": plan,
            "network": request.network,
            "to_address": request.to_address,
            "strategy_used": strategy
        }))),
        Err(e) => {
            log::error!("Failed to create withdrawal plan: {:?}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create withdrawal plan",
                "details": format!("{:?}", e)
            })))
        }
    }
}

pub async fn preview_evm_withdrawal(
    app_state: web::Data<AppState>,
    merchant: MerchantAuth,
    withdrawal_data: web::Json<CreateWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    let request = withdrawal_data.into_inner();

    if let Err(validation_errors) = request.validate() {
        return Err(AppError::ValidationError(format!(
            "{:?}",
            validation_errors
        )));
    }

    if !matches!(
        request.network.to_lowercase().as_str(),
        "bnb" | "bsc" | "eth" | "ethereum"
    ) {
        return Err(AppError::ValidationError(format!(
            "Network {} not supported for EVM preview",
            request.network
        )));
    }

    log::info!(
        "🔮 Previewing EVM withdrawal for merchant {}: {} {} to {}",
        merchant.merchant_id,
        request.amount,
        request.network,
        request.to_address
    );

    match EvmMultiWalletService::calculate_net_withdrawable_amount(
        &app_state.db,
        &merchant.merchant_id,
        &request.network,
        &request.environment,
        &request.amount,
    )
    .await
    {
        Ok(preview) => {
            log::info!("   EVM withdrawal preview generated");

            let response = serde_json::json!({
                "merchant_id": merchant.merchant_id,
                "network": request.network.to_uppercase(),
                "environment": request.environment,
                "to_address": request.to_address,
                "gross_amount": preview.gross_amount,
                "estimated_gas_fee": preview.estimated_gas_fee,
                "net_amount": preview.net_amount,
                "can_withdraw": preview.can_withdraw,
                "recommendation": if preview.can_withdraw {
                    format!("You can withdraw {} {} (after {} {} gas fees)",
                           preview.net_amount, request.network.to_uppercase(),
                           preview.estimated_gas_fee, request.network.to_uppercase())
                } else {
                    format!("Cannot withdraw {} {} - insufficient funds after gas fees",
                           request.amount, request.network.to_uppercase())
                },
                "note": "Gas fees are automatically deducted from payment amounts to ensure successful withdrawal",
                "created_at": chrono::Utc::now().to_rfc3339()
            });

            Ok(HttpResponse::Ok().json(response))
        }
        Err(e) => {
            log::error!(" Failed to generate EVM withdrawal preview: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "preview_failed",
                "message": format!("Failed to generate withdrawal preview: {}", e),
                "network": request.network,
                "amount": request.amount
            })))
        }
    }
}

pub async fn create_multi_wallet_withdrawal(
    app_state: web::Data<AppState>,
    merchant: MerchantAuth,
    withdrawal_data: web::Json<CreateWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    let request = withdrawal_data.into_inner();

    if let Err(validation_errors) = request.validate() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Invalid withdrawal request",
            "details": format!("{:?}", validation_errors)
        })));
    }

    log::info!(
        " Creating multi-wallet withdrawal for merchant {}: {} {} to {}",
        merchant.merchant_id,
        request.amount,
        request.network,
        request.to_address
    );

    let existing_withdrawal = WithdrawalRequest::find()
        .filter(withdrawal_request::Column::MerchantId.eq(&merchant.merchant_id))
        .filter(withdrawal_request::Column::IdempotencyKey.eq(&request.idempotency_key))
        .one(&app_state.db)
        .await;

    match existing_withdrawal {
        Ok(Some(existing)) => {
            log::warn!(
                "Duplicate withdrawal attempt with idempotency key: {}",
                request.idempotency_key
            );
            return Ok(HttpResponse::Conflict().json(serde_json::json!({
                "error": "Duplicate withdrawal request",
                "existing_withdrawal_id": existing.id,
                "status": existing.status
            })));
        }
        Ok(None) => {
            // Continue with new withdrawal
        }
        Err(e) => {
            log::error!("Error checking idempotency key: {:?}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Database error",
                "details": format!("{:?}", e)
            })));
        }
    }

    let processor = MultiWalletProcessor::new(std::sync::Arc::new(app_state.as_ref().clone()));

    match processor
        .execute_withdrawal(&merchant.merchant_id, &request)
        .await
    {
        Ok(response) => {
            log::info!(
                "   Multi-wallet withdrawal created successfully: {}",
                response.id
            );
            Ok(HttpResponse::Created().json(response))
        }
        Err(e) => {
            log::error!("Failed to create multi-wallet withdrawal: {:?}", e);

            let (mut status_code, error_message) = match &e {
                AppError::InsufficientBalance(_) => (
                    HttpResponse::BadRequest(),
                    "Insufficient balance across all wallets",
                ),
                AppError::InvalidInput(_) => {
                    (HttpResponse::BadRequest(), "Invalid withdrawal amount")
                }
                AppError::DatabaseError(_) => (
                    HttpResponse::ServiceUnavailable(),
                    "Database connectivity issue",
                ),
                _ => (HttpResponse::InternalServerError(), "Internal server error"),
            };

            Ok(status_code.json(serde_json::json!({
                "error": error_message,
                "details": format!("{:?}", e)
            })))
        }
    }
}

/// Get detailed withdrawal information including multi-wallet details
pub async fn get_withdrawal_details(
    app_state: web::Data<AppState>,
    merchant: MerchantAuth,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let withdrawal_id = path.into_inner();

    log::info!(
        " Getting detailed withdrawal info for {} by merchant {}",
        withdrawal_id,
        merchant.merchant_id
    );

    let withdrawal = WithdrawalRequest::find()
        .filter(withdrawal_request::Column::Id.eq(&withdrawal_id))
        .filter(withdrawal_request::Column::MerchantId.eq(&merchant.merchant_id))
        .one(&app_state.db)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Database query failed: {}", e)))?;

    match withdrawal {
        Some(w) => {
            let mut tx_hashes = vec![];
            if let Some(hash) = &w.tx_hash {
                tx_hashes.push(hash.clone());
            }

            let wallet_usage = vec![];

            let network = w.network.clone();
            let environment = w.environment.clone();

            let explorer_urls = if let Some(tx_hash) = &w.tx_hash {
                vec![MerchantAuth::get_explorer_url_for_network(
                    &network,
                    &environment,
                    tx_hash,
                )]
            } else {
                vec![]
            };

            let response = WithdrawalResponse {
                id: w.id,
                merchant_id: w.merchant_id,
                external_id: Some(w.external_id),
                idempotency_key: w.idempotency_key,
                environment: w.environment,
                network: w.network,
                currency: w.currency,
                to_address: w.to_address,
                amount: w.amount.to_string(),
                requested_amount: w.amount.to_string(),
                transferred_amount: w.amount.to_string(),
                amount_type: "crypto".to_string(),
                usd_equivalent: None,
                fee: w.fee.map(|f| f.to_string()),
                total_fee: w.fee.map(|f| f.to_string()),
                net_amount: (w.amount - w.fee.unwrap_or_default()).to_string(),
                status: w.status,
                wallets_used: wallet_usage,
                tx_hash: w.tx_hash.clone(),
                tx_hashes,
                confirmations: w.blockchain_confirmations.unwrap_or(0),
                blockchain_confirmations: w.blockchain_confirmations,
                required_confirmations: match network.to_lowercase().as_str() {
                    "sol" | "solana" => 32,
                    "eth" | "ethereum" => 12,
                    "bnb" | "bsc" => 12,
                    "btc" | "bitcoin" => 6,
                    _ => 12,
                },
                explorer_url: explorer_urls.first().cloned(),
                explorer_urls,
                created_at: w.created_at.into(),
                processed_at: w.processed_at.map(|dt| dt.into()),
                confirmed_at: w.confirmed_at.map(|dt| dt.into()),
                updated_at: w.updated_at.into(),
                broadcast_at: w.broadcast_at.map(|dt| dt.into()),
                failed_at: w.failed_at.map(|dt| dt.into()),
                failure_reason: w.failure_reason,
            };

            Ok(HttpResponse::Ok().json(response))
        }
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Withdrawal not found"
        }))),
    }
}

/// GET /api/v1/merchant/multi-wallet/balances?environment=mainnet|testnet (Enhanced)
pub async fn get_enhanced_multi_wallet_balances(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<BalanceQuery>,
) -> Result<HttpResponse, AppError> {
    log::info!("get_multi_wallet_balances called with query: {:?}", query);

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!("User claims found: user_id={}", claims.id);

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            log::error!("Database error finding merchants: {:?}", e);
            AppError::DatabaseError(format!("Failed to find merchants: {}", e))
        })?;

    let environment = query.environment.as_deref().unwrap_or("testnet");
    log::info!("Environment: {}", environment);

    if !matches!(environment, "mainnet" | "testnet") {
        log::warn!("Invalid environment: {}", environment);
        return Err(AppError::ValidationError(
            "Environment must be 'mainnet' or 'testnet'".to_string(),
        ));
    }

    if merchants.is_empty() {
        log::warn!("No merchants found for user {}", claims.id);
        return Ok(HttpResponse::Ok().json(MultiWalletBalanceResponse {
            environment: environment.to_string(),
            total_wallets: 0,
            total_usd_value: "0.00".to_string(),
            wallet_balances: vec![],
            currency_totals: std::collections::HashMap::new(),
            last_updated: chrono::Utc::now(),
        }));
    }

    let merchant = &merchants[0];
    log::info!("Using merchant {} for user {}", merchant.id, claims.id);

    let balances =
        MultiWalletService::get_multi_wallet_balances(&app_state, &merchant.id, environment)
            .await
            .map_err(|e| {
                log::error!(
                    "MultiWalletService::get_multi_wallet_balances failed: {:?}",
                    e
                );
                e
            })?;

    log::info!(
        "Successfully retrieved multi-wallet balances with {} wallets",
        balances.total_wallets
    );
    Ok(HttpResponse::Ok().json(balances))
}

/// POST /api/v1/merchant/multi-wallet/withdrawals/preview (Enhanced)
pub async fn preview_enhanced_withdrawal_plan(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<MultiWalletWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    log::info!("preview_withdrawal_plan called with body: {:?}", body);

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!("User claims found: user_id={}", claims.id);

    body.validate().map_err(|e| {
        log::error!("Validation error: {:?}", e);
        AppError::ValidationError(format!("Invalid request: {}", e))
    })?;

    log::info!("Request validation passed");

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            log::error!("Database error finding merchants: {:?}", e);
            AppError::DatabaseError(format!("Failed to find merchants: {}", e))
        })?;

    if merchants.is_empty() {
        log::warn!("No merchants found for user {}", claims.id);
        return Err(AppError::BadRequest(
            "No merchant account found. Please complete your merchant setup.".to_string(),
        ));
    }

    let merchant = &merchants[0];
    log::info!("Using merchant {} for withdrawal preview", merchant.id);

    let request_body = body.into_inner();

    let preview = MultiWalletService::preview_withdrawal(&app_state, &merchant.id, &request_body)
        .await
        .map_err(|e| {
            log::error!("MultiWalletService::preview_withdrawal failed: {:?}", e);
            e
        })?;

    log::info!(
        "Successfully generated withdrawal preview with {} wallets selected",
        preview.selected_wallets
    );
    Ok(HttpResponse::Ok().json(preview))
}

/// POST /api/v1/merchant/multi-wallet/withdrawals (Enhanced)
pub async fn create_enhanced_multi_wallet_withdrawal(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<MultiWalletWithdrawalRequest>,
) -> Result<HttpResponse, AppError> {
    log::info!(
        "create_multi_wallet_withdrawal called with body: {:?}",
        body
    );

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!("User claims found: user_id={}", claims.id);

    body.validate().map_err(|e| {
        log::error!("Validation error: {:?}", e);
        AppError::ValidationError(format!("Invalid request: {}", e))
    })?;

    log::info!("Request validation passed");

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            log::error!("Database error finding merchants: {:?}", e);
            AppError::DatabaseError(format!("Failed to find merchants: {}", e))
        })?;

    if merchants.is_empty() {
        log::warn!("No merchants found for user {}", claims.id);
        return Err(AppError::BadRequest(
            "No merchant account found. Please complete your merchant setup.".to_string(),
        ));
    }

    let merchant = &merchants[0];
    log::info!("Using merchant {} for multi-wallet withdrawal", merchant.id);

    let request_body = body.into_inner();

    let withdrawal =
        MultiWalletService::execute_multi_wallet_withdrawal(&app_state, &merchant.id, request_body)
            .await
            .map_err(|e| {
                log::error!(
                    "MultiWalletService::execute_multi_wallet_withdrawal failed: {:?}",
                    e
                );
                e
            })?;

    log::info!(
        "Successfully created multi-wallet withdrawal with id: {}",
        withdrawal.id
    );
    Ok(HttpResponse::Created().json(withdrawal))
}

/// GET /api/v1/merchant/multi-wallet/withdrawals/{id} (Enhanced)
pub async fn get_enhanced_withdrawal_details(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let withdrawal_id = path.into_inner();
    log::info!(
        "get_withdrawal_details called for withdrawal_id: {}",
        withdrawal_id
    );

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or_else(|| {
            log::error!("No user claims found in request");
            AppError::Unauthorized("Unauthorized".to_string())
        })?;

    log::info!("User claims found: user_id={}", claims.id);

    let merchants = Merchant::find()
        .filter(merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await
        .map_err(|e| {
            log::error!("Database error finding merchants: {:?}", e);
            AppError::DatabaseError(format!("Failed to find merchants: {}", e))
        })?;

    if merchants.is_empty() {
        log::warn!("No merchants found for user {}", claims.id);
        return Err(AppError::BadRequest(
            "No merchant account found".to_string(),
        ));
    }

    let withdrawal = withdrawal_request::Entity::find()
        .filter(withdrawal_request::Column::Id.eq(&withdrawal_id))
        .one(&app_state.db)
        .await?;

    match withdrawal {
        Some(w) => {
            let user_merchant_ids: Vec<String> = merchants.iter().map(|m| m.id.clone()).collect();
            if !user_merchant_ids.contains(&w.merchant_id) {
                return Err(AppError::Unauthorized(
                    "Access denied to this withdrawal".to_string(),
                ));
            }

            let response = MultiWalletWithdrawalResponse {
                id: w.id,
                merchant_id: w.merchant_id,
                external_id: Some(w.external_id),
                idempotency_key: w.idempotency_key,
                environment: w.environment,
                to_address: w.to_address,
                requested_amount: w.amount.to_string(),
                requested_amount_type: "usd".to_string(),
                transferred_amount_usd: w.net_amount.to_string(),
                total_fees_usd: w.fee.unwrap_or_default().to_string(),
                net_amount_usd: w.net_amount.to_string(),
                status: w.status,
                wallets_used: 1,
                wallets_failed: 0,
                tx_hashes: w.tx_hash.into_iter().collect(),
                transactions: vec![],
                created_at: w.created_at.into(),
                processed_at: w.processed_at.map(|dt| dt.into()),
                confirmed_at: w.confirmed_at.map(|dt| dt.into()),
                updated_at: w.updated_at.into(),
            };

            Ok(HttpResponse::Ok().json(response))
        }
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Withdrawal not found"
        }))),
    }
}

impl MerchantAuth {
    async fn try_solana_multi_wallet_withdrawal_fallback(
        app_state: &web::Data<AppState>,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
    ) -> Result<HttpResponse, AppError> {
        log::info!(
            "💎 Attempting Solana multi-wallet aggregation for {} SOL to {}",
            request.amount,
            request.to_address
        );

        match SolanaMultiWalletService::execute_multi_wallet_withdrawal(
            &app_state.db,
            merchant_id,
            request,
            &request.to_address,
        )
        .await
        {
            Ok(result) => {
                log::info!(
                    "   Solana multi-wallet withdrawal successful: {} lamports from {} wallets",
                    result.total_transferred_lamports,
                    result.wallets_used.len()
                );

                log::info!("🔄 Syncing balances after successful multi-wallet withdrawal");
                if let Err(sync_error) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(&app_state, merchant_id, &request.environment).await {
                    log::warn!("Balance sync failed after multi-wallet withdrawal: {:?}", sync_error);
                }

                let response = serde_json::json!({
                    "id": result.withdrawal_id,
                    "merchant_id": merchant_id,
                    "external_id": result.withdrawal_id.clone(),
                    "idempotency_key": request.idempotency_key,
                    "environment": request.environment,
                    "network": request.network,
                    "currency": "solana",
                    "to_address": request.to_address,
                    "amount": format!("{}", result.total_transferred_lamports as f64 / 1_000_000_000.0),
                    "requested_amount": request.amount,
                    "transferred_amount": format!("{}", result.total_transferred_lamports as f64 / 1_000_000_000.0),
                    "amount_type": "crypto",
                    "usd_equivalent": null,
                    "fee": format!("{}", result.total_fee_lamports as f64 / 1_000_000_000.0),
                    "total_fee": format!("{}", result.total_fee_lamports as f64 / 1_000_000_000.0),
                    "net_amount": format!("{}", result.total_transferred_lamports as f64 / 1_000_000_000.0),
                    "status": "confirmed",
                    "wallets_used": result.wallets_used,
                    "tx_hash": result.tx_hashes.first().cloned(),
                    "tx_hashes": result.tx_hashes,
                    "confirmations": 1,
                    "blockchain_confirmations": Some(1),
                    "required_confirmations": 32,
                    "explorer_url": result.explorer_urls.first().cloned(),
                    "explorer_urls": result.explorer_urls,
                    "multi_wallet": true,
                    "aggregation_method": "solana_deposit_discovery",
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "processed_at": chrono::Utc::now().to_rfc3339(),
                    "confirmed_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                    "broadcast_at": chrono::Utc::now().to_rfc3339(),
                    "message": format!("Withdrawal completed using {} Solana wallets via deposit aggregation", result.wallets_used.len())
                });

                Ok(HttpResponse::Created().json(response))
            }
            Err(e) => {
                log::error!(" Solana multi-wallet withdrawal failed: {}", e);

                let error_response = match &e {
                    AppError::ValidationError(msg) if msg.contains("Insufficient balance") => {
                        serde_json::json!({
                            "error": "insufficient_balance_all_solana_wallets",
                            "message": format!("Insufficient balance across all Solana deposit wallets: {}", msg),
                            "code": "SOLANA_MULTI_WALLET_INSUFFICIENT",
                            "details": {
                                "requested_amount": request.amount,
                                "network": "SOL",
                                "environment": "mainnet",
                                "attempted_strategies": ["single_wallet", "solana_multi_wallet_aggregation"],
                                "suggestion": "Add more SOL to your deposit wallets or reduce withdrawal amount",
                                "note": "Multi-wallet aggregation attempted across all wallets with paid payments"
                            }
                        })
                    }
                    AppError::ValidationError(msg) if msg.contains("No deposit wallets found") => {
                        serde_json::json!({
                            "error": "no_solana_deposit_wallets",
                            "message": "No SOL deposit wallets found for merchant. Create payments first.",
                            "code": "NO_DEPOSIT_WALLETS",
                            "details": {
                                "network": "SOL",
                                "environment": "mainnet",
                                "suggestion": "Process some SOL payments first to create deposit wallets, then try withdrawal again"
                            }
                        })
                    }
                    _ => {
                        serde_json::json!({
                            "error": "solana_withdrawal_failed",
                            "message": format!("Solana multi-wallet withdrawal failed: {}", e),
                            "code": "SOLANA_WITHDRAWAL_FAILED",
                            "details": {
                                "requested_amount": request.amount,
                                "network": "SOL",
                                "error_type": "solana_multi_wallet_failed"
                            }
                        })
                    }
                };

                Ok(HttpResponse::BadRequest().json(error_response))
            }
        }
    }

    async fn try_evm_multi_wallet_withdrawal_fallback(
        app_state: &web::Data<AppState>,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
    ) -> Result<HttpResponse, AppError> {
        log::info!(
            "⚡ Attempting EVM multi-wallet aggregation for {} {} to {}",
            request.amount,
            request.network.to_uppercase(),
            request.to_address
        );

        match EvmMultiWalletService::execute_multi_wallet_withdrawal(
            &app_state.db,
            merchant_id,
            request,
            &request.to_address,
        )
        .await
        {
            Ok(result) => {
                log::info!(
                    "   EVM multi-wallet withdrawal successful: {} wei from {} wallets",
                    result.total_transferred_wei,
                    result.wallets_used.len()
                );

                let withdrawal_record = withdrawal_request::ActiveModel {
                    id: sea_orm::Set(result.withdrawal_id.clone()),
                    merchant_id: sea_orm::Set(merchant_id.to_string()),
                    wallet_id: sea_orm::Set(
                        result
                            .wallets_used
                            .first()
                            .map(|w| w.wallet_id.clone())
                            .unwrap_or_default(),
                    ),
                    external_id: sea_orm::Set(result.withdrawal_id.clone()),
                    idempotency_key: sea_orm::Set(request.idempotency_key.clone()),
                    environment: sea_orm::Set(request.environment.clone()),
                    network: sea_orm::Set(request.network.clone()),
                    currency: sea_orm::Set(request.network.to_uppercase()),
                    to_address: sea_orm::Set(request.to_address.clone()),
                    amount: sea_orm::Set(Decimal::from_str(&request.amount).unwrap_or_default()),
                    fee: sea_orm::Set(Some(
                        Decimal::from(result.total_gas_cost.as_u128())
                            / Decimal::from(1_000_000_000_000_000_000u128),
                    )),
                    net_amount: sea_orm::Set(
                        Decimal::from(result.total_transferred_wei.as_u128())
                            / Decimal::from(1_000_000_000_000_000_000u128),
                    ),
                    status: sea_orm::Set("confirmed".to_string()),
                    tx_hash: sea_orm::Set(result.tx_hashes.first().cloned()),
                    blockchain_confirmations: sea_orm::Set(Some(1)),
                    required_confirmations: sea_orm::Set(12),
                    confirmed_at: sea_orm::Set(Some(chrono::Utc::now().into())),
                    processed_at: sea_orm::Set(Some(chrono::Utc::now().into())),
                    broadcast_at: sea_orm::Set(Some(chrono::Utc::now().into())),
                    created_at: sea_orm::Set(chrono::Utc::now().into()),
                    updated_at: sea_orm::Set(chrono::Utc::now().into()),
                    metadata: sea_orm::Set(Some(
                        serde_json::json!({
                            "wallets_used": result.wallets_used,
                            "tx_hashes": result.tx_hashes,
                            "multi_wallet": true,
                            "aggregation_method": "evm_deposit_discovery"
                        })
                        .to_string(),
                    )),
                    finality_required: sea_orm::Set(12),
                    finality_reached_at: sea_orm::Set(None),
                    replaced_by_tx: sea_orm::Set(None),
                    reorg_depth: sea_orm::Set(0),
                    lifecycle_state: sea_orm::Set(WithdrawalLifecycleState::Quoted.to_string()),
                    withdrawal_funds_state: sea_orm::Set(
                        WithdrawalFundsState::Unlocked.to_string(),
                    ),
                    ..Default::default()
                };

                withdrawal_record.insert(&app_state.db).await.map_err(|e| {
                    log::error!("Failed to save EVM multi-wallet withdrawal record: {}", e);
                    AppError::DatabaseError(format!("Failed to save withdrawal: {}", e))
                })?;

                log::info!("   EVM multi-wallet withdrawal record saved to database");

                log::info!("🔄 Syncing balances after successful EVM multi-wallet withdrawal");
                if let Err(sync_error) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(&app_state, merchant_id, &request.environment).await {
                    log::warn!("Balance sync failed after EVM multi-wallet withdrawal: {:?}", sync_error);
                }

                let transferred_amount =
                    result.total_transferred_wei.as_u128() as f64 / 1_000_000_000_000_000_000.0;
                let total_fee =
                    result.total_gas_cost.as_u128() as f64 / 1_000_000_000_000_000_000.0;

                let response = serde_json::json!({
                    "id": result.withdrawal_id,
                    "merchant_id": merchant_id,
                    "external_id": result.withdrawal_id.clone(),
                    "idempotency_key": request.idempotency_key,
                    "environment": request.environment,
                    "network": request.network,
                    "currency": request.network.to_uppercase(),
                    "to_address": request.to_address,
                    "amount": format!("{}", transferred_amount),
                    "requested_amount": request.amount,
                    "transferred_amount": format!("{}", transferred_amount),
                    "amount_type": "crypto",
                    "usd_equivalent": null,
                    "fee": format!("{}", total_fee),
                    "total_fee": format!("{}", total_fee),
                    "net_amount": format!("{}", transferred_amount),
                    "status": "confirmed",
                    "wallets_used": result.wallets_used,
                    "tx_hash": result.tx_hashes.first().cloned(),
                    "tx_hashes": result.tx_hashes,
                    "confirmations": 1,
                    "blockchain_confirmations": Some(1),
                    "required_confirmations": 12,
                    "explorer_url": result.explorer_urls.first().cloned(),
                    "explorer_urls": result.explorer_urls,
                    "multi_wallet": true,
                    "aggregation_method": "evm_deposit_discovery",
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "processed_at": chrono::Utc::now().to_rfc3339(),
                    "confirmed_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                    "broadcast_at": chrono::Utc::now().to_rfc3339(),
                    "message": format!("Withdrawal completed using {} {} wallets via deposit aggregation", result.wallets_used.len(), request.network.to_uppercase())
                });

                Ok(HttpResponse::Created().json(response))
            }
            Err(e) => {
                log::error!(" EVM multi-wallet withdrawal failed: {}", e);

                let error_response = match &e {
                    AppError::ValidationError(msg) if msg.contains("Insufficient") => {
                        let wallet_breakdown = match Self::get_wallet_breakdown_for_error(
                            &app_state.db,
                            merchant_id,
                            &request.network,
                            &request.environment,
                        )
                        .await
                        {
                            Ok(breakdown) => Some(breakdown),
                            Err(_) => None,
                        };

                        let mut details = serde_json::json!({
                            "requested_amount": request.amount,
                            "network": request.network.to_uppercase(),
                            "environment": request.environment,
                            "attempted_strategies": ["single_wallet", "evm_multi_wallet_aggregation"],
                            "suggestion": format!("Add more {} to your deposit wallets or reduce withdrawal amount", request.network.to_uppercase()),
                            "note": "Fresh on-chain balances checked with dynamic gas estimation"
                        });

                        if let Some(breakdown) = wallet_breakdown {
                            details["wallet_breakdown"] = breakdown;
                        }

                        serde_json::json!({
                            "error": format!("insufficient_balance_all_{}_wallets", request.network.to_lowercase()),
                            "message": format!("Insufficient balance across all {} deposit wallets: {}", request.network.to_uppercase(), msg),
                            "code": format!("{}_MULTI_WALLET_INSUFFICIENT", request.network.to_uppercase()),
                            "details": details
                        })
                    }
                    AppError::ValidationError(msg)
                        if msg.contains("No") && msg.contains("wallets found") =>
                    {
                        serde_json::json!({
                            "error": format!("no_{}_deposit_wallets", request.network.to_lowercase()),
                            "message": format!("No {} deposit wallets found for merchant. Create payments first.", request.network.to_uppercase()),
                            "code": "NO_DEPOSIT_WALLETS",
                            "details": {
                                "network": request.network.to_uppercase(),
                                "environment": "mainnet",
                                "suggestion": format!("Process some {} payments first to create deposit wallets, then try withdrawal again", request.network.to_uppercase())
                            }
                        })
                    }
                    _ => {
                        serde_json::json!({
                            "error": format!("{}_withdrawal_failed", request.network.to_lowercase()),
                            "message": format!("{} multi-wallet withdrawal failed: {}", request.network.to_uppercase(), e),
                            "code": format!("{}_WITHDRAWAL_FAILED", request.network.to_uppercase()),
                            "details": {
                                "requested_amount": request.amount,
                                "network": request.network.to_uppercase(),
                                "error_type": "evm_multi_wallet_failed"
                            }
                        })
                    }
                };

                Ok(HttpResponse::BadRequest().json(error_response))
            }
        }
    }

    fn get_explorer_url_for_network(network: &str, environment: &str, tx_hash: &str) -> String {
        match network.to_lowercase().as_str() {
            "bnb" | "bsc" => {
                if environment == "mainnet" {
                    format!("https://bscscan.com/tx/{}", tx_hash)
                } else {
                    format!("https://testnet.bscscan.com/tx/{}", tx_hash)
                }
            }
            "eth" | "ethereum" => {
                if environment == "mainnet" {
                    format!("https://etherscan.io/tx/{}", tx_hash)
                } else {
                    format!("https://sepolia.etherscan.io/tx/{}", tx_hash)
                }
            }
            "sol" | "solana" => {
                let cluster = if environment == "mainnet" {
                    "mainnet-beta"
                } else {
                    "testnet"
                };
                format!(
                    "https://explorer.solana.com/tx/{}?cluster={}",
                    tx_hash, cluster
                )
            }
            "btc" | "bitcoin" => {
                if environment == "mainnet" {
                    format!("https://blockstream.info/tx/{}", tx_hash)
                } else {
                    format!("https://blockstream.info/testnet/tx/{}", tx_hash)
                }
            }
            _ => {
                format!("https://etherscan.io/tx/{}", tx_hash)
            }
        }
    }

    async fn get_wallet_breakdown_for_error(
        db: &DatabaseConnection,
        merchant_id: &str,
        network: &str,
        environment: &str,
    ) -> Result<serde_json::Value, AppError> {
        use crate::merchant::services::evm_multi_wallet_service::EvmMultiWalletService;

        log::info!(" Generating wallet breakdown for error response...");

        let deposit_wallets = EvmMultiWalletService::discover_deposit_wallets_for_breakdown(
            db,
            merchant_id,
            network,
            environment,
        )
        .await?;
        let candidates = EvmMultiWalletService::evaluate_candidates_for_breakdown(
            deposit_wallets,
            network,
            environment,
        )
        .await?;

        let mut wallet_breakdown = Vec::new();
        let mut total_balance = 0.0f64;
        let mut total_transferable = 0.0f64;
        let mut total_gas_fees = 0.0f64;

        for candidate in candidates {
            let balance_ether = candidate.native_balance_wei.as_u128() as f64 / 1e18;
            let gas_ether = candidate.estimated_gas_cost.as_u128() as f64 / 1e18;
            let transferable_ether = candidate.transferable_amount.as_u128() as f64 / 1e18;

            total_balance += balance_ether;
            total_transferable += transferable_ether;
            total_gas_fees += gas_ether;

            wallet_breakdown.push(serde_json::json!({
                "address": candidate.address,
                "balance": format!("{:.8}", balance_ether),
                "gas_estimate": format!("{:.8}", gas_ether),
                "transferable": format!("{:.8}", transferable_ether),
            }));
        }

        Ok(serde_json::json!({
            "total_wallets": wallet_breakdown.len(),
            "total_balance": format!("{:.8}", total_balance),
            "total_gas_fees": format!("{:.8}", total_gas_fees),
            "total_transferable": format!("{:.8}", total_transferable),
            "wallets": wallet_breakdown,
        }))
    }

    async fn try_btc_multi_wallet_withdrawal_fallback(
        app_state: &web::Data<AppState>,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
    ) -> Result<HttpResponse, AppError> {
        log::info!(
            "₿ Attempting Bitcoin multi-wallet aggregation for {} BTC to {}",
            request.amount,
            request.to_address
        );

        match BtcMultiWalletService::execute_multi_wallet_withdrawal(
            &app_state.db,
            merchant_id,
            request,
            &request.to_address,
        )
        .await
        {
            Ok(result) => {
                log::info!(
                    "   Bitcoin multi-wallet withdrawal successful: {} satoshis from {} wallets",
                    result.total_transferred_satoshis,
                    result.wallets_used.len()
                );

                log::info!("🔄 Syncing balances after successful Bitcoin multi-wallet withdrawal");
                if let Err(sync_error) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(&app_state, merchant_id, &request.environment).await {
                    log::warn!("Balance sync failed after Bitcoin multi-wallet withdrawal: {:?}", sync_error);
                }

                let transferred_amount = result.total_transferred_satoshis as f64 / 100_000_000.0;
                let total_fee = result.total_fee_satoshis as f64 / 100_000_000.0;

                let response = serde_json::json!({
                    "id": result.withdrawal_id,
                    "merchant_id": merchant_id,
                    "external_id": result.withdrawal_id.clone(),
                    "idempotency_key": request.idempotency_key,
                    "environment": request.environment,
                    "network": request.network,
                    "currency": "BTC",
                    "to_address": request.to_address,
                    "amount": format!("{}", transferred_amount),
                    "requested_amount": request.amount,
                    "transferred_amount": format!("{}", transferred_amount),
                    "amount_type": "crypto",
                    "usd_equivalent": null,
                    "fee": format!("{}", total_fee),
                    "total_fee": format!("{}", total_fee),
                    "net_amount": format!("{}", transferred_amount),
                    "status": "confirmed",
                    "wallets_used": result.wallets_used,
                    "tx_hash": result.tx_hashes.first().cloned(),
                    "tx_hashes": result.tx_hashes,
                    "confirmations": 1,
                    "blockchain_confirmations": Some(1),
                    "required_confirmations": 6,
                    "explorer_url": result.explorer_urls.first().cloned(),
                    "explorer_urls": result.explorer_urls,
                    "multi_wallet": true,
                    "aggregation_method": "btc_deposit_discovery",
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "processed_at": chrono::Utc::now().to_rfc3339(),
                    "confirmed_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                    "broadcast_at": chrono::Utc::now().to_rfc3339(),
                    "message": format!("Withdrawal completed using {} Bitcoin wallets via deposit aggregation", result.wallets_used.len())
                });

                Ok(HttpResponse::Created().json(response))
            }
            Err(e) => {
                log::error!(" Bitcoin multi-wallet withdrawal failed: {}", e);

                let error_response = match &e {
                    AppError::ValidationError(msg) if msg.contains("Insufficient") => {
                        serde_json::json!({
                            "error": "insufficient_balance_all_btc_wallets",
                            "message": format!("Insufficient balance across all Bitcoin deposit wallets: {}", msg),
                            "code": "BTC_MULTI_WALLET_INSUFFICIENT",
                            "details": {
                                "requested_amount": request.amount,
                                "network": "BTC",
                                "environment": "mainnet",
                                "attempted_strategies": ["single_wallet", "btc_multi_wallet_aggregation"],
                                "suggestion": "Add more BTC to your deposit wallets or reduce withdrawal amount",
                                "note": "Multi-wallet aggregation attempted across all wallets with paid payments"
                            }
                        })
                    }
                    AppError::ValidationError(msg)
                        if msg.contains("No") && msg.contains("wallets found") =>
                    {
                        serde_json::json!({
                            "error": "no_btc_deposit_wallets",
                            "message": "No BTC deposit wallets found for merchant. Create payments first.",
                            "code": "NO_DEPOSIT_WALLETS",
                            "details": {
                                "network": "BTC",
                                "environment": "mainnet",
                                "suggestion": "Process some BTC payments first to create deposit wallets, then try withdrawal again"
                            }
                        })
                    }
                    _ => {
                        serde_json::json!({
                            "error": "btc_withdrawal_failed",
                            "message": format!("Bitcoin multi-wallet withdrawal failed: {}", e),
                            "code": "BTC_WITHDRAWAL_FAILED",
                            "details": {
                                "requested_amount": request.amount,
                                "network": "BTC",
                                "error_type": "btc_multi_wallet_failed"
                            }
                        })
                    }
                };

                Ok(HttpResponse::BadRequest().json(error_response))
            }
        }
    }

    async fn try_multi_wallet_withdrawal_fallback(
        app_state: &web::Data<AppState>,
        merchant_id: &str,
        request: &CreateWithdrawalRequest,
    ) -> Result<HttpResponse, AppError> {
        log::info!(
            "🔄 Attempting multi-wallet withdrawal fallback for {} {} to {}",
            request.amount,
            request.network,
            request.to_address
        );

        let multi_wallet_request =
            crate::merchant::models::withdrawal::MultiWalletWithdrawalRequest {
                environment: request.environment.clone(),
                to_address: request.to_address.clone(),
                amount: request.amount.clone(),
                amount_type: request.amount_type.clone().unwrap_or("crypto".to_string()),
                currency: Some(Self::network_to_currency(&request.network)),
                idempotency_key: request.idempotency_key.clone(),
                external_id: request.external_id.clone(),
                wallet_selection_strategy: request.wallet_selection_strategy.clone(),
            };

        log::info!(
            " Executing multi-wallet withdrawal with {} strategy",
            multi_wallet_request
                .wallet_selection_strategy
                .as_deref()
                .unwrap_or("optimal")
        );

        match MultiWalletService::execute_multi_wallet_withdrawal(
            app_state,
            merchant_id,
            multi_wallet_request,
        )
        .await
        {
            Ok(multi_response) => {
                log::info!(
                    "   Multi-wallet withdrawal successful: {} wallets used, {} failed",
                    multi_response.wallets_used,
                    multi_response.wallets_failed
                );

                let response = serde_json::json!({
                    "id": multi_response.id,
                    "merchant_id": multi_response.merchant_id,
                    "external_id": multi_response.external_id,
                    "idempotency_key": multi_response.idempotency_key,
                    "environment": multi_response.environment,
                    "network": request.network,
                    "currency": Self::network_to_currency(&request.network),
                    "to_address": multi_response.to_address,
                    "requested_amount": multi_response.requested_amount,
                    "transferred_amount": multi_response.transferred_amount_usd,
                    "amount_type": multi_response.requested_amount_type,
                    "usd_equivalent": multi_response.transferred_amount_usd,
                    "total_fee": multi_response.total_fees_usd,
                    "net_amount": multi_response.net_amount_usd,
                    "status": multi_response.status,
                    "wallets_used": multi_response.wallets_used,
                    "wallets_failed": multi_response.wallets_failed,
                    "tx_hashes": multi_response.tx_hashes,
                    "multi_wallet": true,
                    "blockchain_confirmations": 1,
                    "required_confirmations": 32,
                    "explorer_urls": multi_response.tx_hashes.iter()
                        .map(|tx| format!("https://explorer.solana.com/tx/{}?cluster=mainnet", tx))
                        .collect::<Vec<String>>(),
                    "created_at": multi_response.created_at,
                    "processed_at": multi_response.processed_at,
                    "confirmed_at": multi_response.confirmed_at,
                    "updated_at": multi_response.updated_at,
                    "transactions": multi_response.transactions,
                    "message": format!("Withdrawal completed using {} wallets due to insufficient single wallet balance", multi_response.wallets_used)
                });

                Ok(HttpResponse::Created().json(response))
            }
            Err(e) => {
                log::error!(" Multi-wallet withdrawal also failed: {}", e);

                let error_response = match &e {
                    AppError::ValidationError(msg) => {
                        serde_json::json!({
                            "error": "insufficient_balance_all_wallets",
                            "message": format!("Insufficient balance across all wallets: {}", msg),
                            "code": "MULTI_WALLET_INSUFFICIENT",
                            "details": {
                                "requested_amount": request.amount,
                                "network": request.network,
                                "attempted_strategies": ["single_wallet", "multi_wallet_aggregation"],
                                "suggestion": "Add more funds to your wallets or reduce withdrawal amount"
                            }
                        })
                    }
                    _ => {
                        serde_json::json!({
                            "error": "withdrawal_failed",
                            "message": format!("Both single-wallet and multi-wallet withdrawal failed: {}", e),
                            "code": "WITHDRAWAL_FAILED",
                            "details": {
                                "requested_amount": request.amount,
                                "network": request.network,
                                "error_type": "multi_wallet_fallback_failed"
                            }
                        })
                    }
                };

                Ok(HttpResponse::BadRequest().json(error_response))
            }
        }
    }

    fn network_to_currency(network: &str) -> String {
        match network.to_lowercase().as_str() {
            "sol" => "solana".to_string(),
            "eth" => "ethereum".to_string(),
            "btc" => "bitcoin".to_string(),
            "bnb" => "bnb".to_string(),
            "usdt_erc20" => "usdt".to_string(),
            "usdt_bep20" => "usdt".to_string(),
            _ => network.to_string(),
        }
    }

    fn currency_to_network(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "solana" | "sol" => "sol".to_string(),
            "ethereum" | "eth" => "eth".to_string(),
            "bitcoin" | "btc" => "btc".to_string(),
            "bnb" => "bnb".to_string(),
            "usdt" => "usdt_erc20".to_string(),
            _ => currency.to_string(),
        }
    }
}

async fn handle_evm_withdrawal(
    app_state: &web::Data<AppState>,
    merchant: &merchant::Model,
    request: &CreateWithdrawalRequest,
) -> Result<HttpResponse, AppError> {
    use crate::merchant::services::evm_withdrawal_service::EvmWithdrawalService;

    log::info!(
        "🔷 Executing EVM withdrawal for {} {} to {}",
        request.amount,
        request.network.to_uppercase(),
        request.to_address
    );

    let wallet = match get_wallet_for_network(
        &app_state.db,
        &merchant.user_id,
        &request.network,
        &request.environment,
    )
    .await
    {
        Ok(wallet) => wallet,
        Err(e) => {
            log::error!("Failed to get wallet for EVM withdrawal: {}", e);
            return Err(e);
        }
    };

    match EvmWithdrawalService::execute_withdrawal(
        &app_state.db,
        &merchant.id,
        request,
        &request.idempotency_key,
    )
    .await
    {
        Ok(result) => {
            log::info!(
                "   EVM withdrawal initiated: {} tx_hashes, status={}",
                result.tx_hashes.len(),
                result.status
            );

            if let Err(sync_error) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(app_state, &merchant.id, &request.environment).await {
                log::warn!("Balance sync failed after EVM withdrawal: {:?}", sync_error);
            }

            let response = serde_json::json!({
                "id": result.withdrawal_id,
                "merchant_id": merchant.id,
                "external_id": request.external_id.clone().unwrap_or_else(|| result.withdrawal_id.clone()),
                "idempotency_key": request.idempotency_key,
                "environment": result.environment,
                "network": result.network,
                "currency": result.network,
                "to_address": request.to_address,
                "amount": result.amount,
                "fee": result.fee,
                "net_debit": result.net_debit,
                "status": result.status,
                "confirmations": result.confirmations,
                "required_confirmations": result.required_confirmations,
                "tx_hashes": result.tx_hashes,
                "explorer_urls": result.explorer_urls,
                "wallets_used": result.wallets_used.iter().map(|usage| serde_json::json!({
                    "address": usage.address,
                    "amount_sent": crate::merchant::services::evm_withdrawal_service::EvmWithdrawalService::wei_to_decimal(usage.transferred_wei, 18).unwrap_or_default().to_string(),
                    "fee": crate::merchant::services::evm_withdrawal_service::EvmWithdrawalService::wei_to_decimal(usage.actual_fee_wei, 18).unwrap_or_default().to_string(),
                    "tx_hash": usage.tx_hash,
                    "explorer_url": usage.explorer_url
                })).collect::<Vec<_>>(),
                "created_at": chrono::Utc::now().to_rfc3339(),
                "updated_at": chrono::Utc::now().to_rfc3339()
            });

            Ok(HttpResponse::Ok().json(response))
        }
        Err(e) => {
            log::error!(" EVM single wallet withdrawal failed: {}", e);

            if let AppError::ValidationError(ref msg) = e {
                if msg.contains("Insufficient") && msg.contains("balance") {
                    log::info!("🔄 Insufficient balance in single wallet, attempting EVM multi-wallet aggregation fallback");
                    return MerchantAuth::try_evm_multi_wallet_withdrawal_fallback(
                        app_state,
                        &merchant.id,
                        request,
                    )
                    .await;
                }
            }

            Err(e)
        }
    }
}

async fn handle_btc_withdrawal(
    app_state: &web::Data<AppState>,
    merchant: &merchant::Model,
    request: &CreateWithdrawalRequest,
) -> Result<HttpResponse, AppError> {
    use crate::merchant::services::btc_withdrawal_service::BtcWithdrawalService;

    log::info!(
        "🟠 Executing Bitcoin withdrawal for {} BTC to {}",
        request.amount,
        request.to_address
    );

    let wallet = match get_wallet_for_network(
        &app_state.db,
        &merchant.user_id,
        "btc",
        &request.environment,
    )
    .await
    {
        Ok(wallet) => wallet,
        Err(e) => {
            log::error!("Failed to get wallet for Bitcoin withdrawal: {}", e);
            return Err(e);
        }
    };

    match BtcWithdrawalService::execute_withdrawal(&app_state.db, &merchant.id, request).await {
        Ok(result) => {
            log::info!(
                "   Bitcoin withdrawal successful: tx_hash={}",
                result.tx_hash
            );

            let withdrawal_record = withdrawal_request::ActiveModel {
                id: sea_orm::Set(result.withdrawal_id.clone()),
                merchant_id: sea_orm::Set(merchant.id.clone()),
                wallet_id: sea_orm::Set(wallet.id.clone()),
                external_id: sea_orm::Set(result.withdrawal_id.clone()),
                idempotency_key: sea_orm::Set(request.idempotency_key.clone()),
                environment: sea_orm::Set(request.environment.clone()),
                network: sea_orm::Set("btc".to_string()),
                currency: sea_orm::Set("bitcoin".to_string()),
                to_address: sea_orm::Set(request.to_address.clone()),
                amount: sea_orm::Set(Decimal::from_str(&request.amount).unwrap_or_default()),
                fee: sea_orm::Set(Some(
                    Decimal::from(result.fee_satoshis) / Decimal::from(100_000_000),
                )),
                net_amount: sea_orm::Set(Decimal::from_str(&request.amount).unwrap_or_default()),
                status: sea_orm::Set("confirmed".to_string()),
                tx_hash: sea_orm::Set(Some(result.tx_hash.clone())),
                blockchain_confirmations: sea_orm::Set(Some(1)),
                required_confirmations: sea_orm::Set(6),
                confirmed_at: sea_orm::Set(Some(chrono::Utc::now().into())),
                processed_at: sea_orm::Set(Some(chrono::Utc::now().into())),
                broadcast_at: sea_orm::Set(Some(chrono::Utc::now().into())),
                created_at: sea_orm::Set(chrono::Utc::now().into()),
                updated_at: sea_orm::Set(chrono::Utc::now().into()),
                metadata: sea_orm::Set(Some(
                    serde_json::json!({
                        "inputs_used": result.inputs_used
                    })
                    .to_string(),
                )),
                finality_required: sea_orm::Set(6),
                finality_reached_at: sea_orm::Set(None),
                replaced_by_tx: sea_orm::Set(None),
                reorg_depth: sea_orm::Set(0),
                lifecycle_state: sea_orm::Set(WithdrawalLifecycleState::Quoted.to_string()),
                withdrawal_funds_state: sea_orm::Set(WithdrawalFundsState::Unlocked.to_string()),
                ..Default::default()
            };

            withdrawal_record.insert(&app_state.db).await.map_err(|e| {
                AppError::DatabaseError(format!("Failed to save withdrawal: {}", e))
            })?;

            if let Err(sync_error) = crate::merchant::services::balance_sync_service::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(app_state, &merchant.id, &request.environment).await {
                log::warn!("Balance sync failed after Bitcoin withdrawal: {:?}", sync_error);
            }

            let response = serde_json::json!({
                "id": result.withdrawal_id,
                "merchant_id": merchant.id,
                "external_id": result.withdrawal_id,
                "idempotency_key": request.idempotency_key,
                "environment": request.environment,
                "network": "btc",
                "currency": "bitcoin",
                "to_address": result.to_address,
                "amount": request.amount,
                "fee": (result.fee_satoshis as f64 / 100_000_000.0).to_string(),
                "net_amount": request.amount,
                "status": "confirmed",
                "tx_hash": result.tx_hash,
                "blockchain_confirmations": 1,
                "required_confirmations": 6,
                "confirmations": 1,
                "explorer_url": result.explorer_url,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
                "processed_at": chrono::Utc::now().to_rfc3339(),
                "confirmed_at": chrono::Utc::now().to_rfc3339(),
                "broadcast_at": chrono::Utc::now().to_rfc3339(),
                "wallet_used": {
                    "address": result.from_address,
                },
                "metadata": {
                    "inputs_used": result.inputs_used,
                }
            });

            Ok(HttpResponse::Ok().json(response))
        }
        Err(e) => {
            log::error!(" Bitcoin single wallet withdrawal failed: {}", e);

            if let AppError::ValidationError(ref msg) = e {
                if msg.contains("Insufficient") && msg.contains("balance") {
                    log::info!("🔄 Insufficient balance in single wallet, attempting Bitcoin multi-wallet aggregation fallback");
                    return MerchantAuth::try_btc_multi_wallet_withdrawal_fallback(
                        app_state,
                        &merchant.id,
                        request,
                    )
                    .await;
                }
            }

            Err(e)
        }
    }
}

async fn get_wallet_for_network(
    db: &sea_orm::DatabaseConnection,
    user_id: &str,
    network: &str,
    environment: &str,
) -> Result<wallet::Model, AppError> {
    let currency_pattern = format!("{}_{}", network.to_lowercase(), environment);

    let wallet = wallet::Entity::find()
        .filter(wallet::Column::UserId.eq(user_id))
        .filter(wallet::Column::Currency.contains(&currency_pattern))
        .one(db)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to query wallet: {}", e)))?
        .ok_or_else(|| {
            AppError::ValidationError(format!("No {} wallet found for user", network))
        })?;

    Ok(wallet)
}

pub async fn fix_bnb_usdt_wallet_currencies(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    log::info!("🔧 Starting BNB USDT wallet currency fix...");

    let bnb_usdt_payments = payment_request::Entity::find()
        .filter(payment_request::Column::Currency.eq("USDT_BNB"))
        .filter(payment_request::Column::Status.eq("paid"))
        .filter(payment_request::Column::Environment.eq("mainnet"))
        .all(&app_state.db)
        .await?;

    log::info!(
        "Found {} BNB USDT payments to process",
        bnb_usdt_payments.len()
    );

    let mut fixed_wallets = 0;
    let mut wallet_details = Vec::new();

    for payment in &bnb_usdt_payments {
        log::info!(
            "Processing payment {} with wallet {}",
            payment.id,
            payment.wallet_address
        );

        let wallet = wallet::Entity::find()
            .filter(wallet::Column::Address.eq(&payment.wallet_address))
            .one(&app_state.db)
            .await?;

        if let Some(wallet) = wallet {
            wallet_details.push(serde_json::json!({
                "payment_id": payment.id,
                "wallet_id": wallet.id,
                "wallet_address": wallet.address,
                "current_currency": wallet.currency,
                "payment_currency": payment.currency,
                "payment_amount": payment.amount.to_string(),
                "needs_fix": wallet.currency.contains("usdt_mainnet") || wallet.currency == "usdt_mainnet"
            }));

            if wallet.currency.contains("usdt_mainnet") || wallet.currency == "usdt_mainnet" {
                log::info!(
                    "🔧 Fixing wallet {} currency from {} to usdt_bnb_mainnet",
                    wallet.id,
                    wallet.currency
                );

                let mut wallet_active: wallet::ActiveModel = wallet.into();
                wallet_active.currency = Set("usdt_bnb_mainnet".to_string());

                match wallet_active.update(&app_state.db).await {
                    Ok(_) => {
                        fixed_wallets += 1;
                        log::info!("   Fixed wallet currency");
                    }
                    Err(e) => {
                        log::error!(" Failed to fix wallet currency: {}", e);
                    }
                }
            }
        }
    }

    if fixed_wallets > 0 {
        log::info!(
            "🔄 Triggering balance sync after fixing {} wallets",
            fixed_wallets
        );

        if let Some(first_payment) = bnb_usdt_payments.first() {
            if let Err(e) = crate::merchant::services::BalanceSyncService::sync_wallet_balances_from_payments_for_environment(
                &app_state,
                &first_payment.merchant_id,
                "mainnet"
            ).await {
                log::warn!("Balance sync failed after wallet fix: {:?}", e);
            }
        }
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": format!("Fixed {} wallet currency mappings", fixed_wallets),
        "bnb_usdt_payments_found": bnb_usdt_payments.len(),
        "wallets_fixed": fixed_wallets,
        "wallet_details": wallet_details
    })))
}

pub async fn manual_payment_confirmation(
    app_state: web::Data<AppState>,
    query: web::Query<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    let payment_id = query
        .get("payment_id")
        .and_then(|v| v.as_str())
        .unwrap_or("7a667d07-3ace-496d-9b7b-bb8a542b6442");
    let tx_hash = query
        .get("tx_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("0x0ca0ebc790590ba47384c2c6eb56515b6b25931da3c9c53b94e443e43ed9f6ff");

    log::info!(
        "🔧 [MANUAL_CONFIRM] Manual payment confirmation: payment={}, tx={}",
        payment_id,
        tx_hash
    );

    let payment = payment_request::Entity::find_by_id(payment_id)
        .one(&app_state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Payment not found".to_string()))?;

    log::info!(
        "📋 [MANUAL_CONFIRM] Found payment: {} {} to {}",
        payment.amount,
        payment.currency,
        payment.wallet_address
    );

    let monitor = crate::shared::service::universal_address_monitor::UniversalAddressMonitor::new();
    let expected_amount = payment.amount.to_string().parse::<f64>().unwrap_or(0.0);

    match monitor
        .verify_usdt_transaction(tx_hash, &payment.wallet_address, expected_amount)
        .await
    {
        Ok(Some(detected_tx)) => {
            log::info!("   [MANUAL_CONFIRM] Transaction verified, proceeding with confirmation");

            let result =
                crate::shared::service::payment_monitor::PaymentMonitorService::confirm_payment(
                    &app_state, &payment, tx_hash,
                )
                .await;

            match result {
                Ok(_) => {
                    log::info!("🎉 [MANUAL_CONFIRM] Payment confirmed successfully");
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "status": "success",
                        "message": "Payment confirmed successfully",
                        "payment_id": payment_id,
                        "tx_hash": tx_hash,
                        "detected_transaction": {
                            "amount": detected_tx.amount,
                            "currency": detected_tx.currency,
                            "confirmations": detected_tx.confirmations,
                            "network": detected_tx.network
                        }
                    })))
                }
                Err(e) => {
                    log::error!(" [MANUAL_CONFIRM] Failed to confirm payment: {}", e);
                    Ok(HttpResponse::Ok().json(serde_json::json!({
                        "status": "error",
                        "message": format!("Failed to confirm payment: {}", e),
                        "payment_id": payment_id
                    })))
                }
            }
        }
        Ok(None) => {
            log::warn!(" [MANUAL_CONFIRM] Transaction verification failed");
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "status": "verification_failed",
                "message": "Transaction could not be verified",
                "payment_id": payment_id,
                "tx_hash": tx_hash
            })))
        }
        Err(e) => {
            log::error!(" [MANUAL_CONFIRM] Verification error: {}", e);
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "status": "error",
                "message": format!("Verification failed: {}", e),
                "payment_id": payment_id
            })))
        }
    }
}

pub async fn trigger_payment_monitoring(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    log::info!("🔧 [MANUAL_TRIGGER] Manual payment monitoring trigger");

    let pending_payments = payment_request::Entity::find()
        .filter(payment_request::Column::Status.eq("pending"))
        .all(&app_state.db)
        .await?;

    log::info!(
        "📋 [MANUAL_TRIGGER] Found {} pending payments",
        pending_payments.len()
    );

    let mut results = Vec::new();

    for payment in &pending_payments {
        log::info!(
            " [MANUAL_TRIGGER] Checking payment {} - {} {} to {}",
            payment.id,
            payment.amount,
            payment.currency,
            payment.wallet_address
        );

        let expected_amount = payment.amount.to_string().parse::<f64>().unwrap_or(0.0);

        let monitor =
            crate::shared::service::universal_address_monitor::UniversalAddressMonitor::new();

        let effective_currency = match payment.environment.as_str() {
            "testnet" => format!("{}_testnet", payment.currency.to_lowercase()),
            _ => payment.currency.clone(),
        };

        let since_time = payment.created_at.into();

        match monitor
            .check_payment_received(
                &payment.wallet_address,
                expected_amount,
                &effective_currency,
                since_time,
            )
            .await
        {
            Ok(Some(detected_tx)) => {
                log::info!(
                    "🎉 [MANUAL_TRIGGER] Transaction found for payment {}: {}",
                    payment.id,
                    detected_tx.hash
                );

                if let Err(e) =
                    crate::shared::service::payment_monitor::PaymentMonitorService::confirm_payment(
                        &app_state,
                        payment,
                        &detected_tx.hash,
                    )
                    .await
                {
                    log::error!(
                        " [MANUAL_TRIGGER] Failed to confirm payment {}: {}",
                        payment.id,
                        e
                    );
                    results.push(serde_json::json!({
                        "payment_id": payment.id,
                        "status": "confirmation_failed",
                        "error": format!("{}", e),
                        "detected_tx": detected_tx.hash
                    }));
                } else {
                    results.push(serde_json::json!({
                        "payment_id": payment.id,
                        "status": "confirmed",
                        "tx_hash": detected_tx.hash,
                        "amount": detected_tx.amount,
                        "currency": detected_tx.currency
                    }));
                }
            }
            Ok(None) => {
                log::info!(
                    "⏳ [MANUAL_TRIGGER] No transaction found yet for payment {}",
                    payment.id
                );
                results.push(serde_json::json!({
                    "payment_id": payment.id,
                    "status": "not_found",
                    "currency": payment.currency,
                    "amount": payment.amount.to_string(),
                    "address": payment.wallet_address
                }));
            }
            Err(e) => {
                log::error!(
                    " [MANUAL_TRIGGER] Error checking payment {}: {}",
                    payment.id,
                    e
                );
                results.push(serde_json::json!({
                    "payment_id": payment.id,
                    "status": "error",
                    "error": format!("{}", e)
                }));
            }
        }
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": format!("Checked {} pending payments", pending_payments.len()),
        "payments_checked": pending_payments.len(),
        "results": results
    })))
}

pub async fn verify_bnb_usdt_transaction(
    app_state: web::Data<AppState>,
    query: web::Query<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    let tx_hash = query
        .get("tx_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("0x0ca0ebc790590ba47384c2c6eb56515b6b25931da3c9c53b94e443e43ed9f6ff");
    let to_address = query
        .get("to_address")
        .and_then(|v| v.as_str())
        .unwrap_or("0xf93c6547f2d2f08ae46b3361a16fa0fce294ceeb");
    let expected_amount = query.get("amount").and_then(|v| v.as_f64()).unwrap_or(1.0);

    log::info!(
        " [VERIFY_BNB_USDT] Manual verification: tx={}, to={}, amount={}",
        tx_hash,
        to_address,
        expected_amount
    );

    let monitor = crate::shared::service::universal_address_monitor::UniversalAddressMonitor::new();

    match monitor
        .verify_usdt_transaction(tx_hash, to_address, expected_amount)
        .await
    {
        Ok(Some(detected_tx)) => {
            log::info!(
                "   [VERIFY_SUCCESS] Transaction verified: {:?}",
                detected_tx
            );
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "status": "success",
                "message": "Transaction verified successfully",
                "transaction": {
                    "hash": detected_tx.hash,
                    "amount": detected_tx.amount,
                    "currency": detected_tx.currency,
                    "confirmations": detected_tx.confirmations,
                    "network": detected_tx.network,
                    "from_address": detected_tx.from_address,
                    "to_address": detected_tx.to_address,
                    "block_time": detected_tx.block_time,
                    "status": detected_tx.status
                }
            })))
        }
        Ok(None) => {
            log::warn!(" [VERIFY_FAILED] Transaction not found or doesn't match criteria");
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "status": "not_found",
                "message": "Transaction not found or doesn't match expected criteria",
                "searched_tx": tx_hash,
                "expected_to": to_address,
                "expected_amount": expected_amount
            })))
        }
        Err(e) => {
            log::error!(" [VERIFY_ERROR] Verification failed: {}", e);
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "status": "error",
                "message": format!("Verification failed: {}", e),
                "searched_tx": tx_hash
            })))
        }
    }
}

pub async fn debug_usdt_bep20_wallet_discovery(
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    use crate::merchant::services::evm_multi_wallet_service::EvmMultiWalletService;

    let user_id = "44427cee-f15f-45c6-9075-af352dd5392f";
    let merchant_id = "f5a65b8b-072e-4415-8110-26d43663ca0a";
    let network = "usdt_bep20";
    let environment = "mainnet";

    log::info!(" DEBUG: Starting USDT BEP-20 wallet discovery test");
    log::info!("   User ID: {}", user_id);
    log::info!("   Merchant ID: {}", merchant_id);
    log::info!("   Network: {}", network);
    log::info!("   Environment: {}", environment);

    let deposit_wallets = EvmMultiWalletService::discover_deposit_wallets_for_breakdown(
        &app_state.db,
        merchant_id,
        network,
        environment,
    )
    .await?;

    log::info!(" Found {} deposit wallets", deposit_wallets.len());
    for wallet in &deposit_wallets {
        log::info!(
            "   - Wallet {}: {} (currency: {})",
            wallet.id,
            wallet.address,
            wallet.currency
        );
    }

    let candidates = EvmMultiWalletService::evaluate_candidates_for_breakdown(
        deposit_wallets.clone(),
        network,
        environment,
    )
    .await?;

    log::info!(
        " Found {} candidates with transferable balance",
        candidates.len()
    );
    for candidate in &candidates {
        log::info!("   - Candidate {}: native {} wei, token {} wei, transferable {} wei, needs gas topup: {}",
                  candidate.address,
                  candidate.native_balance_wei,
                  candidate.token_balance_wei,
                  candidate.transferable_amount,
                  candidate.needs_gas_topup);
    }

    let debug_info = serde_json::json!({
        "success": true,
        "wallets_found": deposit_wallets.len(),
        "candidates_found": candidates.len(),
        "wallets": deposit_wallets.iter().map(|w| serde_json::json!({
            "id": w.id,
            "address": w.address,
            "currency": w.currency
        })).collect::<Vec<_>>(),
        "candidates": candidates.iter().map(|c| serde_json::json!({
            "address": c.address,
            "native_balance_wei": c.native_balance_wei.to_string(),
            "token_balance_wei": c.token_balance_wei.to_string(),
            "transferable_amount": c.transferable_amount.to_string(),
            "needs_gas_topup": c.needs_gas_topup,
            "gas_topup_amount": c.gas_topup_amount.to_string()
        })).collect::<Vec<_>>()
    });

    Ok(HttpResponse::Ok().json(debug_info))
}
