use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Result};
use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::str::FromStr;
use uuid::Uuid;

use crate::{
    merchant::models::payment::{CreatePaymentRequest, PaymentResponse},
    merchant::services::WalletGenerationService,
    shared::models::temp_auth::UserClaims,
    shared::{
        entities::{api_key, merchant, payment_request, prelude::*, wallet},
        service::{
            universal_address_monitor::UniversalAddressMonitor,
            wallet::{Currency, WalletService as CoreWalletService},
        },
        utils::{
            api_key::generate_api_key_pair, constants, encryption::CryptoEncryption,
            errors::AppError, payment_uri::PaymentUriGenerator,
        },
        AppState,
    },
};

// Testnet-specific currencies (Base Sepolia ETH, Bitcoin, and USDT)
const TESTNET_CURRENCIES: &[&str] = &["eth", "bitcoin", "btc", "usdt", "usdt_bnb", "usdt_bep20"];

// Create test API key for merchant
pub async fn create_test_api_key(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    // Extract user from JWT
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let merchant_id = path.into_inner();
    println!("Authenticated user ID: {}", claims.id);
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Testnet API Key");

    // Find merchant by ID and verify ownership
    println!("Looking for merchant with ID: {}", merchant_id);
    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Merchant not found with ID: {}",
            merchant_id
        )))?;

    // Verify the merchant belongs to the authenticated user
    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden(
            "Access denied to this merchant".to_string(),
        ));
    }

    // Generate test API key with special prefix
    let mut key_pair = generate_api_key_pair()?;

    // Modify the API key to have test prefix
    key_pair.api_key = key_pair.api_key.replace("ak_", "test_ak_");

    // Encrypt keys
    let crypto = CryptoEncryption::new()?;
    let encrypted_api_key = crypto.encrypt(&key_pair.api_key)?;
    let encrypted_secret_key = crypto.encrypt(&key_pair.secret_key)?;

    // Create API key record
    let api_key_id = Uuid::new_v4().to_string();
    let new_api_key = api_key::ActiveModel {
        id: Set(api_key_id.clone()),
        merchant_id: Set(merchant.id.clone()),
        name: Set(format!("[TESTNET] {}", name)),
        key_hash: Set(key_pair.key_hash),
        secret_hash: Set(key_pair.secret_hash),
        key_prefix: Set(format!("test_{}", &key_pair.key_prefix)),
        encrypted_api_key: Set(Some(encrypted_api_key)),
        encrypted_secret_key: Set(Some(encrypted_secret_key)),
        api_key: Set(Some(key_pair.api_key.clone())),
        secret_key: Set(Some(key_pair.secret_key.clone())),
        permissions: Set("[]".to_string()), // Empty permissions array for testnet
        environment_type: Set("testnet".to_string()),
        expires_at: Set(None),
        is_active: Set(true),
        last_used: Set(None),
        rate_limit_per_minute: Set(None),
        rate_limit_per_hour: Set(None),
        last_revoked_at: Set(None),
        created_at: Set(Utc::now().into()),
    };

    // Add detailed error logging for database insert
    println!("Attempting to insert API key with data:");
    println!("  id: {}", &api_key_id);
    println!("  merchant_id: {}", &merchant.id);
    println!("  name: {}", &format!("[TESTNET] {}", name));
    println!(
        "  key_prefix: {}",
        &format!("test_{}", &key_pair.key_prefix)
    );
    println!("  environment_type: testnet");

    let saved_api_key = match new_api_key.insert(&app_state.db).await {
        Ok(key) => {
            println!("Successfully inserted API key");
            key
        }
        Err(e) => {
            eprintln!("Database error during API key insert: {:?}", e);
            eprintln!("Error type: {}", std::any::type_name_of_val(&e));
            return Err(AppError::InternalServerError(format!(
                "Database error: {}",
                e
            )));
        }
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": {
            "id": saved_api_key.id,
            "name": saved_api_key.name,
            "api_key": key_pair.api_key,
            "secret_key": key_pair.secret_key,
            "environment": "testnet",
            "is_active": saved_api_key.is_active,
            "created_at": saved_api_key.created_at.to_rfc3339(),
            "message": "This is a TESTNET API key. It can only be used with testnet endpoints and Base Sepolia network."
        }
    })))
}

