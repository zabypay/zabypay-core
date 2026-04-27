use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Result};
use crate::shared::utils::constants;
use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::str::FromStr;
use validator::Validate;

use crate::{
    merchant::{
        models::payment::{
            ConfirmPaymentRequest, CreatePaymentRequest, PaginationInfo, PaymentListQuery,
            PaymentListResponse, PaymentResponse, UpdatePaymentStatusRequest,
        },
        services::WalletGenerationService,
    },
    shared::models::temp_auth::UserClaims,
    shared::{
        entities::{api_key, payment_request, prelude::*, wallet},
        utils::{
            errors::AppError,
            payment_uri::{PaymentUriGenerator, QrMetadata},
        },
        AppState,
    },
};

fn payment_to_response(payment: &payment_request::Model) -> PaymentResponse {
    let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
        &payment.currency,
        &payment.wallet_address,
        &payment.amount.to_string(),
        &payment.id,
        &payment.external_id.as_ref().unwrap_or(&String::new()),
        Some(&payment.environment), 
    );

    use crate::shared::models::usd_pricing::PaymentMetadata;
    let extracted_usd_pricing = PaymentMetadata::extract_usd_pricing(payment.metadata.clone());

    PaymentResponse {
        id: payment.id.clone(),
        merchant_id: payment.merchant_id.clone(),
        external_id: payment.external_id.clone().unwrap_or_default(),
        amount: payment.amount.to_string(),
        currency: payment.currency.to_uppercase(),
        wallet_address: payment.wallet_address.clone(),
        status: payment.status.clone(),
        metadata: serde_json::from_str(&payment.metadata.clone().unwrap_or_default())
            .unwrap_or_default(),
        payment_url: payment.payment_url.clone().unwrap_or_default(),
        expires_at: payment.expires_at.into(),
        paid_at: payment.paid_at.map(|dt| dt.into()),
        created_at: payment.created_at.into(),
        updated_at: payment
            .updated_at
            .map(|dt| dt.into())
            .unwrap_or_else(|| Utc::now()),
        qr_data: qr_metadata,
        usd_price: extracted_usd_pricing.usd_price,
        usd_value: extracted_usd_pricing.usd_value,
        price_snapshot_ts: extracted_usd_pricing.price_snapshot_ts,
        is_simulated_price: extracted_usd_pricing.is_simulated,
    }
}

pub async fn create_payment_request(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<CreatePaymentRequest>,
) -> Result<HttpResponse, AppError> {
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let key_prefix = if api_key.starts_with("ak_") && api_key.len() >= 11 {
        &api_key[3..11]
    } else {
        return Err(AppError::Unauthorized("Invalid API key format".to_string()));
    };

    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(key_prefix))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?;

    let api_key_record = match api_key_record {
        Some(key) => key,
        None => return Err(AppError::Unauthorized("Invalid API key".to_string())),
    };

    let merchant = Merchant::find_by_id(&api_key_record.merchant_id)
        .one(&app_state.db)
        .await?;

    let merchant = match merchant {
        Some(m) => m,
        None => return Err(AppError::NotFound("Merchant not found".to_string())),
    };

    let expires_at = Utc::now() + chrono::Duration::seconds(body.expires_in_seconds as i64);


    let merchant_environment = crate::merchant::models::merchant::MerchantEnvironmentType::from(
        api_key_record.environment_type.clone(),
    );
    if !WalletGenerationService::is_currency_supported(
        &body.currency,
        Some(merchant_environment.clone()),
    ) {
        return Err(AppError::ValidationError(format!(
            "Unsupported currency: {}. Supported currencies: {:?}",
            body.currency,
            WalletGenerationService::get_supported_currencies(Some(merchant_environment.clone()))
        )));
    }

    let amount = Decimal::from_str(&body.amount)
        .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

    let payment_id = uuid::Uuid::new_v4().to_string();

    let wallet_address = generate_unique_mainnet_payment_wallet(
        &app_state.db,
        &merchant.user_id,
        &body.currency,
        &payment_id,
        Some(merchant_environment.clone()),
    )
    .await
    .map_err(|e| {
        log::error!(
            "Failed to generate unique wallet for payment {}: {:?}",
            payment_id,
            e
        );
        AppError::InternalServerError(format!("Failed to prepare wallet for payment: {}", e))
    })?;

    let payment_url = constants::PAYMENT_REQUEST_DOMAIN.as_str();
    let payment_request = payment_request::ActiveModel {
        id: sea_orm::Set(payment_id.clone()),
        merchant_id: sea_orm::Set(merchant.id.clone()),
        external_id: sea_orm::Set(Some(body.external_id.clone())),
        amount: sea_orm::Set(amount),
        currency: sea_orm::Set(body.currency.to_lowercase()),
        wallet_address: sea_orm::Set(wallet_address),
        status: sea_orm::Set("pending".to_string()),
        metadata: sea_orm::Set(Some(
            serde_json::to_string(&body.metadata).unwrap_or_default(),
        )),
        payment_url: sea_orm::Set(Some(format!("{}/{}", payment_url, payment_id))),
        expires_at: sea_orm::Set(expires_at.into()),
        paid_at: sea_orm::Set(None),
        created_at: sea_orm::Set(Utc::now().into()),
        updated_at: sea_orm::Set(Some(Utc::now().into())),
        environment: sea_orm::Set(String::from(merchant_environment.clone())),
    };

    let saved_payment = payment_request.insert(&app_state.db).await?;

    let response = payment_to_response(&saved_payment);

    Ok(HttpResponse::Ok().json(response))
}

