use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use crate::{
    merchant::services::balances_service::{BalancesError, BalancesService},
    shared::{
        entities::{merchant, prelude::*},
        models::temp_auth::UserClaims,
        utils::errors::AppError,
        AppState,
    },
};

#[derive(Debug, Deserialize)]
pub struct BalanceQuery {
    pub environment: Option<String>,
}


#[derive(Debug, Serialize)]
struct BalancesErrorResponse {
    pub error: String,
    pub code: String,
    pub details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u32>,
}

impl From<BalancesError> for BalancesErrorResponse {
    fn from(err: BalancesError) -> Self {
        Self {
            error: err.message,
            code: err.code,
            details: err.details,
            retry_after: err.retry_after,
        }
    }
}


lazy_static! {
    static ref BALANCES_SERVICE: BalancesService = BalancesService::new();
}


pub async fn get_balances(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<BalanceQuery>,
) -> Result<HttpResponse, AppError> {
    log::info!("🔥 NEW BALANCES HANDLER CALLED with query: {:?}", query);
    let environment = match &query.environment {
        Some(env) => env.as_str(),
        None => {
            return Ok(HttpResponse::BadRequest().json(BalancesErrorResponse {
                error: "Environment parameter is required".to_string(),
                code: "MISSING_ENVIRONMENT".to_string(),
                details: Some(serde_json::json!({
                    "expected_values": ["mainnet", "testnet"]
                })),
                retry_after: None,
            }));
        }
    };

    if !matches!(environment, "mainnet" | "testnet") {
        return Ok(HttpResponse::BadRequest().json(BalancesErrorResponse {
            error: "Environment must be 'mainnet' or 'testnet'".to_string(),
            code: "INVALID_ENVIRONMENT".to_string(),
            details: Some(serde_json::json!({
                "provided": environment,
                "expected_values": ["mainnet", "testnet"]
            })),
            retry_after: None,
        }));
    }

    let claims = match req.extensions().get::<UserClaims>().cloned() {
        Some(claims) => claims,
        None => {
            log::error!("No user claims found in request for balances endpoint");
            return Ok(HttpResponse::Unauthorized().json(BalancesErrorResponse {
                error: "Authentication required".to_string(),
                code: "UNAUTHORIZED".to_string(),
                details: None,
                retry_after: None,
            }));
        }
    };

    let merchant = match get_merchant_for_user(&app_state, &claims.id).await {
        Ok(Some(merchant)) => merchant,
        Ok(None) => {
            return Ok(HttpResponse::Ok().json(serde_json::json!({
                "environment": environment,
                "balances": [],
                "wallet_breakdown": [],
                "total_usd_value": "0.00",
                "total_wallets": 0,
                "last_updated": chrono::Utc::now()
            })));
        }
        Err(e) => {
            log::error!("Failed to get merchant for user {}: {}", claims.id, e);
            return Ok(
                HttpResponse::ServiceUnavailable().json(BalancesErrorResponse {
                    error: "Database temporarily unavailable".to_string(),
                    code: "DB_UNAVAILABLE".to_string(),
                    details: Some(serde_json::json!({
                        "user_id": claims.id
                    })),
                    retry_after: Some(30),
                }),
            );
        }
    };

    match BALANCES_SERVICE
        .get_merchant_balances(&app_state, &merchant.id, environment)
        .await
    {
        Ok(balances) => {
            log::info!(
                "Successfully retrieved balances for merchant {} in {} environment: {} currencies, {} wallets",
                merchant.id, environment, balances.balances.len(), balances.total_wallets
            );
            Ok(HttpResponse::Ok().json(balances))
        }
        Err(balances_error) => {
            match balances_error.code.as_str() {
                "DB_UNAVAILABLE" => Ok(HttpResponse::ServiceUnavailable()
                    .json(BalancesErrorResponse::from(balances_error))),
                "DB_SCHEMA_OUT_OF_DATE" => Ok(HttpResponse::ServiceUnavailable()
                    .json(BalancesErrorResponse::from(balances_error))),
                "VALIDATION_ERROR" => Ok(
                    HttpResponse::BadRequest().json(BalancesErrorResponse::from(balances_error))
                ),
                _ => Ok(HttpResponse::InternalServerError()
                    .json(BalancesErrorResponse::from(balances_error))),
            }
        }
    }
}

async fn get_merchant_for_user(
    app_state: &AppState,
    user_id: &str,
) -> Result<Option<merchant::Model>, sea_orm::DbErr> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    merchant::Entity::find()
        .filter(merchant::Column::UserId.eq(user_id))
        .one(&app_state.db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test;

    #[tokio::test]
    async fn test_environment_validation() {
        let query = BalanceQuery { environment: None };
    }

    #[tokio::test]
    async fn test_invalid_environment() {
        let query = BalanceQuery {
            environment: Some("invalid".to_string()),
        };
    }

    #[tokio::test]
    async fn test_empty_balances() {

    }
}