// Create testnet payment request
pub async fn create_testnet_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<CreatePaymentRequest>,
) -> Result<HttpResponse, AppError> {
    // Extract API key from header
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    // Verify it's a test API key
    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet endpoints".to_string(),
        ));
    }

    // Extract key prefix (skip 'test_ak_' prefix)
    let key_prefix = if api_key.len() >= 16 {
        &api_key[8..16] // Skip "test_ak_" and take next 8 chars
    } else {
        return Err(AppError::Unauthorized(
            "Invalid test API key format".to_string(),
        ));
    };

    // Find API key by prefix
    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(format!("test_{}", key_prefix)))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?
        .ok_or(AppError::Unauthorized("Invalid test API key".to_string()))?;

    // Verify environment is testnet
    if api_key_record.environment_type != "testnet" {
        return Err(AppError::ValidationError(
            "This API key is not configured for testnet".to_string(),
        ));
    }

    // Get merchant
    let merchant = Merchant::find_by_id(&api_key_record.merchant_id)
        .one(&app_state.db)
        .await?
        .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

    // Validate currency is supported on testnet
    let currency = body.currency.to_lowercase();
    if !TESTNET_CURRENCIES.contains(&currency.as_str()) {
        return Err(AppError::ValidationError(format!(
            "Currency '{}' is not supported on testnet. Supported currencies: {:?}",
            currency, TESTNET_CURRENCIES
        )));
    }

    // Parse amount
    let amount = Decimal::from_str(&body.amount)
        .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

    // Generate payment ID first (needed for unique wallet generation)
    let payment_id = Uuid::new_v4().to_string();

    // Generate unique testnet wallet address for this payment
    let wallet_address =
        generate_unique_testnet_wallet(&app_state.db, &merchant.user_id, &currency, &payment_id)
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to generate testnet wallet: {}", e))
            })?;

    // Create payment request
    let expires_at = Utc::now() + chrono::Duration::seconds(body.expires_in_seconds as i64);
    let payment_url = format!(
        "{}/testnet/{}",
        constants::PAYMENT_REQUEST_DOMAIN.as_str(),
        payment_id
    );

    let payment_request = payment_request::ActiveModel {
        id: Set(payment_id.clone()),
        merchant_id: Set(merchant.id.clone()),
        external_id: Set(Some(body.external_id.clone())),
        amount: Set(amount),
        currency: Set(currency.clone()),
        wallet_address: Set(wallet_address.clone()),
        status: Set("pending".to_string()),
        metadata: Set(Some(
            serde_json::to_string(&body.metadata).unwrap_or_default(),
        )),
        payment_url: Set(Some(payment_url.clone())),
        expires_at: Set(expires_at.into()),
        paid_at: Set(None),
        created_at: Set(Utc::now().into()),
        updated_at: Set(Some(Utc::now().into())),
        environment: Set("testnet".to_string()), // Mark as testnet payment
    };

    let saved_payment = payment_request.insert(&app_state.db).await?;

    // Generate QR metadata with testnet environment
    let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
        &saved_payment.currency,
        &saved_payment.wallet_address,
        &saved_payment.amount.to_string(),
        &saved_payment.id,
        &body.external_id,
        Some("testnet"),
    );

    let response = PaymentResponse {
        id: saved_payment.id,
        merchant_id: saved_payment.merchant_id,
        external_id: saved_payment.external_id.unwrap_or_default(),
        amount: saved_payment.amount.to_string(),
        currency: saved_payment.currency.to_uppercase(),
        wallet_address: saved_payment.wallet_address,
        status: saved_payment.status,
        metadata: serde_json::from_str(&saved_payment.metadata.clone().unwrap_or_default())
            .unwrap_or_default(),
        payment_url: saved_payment.payment_url.unwrap_or_default(),
        expires_at: saved_payment.expires_at.into(),
        paid_at: saved_payment.paid_at.map(|dt| dt.into()),
        created_at: saved_payment.created_at.into(),
        updated_at: saved_payment
            .updated_at
            .map(|dt| dt.into())
            .unwrap_or_else(|| Utc::now()),
        qr_data: qr_metadata,
        usd_price: None,
        usd_value: None,
        price_snapshot_ts: None,
        is_simulated_price: Some(true),
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "environment": "testnet",
        "network": "Base Sepolia",
        "chain_id": 84532,
        "data": response
    })))
}