pub async fn get_payment_request(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let key_prefix = if api_key.starts_with("ak_") && api_key.len() >= 11 {
        &api_key[3..11]
    } else {
        return Err(AppError::Unauthorized("Invalid API key format".to_string()));
    };

    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(key_prefix))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?;

    let api_key_record = match api_key_record {
        Some(key) => key,
        None => return Err(AppError::Unauthorized("Invalid API key".to_string())),
    };

    let payment_id = path.into_inner();

    let payment = PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&payment_id))
        .filter(payment_request::Column::MerchantId.eq(&api_key_record.merchant_id))
        .one(&app_state.db)
        .await?;

    match payment {
        Some(p) => {
            let response = payment_to_response(&p);
            Ok(HttpResponse::Ok().json(response))
        }
        None => Err(AppError::NotFound("Payment request not found".to_string())),
    }
}

pub async fn get_public_payment_request(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let payment_id = path.into_inner();

    let payment = PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&payment_id))
        .one(&app_state.db)
        .await?;

    match payment {
        Some(mut p) => {
            log::info!(
                " [MAINNET] Public request for payment {}: current status = {}",
                payment_id,
                p.status
            );

            if p.status == "pending"
                && p.expires_at.with_timezone(&chrono::Utc) > chrono::Utc::now()
            {
                let time_until_expiry =
                    (p.expires_at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
                log::info!(" [MAINNET] POLLING START: payment {} - Status: {}, Environment: '{}', Currency: {}, Amount: {}, Expires in: {}s", 
                    payment_id, p.status, p.environment, p.currency.to_uppercase(), p.amount, time_until_expiry);

                match check_and_update_mainnet_payment(&app_state, &p).await {
                    Ok(updated_payment) => {
                        if updated_payment.status != p.status {
                            log::info!("✅ [MAINNET] POLLING STOP: payment {} - REASON: status_changed ({} -> {})", 
                                payment_id, p.status, updated_payment.status);
                        } else {
                            log::info!("⏳ [MAINNET] POLLING CONTINUE: payment {} - REASON: no_change (status: {})", 
                                payment_id, p.status);
                        }
                        p = updated_payment;
                    }
                    Err(e) => {
                        log::error!(
                            " [MAINNET] POLLING ERROR: payment {} - REASON: check_failed ({})",
                            payment_id,
                            e
                        );
                    }
                }
            } else if p.status == "pending"
                && p.expires_at.with_timezone(&chrono::Utc) <= chrono::Utc::now()
            {
                log::info!(
                    " [MAINNET] POLLING STOP: payment {} - REASON: expired (expired at: {})",
                    payment_id,
                    p.expires_at
                );
            } else if p.status != "pending" {
                log::info!(
                    "✅ [MAINNET] POLLING STOP: payment {} - REASON: final_status ({})",
                    payment_id,
                    p.status
                );
            }

            if p.status == "paid" || p.status == "pending" {
                if let Ok(fresh_connection) = crate::shared::db::get_fresh_db_connection().await {
                    if let Ok(Some(fresh_payment)) = PaymentRequest::find()
                        .filter(payment_request::Column::Id.eq(&payment_id))
                        .one(&fresh_connection)
                        .await
                    {
                        if fresh_payment.status != p.status {
                            log::info!("🔄 Fresh connection shows different status for payment {}: {} -> {}", 
                                payment_id, p.status, fresh_payment.status);
                            let response = payment_to_response(&fresh_payment);
                            return Ok(HttpResponse::Ok().json(response));
                        }
                    }
                }
            }

            let response = payment_to_response(&p);
            Ok(HttpResponse::Ok().json(response))
        }
        None => Err(AppError::NotFound("Payment not found".to_string())),
    }
}

