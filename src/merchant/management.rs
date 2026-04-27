use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Result};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, NotSet, QueryFilter, Set};
use serde_json::json;

use crate::{
    shared::models::temp_auth::UserClaims,
    shared::{
        entities::{
            api_key::{ActiveModel as ApiKeyActiveModel, Column as ApiKeyCol, Entity as ApiKey},
            merchant::{ActiveModel as MerchantActiveModel, Entity as Merchant},
        },
        utils::{api_key::generate_api_key_pair, errors::AppError},
        AppState,
    },
};

use crate::merchant::models::merchant::{
    ApiKeyCreatedResponse, ApiKeyResponse, CreateApiKeyRequest, CreateMerchantRequest,
    MerchantEnvironmentType, MerchantResponse, UpdateMerchantRequest,
};
use crate::merchant::services::wallet_generation_service::WalletGenerationService;
use crate::shared::utils::network_config::{EnvironmentType, NetworkConfigManager};


pub async fn create_merchant_handler(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    merchant_data: web::Json<CreateMerchantRequest>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    log::info!(
        "Creating merchant: user_id={}, name={}, has_description={}, has_website_url={}, has_webhook_url={}",
        claims.id,
        merchant_data.name,
        merchant_data.description.is_some(),
        merchant_data.website_url.is_some(),
        merchant_data.webhook_url.is_some()
    );

    let environment_type = MerchantEnvironmentType::Mainnet; 
    let preferred_networks: Vec<String> = vec![]; 

    let new_merchant = MerchantActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        user_id: Set(claims.id.clone()),
        name: Set(merchant_data.name.clone()),
        description: Set(merchant_data.description.clone()),
        website_url: Set(merchant_data.website_url.clone()),
        webhook_url: Set(merchant_data.webhook_url.clone()),
        webhook_secret: Set(Some(uuid::Uuid::new_v4().to_string())),
        environment_type: Set(String::from(environment_type.clone())),
        preferred_networks: Set(Some(
            serde_json::to_string(&preferred_networks).unwrap_or_default(),
        )),
        token_version: Set(1),
        is_active: Set(true),
        created_at: Set(chrono::Utc::now().into()),
        updated_at: Set(Some(chrono::Utc::now().into())),
    };

    log::debug!("Attempting database insert for merchant");

    let merchant = match new_merchant.insert(&app_state.db).await {
        Ok(merchant) => {
            log::info!(
                "Successfully created merchant: id={}, name={}",
                merchant.id,
                merchant.name
            );
            merchant
        }
        Err(db_err) => {
            log::error!("Database error creating merchant: {:?}", db_err);

            return match &db_err {
                sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(sqlx_err)) => {
                    match sqlx_err {
                        sqlx::Error::Database(db_error) => {
                            let code = db_error.code();
                            let constraint = db_error.constraint();

                            log::warn!(
                                "Database constraint error: code={:?}, constraint={:?}, message={}",
                                code,
                                constraint,
                                db_error.message()
                            );

                            if let Some(constraint_name) = constraint {
                                if constraint_name.contains("merchant_name")
                                    || constraint_name.contains("unique_merchant_name")
                                {
                                    return Err(AppError::Conflict(
                                        json!({
                                            "code": "merchant_name_conflict",
                                            "message": "A merchant with this name already exists"
                                        })
                                        .to_string(),
                                    ));
                                }
                            }

                            if code.as_ref().map(|c| c.as_ref()) == Some("23505") {
                                return Err(AppError::Conflict(
                                    json!({
                                        "code": "merchant_name_conflict",
                                        "message": "A merchant with this name already exists"
                                    })
                                    .to_string(),
                                ));
                            }

                            Err(AppError::DatabaseError(format!(
                                "Database constraint error: {}",
                                db_error.message()
                            )))
                        }
                        _ => Err(AppError::DatabaseError(format!(
                            "Database connection error: {}",
                            sqlx_err
                        ))),
                    }
                }
                sea_orm::DbErr::ConnectionAcquire(_) => Err(AppError::ServiceUnavailable(
                    "Database connection unavailable".to_string(),
                )),
                _ => Err(AppError::DatabaseError(format!(
                    "Unexpected database error: {}",
                    db_err
                ))),
            };
        }
    };

    let preferred_networks = merchant
        .preferred_networks
        .as_ref()
        .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
        .unwrap_or_default();

    let response = MerchantResponse {
        id: merchant.id,
        name: merchant.name,
        description: merchant.description,
        website_url: merchant.website_url,
        webhook_url: merchant.webhook_url,
        environment_type: MerchantEnvironmentType::from(merchant.environment_type),
        preferred_networks,
        is_active: merchant.is_active,
        created_at: merchant.created_at.to_rfc3339(),
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn list_merchants_handler(
    app_state: web::Data<AppState>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;

    let merchants = Merchant::find()
        .filter(crate::shared::entities::merchant::Column::UserId.eq(&claims.id))
        .all(&app_state.db)
        .await?;

    let merchant_responses: Vec<MerchantResponse> = merchants
        .into_iter()
        .map(|merchant| {
            let preferred_networks = merchant
                .preferred_networks
                .as_ref()
                .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
                .unwrap_or_default();

            MerchantResponse {
                id: merchant.id,
                name: merchant.name,
                description: merchant.description,
                website_url: merchant.website_url,
                webhook_url: merchant.webhook_url,
                environment_type: MerchantEnvironmentType::from(merchant.environment_type),
                preferred_networks,
                is_active: merchant.is_active,
                created_at: merchant.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(json!({
        "merchants": merchant_responses
    })))
}

pub async fn get_merchant(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;

    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    let preferred_networks = merchant
        .preferred_networks
        .as_ref()
        .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
        .unwrap_or_default();

    let response = MerchantResponse {
        id: merchant.id,
        name: merchant.name,
        description: merchant.description,
        website_url: merchant.website_url,
        webhook_url: merchant.webhook_url,
        environment_type: MerchantEnvironmentType::from(merchant.environment_type),
        preferred_networks,
        is_active: merchant.is_active,
        created_at: merchant.created_at.to_rfc3339(),
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn update_merchant(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    update_data: web::Json<UpdateMerchantRequest>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;

    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    let mut merchant_active: MerchantActiveModel = merchant.into();

    if let Some(name) = &update_data.name {
        merchant_active.name = Set(name.clone());
    }
    if let Some(description) = &update_data.description {
        merchant_active.description = Set(Some(description.clone()));
    }
    if let Some(website_url) = &update_data.website_url {
        merchant_active.website_url = Set(Some(website_url.clone()));
    }
    if let Some(webhook_url) = &update_data.webhook_url {
        merchant_active.webhook_url = Set(Some(webhook_url.clone()));
    }
    if let Some(is_active) = update_data.is_active {
        merchant_active.is_active = Set(is_active);
    }

    merchant_active.updated_at = Set(Some(chrono::Utc::now().into()));

    let updated_merchant = merchant_active.update(&app_state.db).await?;

    let preferred_networks = updated_merchant
        .preferred_networks
        .as_ref()
        .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
        .unwrap_or_default();

    let response = MerchantResponse {
        id: updated_merchant.id,
        name: updated_merchant.name,
        description: updated_merchant.description,
        website_url: updated_merchant.website_url,
        webhook_url: updated_merchant.webhook_url,
        environment_type: MerchantEnvironmentType::from(updated_merchant.environment_type),
        preferred_networks,
        is_active: updated_merchant.is_active,
        created_at: updated_merchant.created_at.to_rfc3339(),
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn delete_merchant(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;
    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    ApiKey::delete_many()
        .filter(ApiKeyCol::MerchantId.eq(&merchant_id))
        .exec(&app_state.db)
        .await?;

    Merchant::delete_by_id(&merchant_id)
        .exec(&app_state.db)
        .await?;

    Ok(HttpResponse::Ok().json(json!({
        "message": "Merchant and all associated API keys deleted successfully",
        "deleted": true
    })))
}


pub async fn create_api_key(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
    api_key_data: web::Json<CreateApiKeyRequest>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;
    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    let existing_key = ApiKey::find()
        .filter(ApiKeyCol::MerchantId.eq(&merchant_id))
        .filter(ApiKeyCol::IsActive.eq(true))
        .one(&app_state.db)
        .await?;

    if existing_key.is_some() {
        return Err(AppError::Conflict(
            "Merchant already has an active API key. Delete the existing key first.".into(),
        ));
    }

    let key_pair = generate_api_key_pair()?;

    let environment_type = api_key_data.environment_type.clone().unwrap_or_default();

    let new_api_key = ApiKeyActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        merchant_id: Set(merchant_id.clone()),
        name: Set(api_key_data.name.clone()),
        key_hash: Set(key_pair.key_hash),
        secret_hash: Set(key_pair.secret_hash),
        key_prefix: Set(key_pair.key_prefix),
        encrypted_api_key: Set(None),    
        encrypted_secret_key: Set(None), 
        api_key: Set(Some(key_pair.api_key.clone())), 
        secret_key: Set(Some(key_pair.secret_key.clone())), 
        permissions: Set("[]".to_string()), 
        environment_type: Set(String::from(environment_type.clone())),
        expires_at: Set(None),
        is_active: Set(true),
        last_used: Set(None),
        rate_limit_per_minute: Set(None),
        rate_limit_per_hour: Set(None),
        last_revoked_at: Set(None),
        created_at: Set(chrono::Utc::now().into()),
    };

    let saved_api_key = new_api_key.insert(&app_state.db).await?;

    let response = ApiKeyCreatedResponse {
        id: saved_api_key.id,
        name: saved_api_key.name,
        api_key: key_pair.api_key,
        secret_key: key_pair.secret_key,
        environment_type: MerchantEnvironmentType::from(saved_api_key.environment_type),
        is_active: saved_api_key.is_active,
        created_at: saved_api_key.created_at.to_rfc3339(),
    };

    Ok(HttpResponse::Ok().json(response))
}

pub async fn list_api_keys(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;
    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    let api_keys = ApiKey::find()
        .filter(ApiKeyCol::MerchantId.eq(&merchant_id))
        .filter(ApiKeyCol::IsActive.eq(true))
        .all(&app_state.db)
        .await?;

    let api_key_responses: Vec<ApiKeyResponse> = api_keys
        .into_iter()
        .map(|key| {
            ApiKeyResponse {
                id: key.id,
                name: key.name,
                key_prefix: key.key_prefix,
                api_key: key.api_key,       
                secret_key: key.secret_key, 
                environment_type: MerchantEnvironmentType::from(key.environment_type),
                is_active: key.is_active,
                created_at: key.created_at.to_rfc3339(),
                last_used: key.last_used.map(|dt| dt.to_rfc3339()),
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(api_key_responses))
}

pub async fn delete_api_key(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let (merchant_id, key_id) = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;
    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    let api_key = ApiKey::find_by_id(&key_id).one(&app_state.db).await?;
    let api_key = api_key.ok_or(AppError::NotFound("API key not found".into()))?;
    if api_key.merchant_id != merchant_id {
        return Err(AppError::Forbidden(
            "API key does not belong to this merchant".into(),
        ));
    }

    ApiKey::delete_by_id(&key_id).exec(&app_state.db).await?;

    Ok(HttpResponse::Ok().json(json!({
        "message": "API key deleted successfully",
        "deleted": true
    })))
}

pub async fn reveal_api_key(
    app_state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse, AppError> {
    let claims = req
        .extensions()
        .get::<UserClaims>()
        .cloned()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let (merchant_id, key_id) = path.into_inner();

    let merchant = Merchant::find_by_id(&merchant_id)
        .one(&app_state.db)
        .await?;
    let merchant = merchant.ok_or(AppError::NotFound("Merchant not found".into()))?;
    if merchant.user_id != claims.id {
        return Err(AppError::Forbidden("Not allowed".into()));
    }

    let api_key = ApiKey::find_by_id(&key_id).one(&app_state.db).await?;
    let api_key = api_key.ok_or(AppError::NotFound("API key not found".into()))?;
    if api_key.merchant_id != merchant_id {
        return Err(AppError::Forbidden(
            "API key does not belong to this merchant".into(),
        ));
    }

    let api_key_value = api_key
        .api_key
        .ok_or_else(|| AppError::NotFound("API key value not available".to_string()))?;

    let secret_key_value = api_key
        .secret_key
        .ok_or_else(|| AppError::NotFound("Secret key value not available".to_string()))?;

    let response = json!({
        "id": api_key.id,
        "name": api_key.name,
        "api_key": api_key_value,
        "secret_key": secret_key_value,
        "environment_type": MerchantEnvironmentType::from(api_key.environment_type),
        "created_at": api_key.created_at.to_rfc3339(),
        "warning": "Store these keys securely. For security reasons, avoid storing them in plain text."
    });

    Ok(HttpResponse::Ok().json(response))
}

pub async fn get_supported_networks(
    _app_state: web::Data<AppState>,
    _req: HttpRequest,
    query: web::Query<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    let environment = query
        .get("environment")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "testnet" => EnvironmentType::Testnet,
            _ => EnvironmentType::Mainnet,
        })
        .unwrap_or(EnvironmentType::Mainnet);

    let networks = NetworkConfigManager::get_networks_by_environment(environment.clone());
    let network_list: Vec<serde_json::Value> = networks
        .into_iter()
        .map(|(key, config)| {
            json!({
                "key": key,
                "name": config.chain_name,
                "chain_id": config.chain_id,
                "currency": config.native_currency,
                "network_type": config.network_type,
                "environment_type": config.environment_type,
                "supports_eip681": config.supports_eip681,
                "wallet_support": config.wallet_support,
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(json!({
        "environment": environment,
        "networks": network_list
    })))
}

pub async fn get_supported_currencies(
    _app_state: web::Data<AppState>,
    _req: HttpRequest,
    query: web::Query<serde_json::Value>,
) -> Result<HttpResponse, AppError> {
    let environment_str = query
        .get("environment")
        .and_then(|v| v.as_str())
        .unwrap_or("mainnet");

    let merchant_environment = match environment_str {
        "testnet" => MerchantEnvironmentType::Testnet,
        _ => MerchantEnvironmentType::Mainnet,
    };

    let currencies =
        WalletGenerationService::get_supported_currencies(Some(merchant_environment.clone()));

    Ok(HttpResponse::Ok().json(json!({
        "environment": merchant_environment,
        "supported_currencies": currencies
    })))
}