// Get testnet payment status
pub async fn get_testnet_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let payment_id = path.into_inner();

    // Verify test API key
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    if !api_key.starts_with("test_ak_") {
        return Err(AppError::ValidationError(
            "Only test API keys can be used with testnet endpoints".to_string(),
        ));
    }

    // Get payment
    let payment = PaymentRequest::find_by_id(&payment_id)
        .one(&app_state.db)
        .await?
        .ok_or(AppError::NotFound("Payment not found".to_string()))?;

    // Generate QR metadata with testnet environment
    let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
        &payment.currency,
        &payment.wallet_address,
        &payment.amount.to_string(),
        &payment.id,
        &payment.external_id.as_ref().unwrap_or(&String::new()),
        Some("testnet"),
    );

    let response = PaymentResponse {
        id: payment.id,
        merchant_id: payment.merchant_id,
        external_id: payment.external_id.unwrap_or_default(),
        amount: payment.amount.to_string(),
        currency: payment.currency.to_uppercase(),
        wallet_address: payment.wallet_address,
        status: payment.status,
        metadata: serde_json::from_str(&payment.metadata.clone().unwrap_or_default())
            .unwrap_or_default(),
        payment_url: payment.payment_url.unwrap_or_default(),
        expires_at: payment.expires_at.into(),
        paid_at: payment.paid_at.map(|dt| dt.into()),
        created_at: payment.created_at.into(),
        updated_at: payment
            .updated_at
            .map(|dt| dt.into())
            .unwrap_or_else(|| Utc::now()),
        qr_data: qr_metadata,
        usd_price: None,
        usd_value: None,
        price_snapshot_ts: None,
        is_simulated_price: Some(true),
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "environment": "testnet",
        "data": response
    })))
}

// List testnet API keys for merchant
pub async fn list_test_api_keys(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    let merchant_id =
        query
            .get("merchant_id")
            .and_then(|v| v.as_str())
            .ok_or(AppError::ValidationError(
                "merchant_id required".to_string(),
            ))?;

    // Get all testnet API keys for merchant
    let api_keys = ApiKey::find()
        .filter(api_key::Column::MerchantId.eq(merchant_id))
        .filter(api_key::Column::EnvironmentType.eq("testnet"))
        .filter(api_key::Column::IsActive.eq(true))
        .all(&app_state.db)
        .await?;

    let crypto = CryptoEncryption::new()?;

    let keys_response: Vec<serde_json::Value> = api_keys
        .into_iter()
        .map(|key| {
            // Decrypt API key and secret key if available
            let decrypted_api_key = key
                .encrypted_api_key
                .as_ref()
                .and_then(|enc| crypto.decrypt(enc).ok());

            let decrypted_secret_key = key
                .encrypted_secret_key
                .as_ref()
                .and_then(|enc| crypto.decrypt(enc).ok());

            serde_json::json!({
                "id": key.id,
                "name": key.name,
                "key_prefix": key.key_prefix,
                "api_key": decrypted_api_key,
                "secret_key": decrypted_secret_key,
                "environment_type": "testnet",
                "is_active": key.is_active,
                "created_at": key.created_at.to_rfc3339(),
                "last_used": key.last_used.map(|dt| dt.to_rfc3339()),
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": keys_response
    })))
}

// Get testnet configuration info
pub async fn get_testnet_info(_app_state: web::Data<AppState>) -> Result<HttpResponse, AppError> {
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": {
            "network": "Base Sepolia",
            "chain_id": 84532,
            "rpc_urls": [
                "https://sepolia.base.org",
                "https://base-sepolia.public.blastapi.io",
                "https://base-sepolia.blockpi.network/v1/rpc/public"
            ],
            "supported_currencies": TESTNET_CURRENCIES,
            "faucets": [
                "https://www.alchemy.com/faucets/base-sepolia",
                "https://faucet.quicknode.com/base/sepolia"
            ],
            "explorer": "https://sepolia.basescan.org",
            "message": "This is a testnet environment. No real money is involved."
        }
    })))
}

// Public endpoint for testnet payment page access (no API key required)
pub async fn get_public_testnet_payment(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let payment_id = path.into_inner();

    // Find testnet payment request
    let mut payment = PaymentRequest::find_by_id(&payment_id)
        .one(&app_state.db)
        .await?
        .ok_or(AppError::NotFound("Payment request not found".to_string()))?;

    // For testnet payments, actively check for payment status if still pending
    if payment.status == "pending" && payment.expires_at > Utc::now() {
        if let Ok(updated_payment) = check_and_update_testnet_payment(&app_state, &payment).await {
            payment = updated_payment;
        }
    }

    // Generate QR metadata with testnet environment
    let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
        &payment.currency,
        &payment.wallet_address,
        &payment.amount.to_string(),
        &payment.id,
        &payment.external_id.clone().unwrap_or_default(),
        Some("testnet"), // Force testnet environment
    );

    let response = PaymentResponse {
        id: payment.id,
        merchant_id: payment.merchant_id,
        external_id: payment.external_id.unwrap_or_default(),
        amount: payment.amount.to_string(),
        currency: payment.currency.to_uppercase(),
        wallet_address: payment.wallet_address,
        status: payment.status,
        metadata: serde_json::from_str(&payment.metadata.clone().unwrap_or_default())
            .unwrap_or_default(),
        payment_url: payment.payment_url.unwrap_or_default(),
        expires_at: payment.expires_at.into(),
        paid_at: payment.paid_at.map(|dt| dt.into()),
        created_at: payment.created_at.into(),
        updated_at: payment
            .updated_at
            .map(|dt| dt.into())
            .unwrap_or_else(|| Utc::now()),
        qr_data: qr_metadata,
        usd_price: None,
        usd_value: None,
        price_snapshot_ts: None,
        is_simulated_price: Some(true),
    };

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "environment": "testnet",
        "data": response
    })))
}