pub async fn list_payment_requests(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<PaymentListQuery>,
) -> Result<HttpResponse, AppError> {
    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let key_prefix = if api_key.starts_with("ak_") && api_key.len() >= 11 {
        &api_key[3..11]
    } else {
        return Err(AppError::Unauthorized("Invalid API key format".to_string()));
    };

    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(key_prefix))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?;

    let api_key_record = match api_key_record {
        Some(key) => key,
        None => return Err(AppError::Unauthorized("Invalid API key".to_string())),
    };

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let mut payment_query = PaymentRequest::find()
        .filter(payment_request::Column::MerchantId.eq(&api_key_record.merchant_id));

    if let Some(status) = &query.status {
        payment_query = payment_query.filter(payment_request::Column::Status.eq(status));
    }
    if let Some(currency) = &query.currency {
        payment_query =
            payment_query.filter(payment_request::Column::Currency.eq(currency.to_lowercase()));
    }
    if let Some(from_date) = query.from_date {
        payment_query = payment_query.filter(payment_request::Column::CreatedAt.gte(from_date));
    }
    if let Some(to_date) = query.to_date {
        payment_query = payment_query.filter(payment_request::Column::CreatedAt.lte(to_date));
    }
    let environment = query.environment.as_deref().unwrap_or("mainnet");
    payment_query =
        payment_query.filter(payment_request::Column::Environment.eq(environment.to_lowercase()));

    let total = payment_query.clone().count(&app_state.db).await?;

    let payments = payment_query
        .offset(offset)
        .limit(limit)
        .all(&app_state.db)
        .await?;

    let payment_responses: Vec<PaymentResponse> = payments
        .into_iter()
        .map(|p| payment_to_response(&p))
        .collect();

    let total_pages = (total + limit - 1) / limit;

    let response = PaymentListResponse {
        payments: payment_responses,
        pagination: PaginationInfo {
            page,
            limit,
            total,
            total_pages,
        },
        total_volume_usd: None, 
        volume_breakdown: None,
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn update_payment_status(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<UpdatePaymentStatusRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let key_prefix = if api_key.starts_with("ak_") && api_key.len() >= 11 {
        &api_key[3..11]
    } else {
        return Err(AppError::Unauthorized("Invalid API key format".to_string()));
    };

    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(key_prefix))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?;

    let api_key_record = match api_key_record {
        Some(key) => key,
        None => return Err(AppError::Unauthorized("Invalid API key".to_string())),
    };

    let payment_id = path.into_inner();

    let payment = PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&payment_id))
        .filter(payment_request::Column::MerchantId.eq(&api_key_record.merchant_id))
        .one(&app_state.db)
        .await?;

    let payment = match payment {
        Some(p) => p,
        None => return Err(AppError::NotFound("Payment request not found".to_string())),
    };

    let valid_statuses = vec!["pending", "paid", "expired", "failed"];
    if !valid_statuses.contains(&body.status.as_str()) {
        return Err(AppError::ValidationError(
            "Invalid status. Must be: pending, paid, expired, failed".to_string(),
        ));
    }

    let mut active_payment: payment_request::ActiveModel = payment.into();
    active_payment.status = sea_orm::Set(body.status.clone());
    active_payment.updated_at = sea_orm::Set(Some(Utc::now().into()));

    if body.status == "paid" {
        active_payment.paid_at = sea_orm::Set(Some(Utc::now().into()));
    }

    let updated_payment = active_payment.update(&app_state.db).await?;

    let response = payment_to_response(&updated_payment);

    Ok(HttpResponse::Ok().json(response))
}

