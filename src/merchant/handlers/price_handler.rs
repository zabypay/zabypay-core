use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use validator::Validate;

use super::withdrawal_handler::MerchantAuth;
use crate::shared::{
    models::temp_auth::UserClaims,
    service::price_oracle::PRICE_ORACLE,
    utils::{api_response::ApiResponse, errors::AppError},
};


#[derive(Debug, Deserialize, Validate)]
pub struct GetPriceRequest {
    #[validate(length(min = 1, max = 20))]
    pub currency: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct GetMultiplePricesRequest {
    #[validate(length(min = 1, max = 10))]
    pub currencies: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PriceResponse {
    pub currency: String,
    pub usd_price: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub cache_age_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MultiplePricesResponse {
    pub prices: Vec<PriceResponse>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub cache_stats: serde_json::Value,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ConvertCurrencyRequest {
    #[validate(length(min = 1, max = 20))]
    pub from_currency: String,
    #[validate(length(min = 1, max = 20))]
    pub to_currency: String,
    pub amount: String,
}

/// Response with currency conversion
#[derive(Debug, Serialize)]
pub struct ConversionResponse {
    pub original_amount: String,
    pub original_currency: String,
    pub converted_amount: String,
    pub converted_currency: String,
    pub exchange_rate: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub async fn get_usd_price(
    query: web::Query<GetPriceRequest>,
    _auth: MerchantAuth,
) -> Result<HttpResponse, AppError> {
    // Validate input
    query
        .validate()
        .map_err(|e| AppError::ValidationError(format!("Invalid price request: {}", e)))?;

    let currency = query.currency.clone();

    log::info!(" Price request for currency: {}", currency);

    let usd_price = PRICE_ORACLE
        .get_usd_price(&currency)
        .await
        .map_err(|e| AppError::ExternalServiceError(format!("Price fetch failed: {}", e)))?;

    let response = PriceResponse {
        currency: currency.to_uppercase(),
        usd_price: usd_price.to_string(),
        timestamp: chrono::Utc::now(),
        cache_age_seconds: None, 
    };

    log::info!("✅ Price response for {}: ${}", currency, usd_price);

    Ok(HttpResponse::Ok().json(ApiResponse::success(response, "Price fetched successfully")))
}


pub async fn get_multiple_usd_prices(
    payload: web::Json<GetMultiplePricesRequest>,
    _auth: MerchantAuth,
) -> Result<HttpResponse, AppError> {
    // Validate input
    payload.validate().map_err(|e| {
        AppError::ValidationError(format!("Invalid multiple prices request: {}", e))
    })?;

    let currencies: Vec<&str> = payload.currencies.iter().map(|s| s.as_str()).collect();

    log::info!(" Multiple price request for currencies: {:?}", currencies);

    let prices_map = PRICE_ORACLE
        .get_multiple_usd_prices(&currencies)
        .await
        .map_err(|e| {
            AppError::ExternalServiceError(format!("Multiple price fetch failed: {}", e))
        })?;

    let prices: Vec<PriceResponse> = prices_map
        .into_iter()
        .map(|(currency, usd_price)| PriceResponse {
            currency: currency.to_uppercase(),
            usd_price: usd_price.to_string(),
            timestamp: chrono::Utc::now(),
            cache_age_seconds: None,
        })
        .collect();

    let cache_stats = PRICE_ORACLE.get_cache_stats().await;

    let response = MultiplePricesResponse {
        prices,
        timestamp: chrono::Utc::now(),
        cache_stats: serde_json::to_value(cache_stats).unwrap_or(serde_json::Value::Null),
    };

    log::info!(
        "✅ Multiple price response: {} currencies",
        response.prices.len()
    );

    Ok(HttpResponse::Ok().json(ApiResponse::success(
        response,
        "Multiple prices fetched successfully",
    )))
}

pub async fn convert_currency(
    payload: web::Json<ConvertCurrencyRequest>,
    _auth: MerchantAuth,
) -> Result<HttpResponse, AppError> {
    // Validate input
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(format!("Invalid conversion request: {}", e)))?;

    let from_currency = payload.from_currency.clone();
    let to_currency = payload.to_currency.clone();
    let amount_str = payload.amount.clone();

    let amount = amount_str
        .parse::<rust_decimal::Decimal>()
        .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

    log::info!(
        "🔄 Currency conversion: {} {} to {}",
        amount,
        from_currency,
        to_currency
    );

    if from_currency.to_lowercase() == to_currency.to_lowercase() {
        let response = ConversionResponse {
            original_amount: amount_str.clone(),
            original_currency: from_currency.to_uppercase(),
            converted_amount: amount_str,
            converted_currency: to_currency.to_uppercase(),
            exchange_rate: "1.0".to_string(),
            timestamp: chrono::Utc::now(),
        };
        return Ok(HttpResponse::Ok().json(ApiResponse::success(
            response,
            "Currency conversion successful",
        )));
    }

    let currencies = vec![from_currency.as_str(), to_currency.as_str()];
    let prices = PRICE_ORACLE
        .get_multiple_usd_prices(&currencies)
        .await
        .map_err(|e| {
            AppError::ExternalServiceError(format!("Conversion price fetch failed: {}", e))
        })?;

    let from_price = prices.get(&from_currency.to_lowercase()).ok_or_else(|| {
        AppError::ExternalServiceError(format!("Price not found for {}", from_currency))
    })?;

    let to_price = prices.get(&to_currency.to_lowercase()).ok_or_else(|| {
        AppError::ExternalServiceError(format!("Price not found for {}", to_currency))
    })?;

    let usd_value = amount * from_price;
    let converted_amount = usd_value / to_price;
    let exchange_rate = from_price / to_price;

    let response = ConversionResponse {
        original_amount: amount_str,
        original_currency: from_currency.to_uppercase(),
        converted_amount: converted_amount.to_string(),
        converted_currency: to_currency.to_uppercase(),
        exchange_rate: exchange_rate.to_string(),
        timestamp: chrono::Utc::now(),
    };

    log::info!(
        "✅ Conversion result: {} {} = {} {} (rate: {})",
        amount,
        from_currency,
        converted_amount,
        to_currency,
        exchange_rate
    );

    Ok(HttpResponse::Ok().json(ApiResponse::success(
        response,
        "Currency conversion successful",
    )))
}

pub async fn get_cache_stats(_auth: MerchantAuth) -> Result<HttpResponse, AppError> {
    log::info!("📈 Price cache stats request");

    let cache_stats = PRICE_ORACLE.get_cache_stats().await;

    log::info!("✅ Cache stats retrieved");

    Ok(HttpResponse::Ok().json(ApiResponse::success(
        cache_stats,
        "Cache stats retrieved successfully",
    )))
}

/// Clear price cache (for testing/manual refresh)
pub async fn clear_cache(_auth: MerchantAuth) -> Result<HttpResponse, AppError> {
    log::info!("🗑️ Clearing price cache");

    PRICE_ORACLE.clear_cache().await;

    log::info!("✅ Price cache cleared");

    Ok(HttpResponse::Ok().json(ApiResponse::success(
        "Cache cleared successfully",
        "Cache cleared successfully",
    )))
}

/// Health check for price oracle service
pub async fn price_health_check() -> Result<HttpResponse, AppError> {
    log::info!("🏥 Price oracle health check");

    // Try to get a basic price to test connectivity
    match PRICE_ORACLE.get_usd_price("bitcoin").await {
        Ok(price) => {
            let health_status = serde_json::json!({
                "status": "healthy",
                "timestamp": chrono::Utc::now(),
                "test_price_btc": price.to_string(),
                "service": "price_oracle"
            });

            log::info!("✅ Price oracle health check passed");
            Ok(HttpResponse::Ok().json(ApiResponse::success(
                health_status,
                "Price oracle health check passed",
            )))
        }
        Err(e) => {
            let health_status = serde_json::json!({
                "status": "unhealthy",
                "timestamp": chrono::Utc::now(),
                "error": e.to_string(),
                "service": "price_oracle"
            });

            log::warn!(" Price oracle health check failed: {}", e);
            Ok(HttpResponse::ServiceUnavailable().json(ApiResponse::error(
                503,
                &format!("Price oracle unhealthy: {}", e),
            )))
        }
    }
}