/// Check and update testnet payment status by actively monitoring the address
async fn check_and_update_testnet_payment(
    app_state: &AppState,
    payment: &payment_request::Model,
) -> Result<payment_request::Model, AppError> {
    log::info!(
        " [TESTNET] Actively checking payment status for {}",
        payment.id
    );

    // Create universal address monitor
    let monitor = UniversalAddressMonitor::new();
    let expected_amount = payment.amount.to_string().parse::<f64>().unwrap_or(0.0);

    // For testnet ETH, check base_sepolia network
    let effective_currency = "base_sepolia";

    log::info!(
        " [TESTNET] Checking {} address {} for payment of {} ETH",
        effective_currency.to_uppercase(),
        payment.wallet_address,
        expected_amount
    );

    match monitor
        .check_payment_received(
            &payment.wallet_address,
            expected_amount,
            effective_currency,
            payment.created_at.into(),
        )
        .await
    {
        Ok(Some(tx)) => {
            log::info!("🎉 [TESTNET] Payment detected! TX: {}", tx.hash);

            // Update payment status to paid
            let mut active_payment: payment_request::ActiveModel = payment.clone().into();
            active_payment.status = Set("paid".to_string());
            active_payment.paid_at = Set(Some(Utc::now().into()));
            active_payment.updated_at = Set(Some(Utc::now().into()));

            // Update metadata with transaction details
            let mut metadata = if let Some(metadata_str) = &payment.metadata {
                serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
            } else {
                serde_json::json!({})
            };

            metadata["transaction_hash"] = serde_json::Value::String(tx.hash.clone());
            metadata["confirmed_at"] = serde_json::Value::String(Utc::now().to_rfc3339());
            metadata["amount_received"] = serde_json::Value::Number(
                serde_json::Number::from_f64(tx.amount).unwrap_or(serde_json::Number::from(0)),
            );
            metadata["network"] = serde_json::Value::String(tx.network.clone());
            metadata["confirmations"] =
                serde_json::Value::Number(serde_json::Number::from(tx.confirmations));

            active_payment.metadata =
                Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

            match active_payment.update(&app_state.db).await {
                Ok(updated_payment) => {
                    log::info!("✅ [TESTNET] Payment {} updated to paid status", payment.id);
                    Ok(updated_payment)
                }
                Err(e) => {
                    log::error!(" [TESTNET] Failed to update payment {}: {}", payment.id, e);
                    Ok(payment.clone()) // Return original payment on update failure
                }
            }
        }
        Ok(None) => {
            log::info!("⏳ [TESTNET] No payment detected yet for {}", payment.id);
            Ok(payment.clone())
        }
        Err(e) => {
            log::error!(" [TESTNET] Error checking payment {}: {}", payment.id, e);
            Ok(payment.clone()) // Return original payment on error
        }
    }
}

/// Generate a unique wallet address for each testnet payment
/// This ensures every payment has its own dedicated wallet address
async fn generate_unique_testnet_wallet(
    db: &sea_orm::DatabaseConnection,
    user_id: &str,
    currency_str: &str,
    payment_id: &str,
) -> Result<String, AppError> {
    // For testnet, all currencies map to base_sepolia
    let effective_currency = "base_sepolia";

    // Create a unique currency key that includes payment_id to ensure uniqueness
    let unique_currency_key = format!("{}_testnet_{}", effective_currency, payment_id);

    // Base Sepolia uses Ethereum-compatible addresses
    let currency = Currency::from_str("ethereum")?;

    // Use the core wallet service to generate a new wallet with unique key
    let wallet_service = CoreWalletService::new();
    let (address, _mnemonic) = wallet_service
        .generate_wallet_with_key(user_id, currency, &unique_currency_key, db)
        .await?;

    log::info!(
        "Generated unique testnet {} wallet for payment {}: {}",
        effective_currency,
        payment_id,
        address
    );

    Ok(address)
}