pub async fn confirm_payment(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<ConfirmPaymentRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let api_key = req
        .headers()
        .get("X-API-KEY")
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let key_prefix = if api_key.starts_with("ak_") && api_key.len() >= 11 {
        &api_key[3..11]
    } else {
        return Err(AppError::Unauthorized("Invalid API key format".to_string()));
    };

    let api_key_record = ApiKey::find()
        .filter(api_key::Column::KeyPrefix.eq(key_prefix))
        .filter(api_key::Column::IsActive.eq(true))
        .one(&app_state.db)
        .await?;

    let api_key_record = match api_key_record {
        Some(key) => key,
        None => return Err(AppError::Unauthorized("Invalid API key".to_string())),
    };

    let payment_id = path.into_inner();

    let payment = PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&payment_id))
        .filter(payment_request::Column::MerchantId.eq(&api_key_record.merchant_id))
        .one(&app_state.db)
        .await?;

    let payment = match payment {
        Some(p) => p,
        None => return Err(AppError::NotFound("Payment request not found".to_string())),
    };

    if payment.status != "pending" {
        return Err(AppError::ValidationError(
            "Only pending payments can be confirmed".to_string(),
        ));
    }

    let mut active_payment: payment_request::ActiveModel = payment.into();
    active_payment.status = sea_orm::Set("paid".to_string());
    active_payment.paid_at = sea_orm::Set(Some(Utc::now().into()));
    active_payment.updated_at = sea_orm::Set(Some(Utc::now().into()));

    let updated_payment = active_payment.update(&app_state.db).await?;

    let response = payment_to_response(&updated_payment);

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Payment confirmed successfully",
        "transaction_hash": body.transaction_hash,
        "confirmations": body.confirmations,
        "payment": response
    })))
}

