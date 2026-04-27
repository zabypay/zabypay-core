use crate::client::models::wallet::WalletGenerationRequest;
use crate::shared::entities::{prelude::*, wallet};
use crate::shared::models::temp_auth::UserClaims;
use crate::shared::service::wallet::{Currency, WalletService};
use crate::shared::utils::api_response::ApiResponse;
use crate::shared::utils::errors::AppError;
use crate::shared::AppState;
use actix_web::{get, post, web, HttpMessage, HttpRequest, HttpResponse};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::str::FromStr;
use uuid::Uuid;

#[get("/test")]
pub async fn test_handler() -> Result<HttpResponse, AppError> {
    println!("=== TEST HANDLER CALLED ===");

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Test handler working!",
        "success": true,
        "status_code": 200
    })))
}

#[get("/test-jwt")]
pub async fn test_jwt_handler(req: HttpRequest) -> Result<HttpResponse, AppError> {
    println!("=== TEST JWT HANDLER CALLED ===");

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    println!("User ID from claims: {}", claims.id);
    println!("User email from claims: {}", claims.email);

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "JWT middleware working!",
        "user_id": claims.id,
        "user_email": claims.email,
        "success": true,
        "status_code": 200
    })))
}

#[get("/test-simple")]
pub async fn test_simple_handler() -> Result<HttpResponse, AppError> {
    println!("=== TEST SIMPLE HANDLER CALLED ===");

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Simple handler working!",
        "success": true,
        "status_code": 200
    })))
}

#[post("/generate-wallet")]
pub async fn generate_wallet_handler(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<WalletGenerationRequest>,
) -> Result<HttpResponse, AppError> {
    println!("=== GENERATE WALLET HANDLER CALLED ===");

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    println!("User ID from claims: {}", claims.id);
    println!("Request body: {:?}", body);

    let currency = match body.currency.to_lowercase().as_str() {
        "bitcoin" | "btc" => Currency::Bitcoin,
        "ethereum" | "eth" => Currency::Ethereum,
        "usdt" | "tether" => Currency::USDT,
        "solana" | "sol" => Currency::Solana,
        "bnb" | "binance" => Currency::BNB,
        _ => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Unsupported currency",
                "supported": ["bitcoin", "ethereum", "usdt","bnb"]
            })));
        }
    };

    let existing_wallet = Wallet::find()
        .filter(wallet::Column::UserId.eq(&claims.id))
        .filter(wallet::Column::Currency.eq(&body.currency))
        .one(&app_state.db)
        .await?;

    if existing_wallet.is_some() {
        return Err(AppError::ValidationError(format!(
            "Wallet for {} already exists for this user",
            body.currency
        )));
    }

    let wallet_service = WalletService::new();
    let currency = Currency::from_str(&body.currency)?;
    let (address, mnemonic) = wallet_service
        .generate_wallet(&claims.id, currency, &app_state.db)
        .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "Wallet created successfully",
        "data": {
            "address": address,
            "currency": body.currency.to_uppercase(),
            "mnemonic": mnemonic,
            "created_at": chrono::Utc::now().to_rfc3339()
        }
    })))
}

#[get("/wallets")]
pub async fn get_wallets(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    println!("=== GET WALLETS HANDLER CALLED ===");

    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    println!("User ID from claims: {}", claims.id);

    let wallets = Wallet::find()
        .filter(wallet::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    let wallet_data: Vec<serde_json::Value> = wallets
        .into_iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "address": w.address,
                "currency": w.currency,
                "balance": "0.00000000",
                "created_at": w.created_at
            })
        })
        .collect();

    println!("Found {} wallets for user", wallet_data.len());

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "wallets": wallet_data,
        "count": wallet_data.len(),
        "message": "Wallets retrieved successfully",
        "success": true,
        "status_code": 200
    })))
}