async fn check_and_update_mainnet_payment(
    app_state: &AppState,
    payment: &payment_request::Model,
) -> Result<payment_request::Model, AppError> {
    log::info!(
        " [MAINNET] Actively checking payment status for {}",
        payment.id
    );

    let monitor = crate::shared::service::universal_address_monitor::UniversalAddressMonitor::new();
    let expected_amount = payment.amount.to_string().parse::<f64>().unwrap_or(0.0);

    let effective_currency = match payment.environment.as_str() {
        "testnet" => "base_sepolia", 
        _ => {
            match payment.currency.to_lowercase().as_str() {
                "eth" | "ethereum" => "ethereum",
                "bnb" => "bnb",
                "btc" | "bitcoin" => "bitcoin",
                "sol" | "solana" => "solana",
                "usdt" | "usdc" => "ethereum", 
                _ => &payment.currency,
            }
        }
    };

    log::info!(
        " [MAINNET] MONITOR SCAN: scanning {} address {} for payment of {} {} (effective: {})",
        payment.currency.to_uppercase(),
        payment.wallet_address,
        expected_amount,
        payment.currency.to_uppercase(),
        effective_currency.to_uppercase()
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
            log::info!("🎉 [MAINNET] TRANSACTION FOUND: {} - Hash: {}, Network: {}, Amount: {} {}, Status: {}, Confirmations: {}", 
                payment.id, tx.hash, tx.network, tx.amount, tx.currency, tx.status, tx.confirmations);

            let mut active_payment: payment_request::ActiveModel = payment.clone().into();
            active_payment.status = sea_orm::Set("paid".to_string());
            active_payment.paid_at = sea_orm::Set(Some(chrono::Utc::now().into()));
            active_payment.updated_at = sea_orm::Set(Some(chrono::Utc::now().into()));

            let mut metadata = if let Some(metadata_str) = &payment.metadata {
                serde_json::from_str::<serde_json::Value>(metadata_str).unwrap_or_default()
            } else {
                serde_json::json!({})
            };

            metadata["transaction_hash"] = serde_json::Value::String(tx.hash.clone());
            metadata["confirmed_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            metadata["amount_received"] = serde_json::Value::Number(
                serde_json::Number::from_f64(tx.amount).unwrap_or(serde_json::Number::from(0)),
            );
            metadata["network"] = serde_json::Value::String(tx.network.clone());
            metadata["confirmations"] =
                serde_json::Value::Number(serde_json::Number::from(tx.confirmations));

            active_payment.metadata =
                sea_orm::Set(Some(serde_json::to_string(&metadata).unwrap_or_default()));

            match active_payment.update(&app_state.db).await {
                Ok(updated_payment) => {
                    log::info!("✅ [MAINNET] Payment {} updated to paid status", payment.id);
                    Ok(updated_payment)
                }
                Err(e) => {
                    log::error!(
                        " [MAINNET] Failed to update payment {}: {}",
                        payment.id,
                        e
                    );
                    Ok(payment.clone()) 
                }
            }
        }
        Ok(None) => {
            log::info!("⏳ [MAINNET] NO TRANSACTION: payment {} - address {} scanned, no matching transactions found", 
                payment.id, payment.wallet_address);
            Ok(payment.clone())
        }
        Err(e) => {
            log::error!(" [MAINNET] Error checking payment {}: {}", payment.id, e);
            Ok(payment.clone()) 
        }
    }
}

async fn generate_unique_mainnet_payment_wallet(
    db: &sea_orm::DatabaseConnection,
    user_id: &str,
    currency_str: &str,
    payment_id: &str,
    environment: Option<crate::merchant::models::merchant::MerchantEnvironmentType>,
) -> Result<String, AppError> {
    let env_type = environment.unwrap_or_default();

    let effective_currency = match env_type {
        crate::merchant::models::merchant::MerchantEnvironmentType::Testnet => {
            match currency_str.to_lowercase().as_str() {
                "ethereum" | "eth" | "usdt" | "usdt_bnb" | "usdc" | "bnb" => {
                    "base_sepolia".to_string()
                }
                other => other.to_string(),
            }
        }
        crate::merchant::models::merchant::MerchantEnvironmentType::Mainnet => {
            currency_str.to_string()
        }
    };

    let unique_currency_key = format!(
        "{}_{}_{}",
        effective_currency.to_lowercase(),
        String::from(env_type.clone()),
        payment_id
    );

    let base_currency_str = match env_type {
        crate::merchant::models::merchant::MerchantEnvironmentType::Testnet => {
            match effective_currency.as_str() {
                "base_sepolia" => "ethereum", 
                other => other,
            }
        }
        crate::merchant::models::merchant::MerchantEnvironmentType::Mainnet => {
            effective_currency.as_str()
        }
    };

    let currency = crate::shared::service::wallet::Currency::from_str(base_currency_str)?;

    let wallet_service = crate::shared::service::wallet::WalletService::new();
    let (address, _mnemonic) = wallet_service
        .generate_wallet_with_key(user_id, currency, &unique_currency_key, db)
        .await?;

    log::info!(
        "Generated unique {:?} {} wallet for payment {}: {}",
        env_type,
        effective_currency,
        payment_id,
        address
    );

    Ok(address)
}


pub async fn list_payment_requests_jwt(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<PaymentListQuery>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let user_merchants = Merchant::find()
        .filter(crate::shared::entities::merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if user_merchants.is_empty() {
        let empty_response = PaymentListResponse {
            payments: vec![],
            pagination: PaginationInfo {
                page: 1,
                limit: 20,
                total: 0,
                total_pages: 0,
            },
            total_volume_usd: Some(rust_decimal::Decimal::ZERO),
            volume_breakdown: None,
        };

        return Ok(HttpResponse::Ok()
            .insert_header(("x-merchant-setup-required", "true"))
            .insert_header(("x-merchant-message", "No merchant account found. Please complete your merchant setup to start receiving payments."))
            .json(empty_response));
    }

    let merchant_ids: Vec<String> = user_merchants.into_iter().map(|m| m.id).collect();

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let mut payment_query =
        PaymentRequest::find().filter(payment_request::Column::MerchantId.is_in(merchant_ids));

    if let Some(status) = &query.status {
        payment_query = payment_query.filter(payment_request::Column::Status.eq(status));
    }
    if let Some(currency) = &query.currency {
        payment_query =
            payment_query.filter(payment_request::Column::Currency.eq(currency.to_lowercase()));
    }
    if let Some(from_date) = query.from_date {
        payment_query = payment_query.filter(payment_request::Column::CreatedAt.gte(from_date));
    }
    if let Some(to_date) = query.to_date {
        payment_query = payment_query.filter(payment_request::Column::CreatedAt.lte(to_date));
    }
    let environment = query.environment.as_deref().unwrap_or("mainnet");
    payment_query =
        payment_query.filter(payment_request::Column::Environment.eq(environment.to_lowercase()));

    let total_count = payment_query.clone().count(&app_state.db).await?;

    let payments = payment_query
        .order_by_desc(payment_request::Column::CreatedAt)
        .offset(offset)
        .limit(limit)
        .all(&app_state.db)
        .await?;

    let payment_responses: Vec<PaymentResponse> =
        payments.iter().map(payment_to_response).collect();

    let total_pages = ((total_count as f64) / (limit as f64)).ceil() as u64;

    let pagination_info = PaginationInfo {
        page,
        limit,
        total: total_count,
        total_pages,
    };

    let total_volume = payments
        .iter()
        .filter(|p| p.status.to_lowercase() == "paid")
        .filter_map(|p| {
            if let Some(metadata) = &p.metadata {
                if let Ok(meta_json) = serde_json::from_str::<serde_json::Value>(metadata) {
                    if let Some(usd_value) = meta_json.get("usd_amount") {
                        if let Some(usd_str) = usd_value.as_str() {
                            return usd_str.parse::<f64>().ok();
                        } else if let Some(usd_f64) = usd_value.as_f64() {
                            return Some(usd_f64);
                        }
                    }
                }
            }
            None
        })
        .sum::<f64>();

    use crate::merchant::services::payment_service::PaymentService;
    let (total_volume_usd, volume_breakdown) =
        PaymentService::calculate_usd_volume_breakdown(&payment_responses).unwrap_or((None, None));

    let response = PaymentListResponse {
        payments: payment_responses,
        pagination: pagination_info,
        total_volume_usd,
        volume_breakdown,
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn get_payment_request_jwt(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let payment_id = path.into_inner();

    let user_merchants = Merchant::find()
        .filter(crate::shared::entities::merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    if user_merchants.is_empty() {
        return Err(AppError::BadRequest("No merchant account found. Please complete your merchant setup to access payment details.".to_string()));
    }

    let merchant_ids: Vec<String> = user_merchants.into_iter().map(|m| m.id).collect();

    let payment = PaymentRequest::find()
        .filter(payment_request::Column::Id.eq(&payment_id))
        .filter(payment_request::Column::MerchantId.is_in(merchant_ids))
        .one(&app_state.db)
        .await?;

    let payment = match payment {
        Some(p) => p,
        None => {
            return Err(AppError::NotFound(
                "Payment request not found or access denied".to_string(),
            ))
        }
    };

    let response = payment_to_response(&payment);
    Ok(HttpResponse::Ok().json(response))
}
