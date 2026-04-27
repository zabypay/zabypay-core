use crate::{
    merchant::models::merchant::{ApiKeyCreatedResponse, ApiKeyResponse, CreateApiKeyRequest},
    merchant::services::api_key_service::ApiKeyService,
    shared::models::temp_auth::UserClaims,
    shared::AppState,
    shared::{
        entities::{api_key, merchant},
        utils::{api_key::generate_api_key_pair, errors::AppError},
    },
};
use actix_web::{get, post, web, HttpMessage, HttpRequest, HttpResponse};
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;
use validator::Validate;

#[post("/merchants/{merchant_id}/api-keys")]
pub async fn create_api_key(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
    req: HttpRequest,
    body: web::Json<CreateApiKeyRequest>,
) -> Result<HttpResponse, AppError> {
    let extensions = req.extensions();
    let claim = extensions
        .get::<UserClaims>()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    // Check if merchant exists and belongs to user
    let _merchant = merchant::Entity::find()
        .filter(merchant::Column::Id.eq(&merchant_id))
        .filter(merchant::Column::UserId.eq(&claim.sub))
        .one(&app_state.db)
        .await
        .map_err(AppError::from)?
        .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

    // Generate API key pair
    let key_pair = generate_api_key_pair()?;

    let environment_type = body.environment_type.clone().unwrap_or_default();

    let new_api_key = api_key::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        merchant_id: Set(merchant_id),
        name: Set(body.name.clone()),
        key_prefix: Set(key_pair.key_prefix.clone()),
        key_hash: Set(key_pair.key_hash),
        secret_hash: Set(key_pair.secret_hash),
        api_key: Set(Some(key_pair.api_key.clone())), // Store plain text for display
        secret_key: Set(Some(key_pair.secret_key.clone())), // Store plain text for display
        environment_type: Set(String::from(environment_type.clone())),
        is_active: Set(true),
        created_at: Set(Utc::now().into()),
        last_used: Set(None),
        ..Default::default()
    };

    let created_api_key = api_key::Entity::insert(new_api_key)
        .exec_with_returning(&app_state.db)
        .await
        .map_err(AppError::from)?;

    let response = ApiKeyCreatedResponse {
        id: created_api_key.id,
        name: created_api_key.name,
        api_key: key_pair.api_key,
        secret_key: key_pair.secret_key,
        environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(
            created_api_key.environment_type,
        ),
        is_active: created_api_key.is_active,
        created_at: created_api_key.created_at.to_rfc3339(),
    };

    Ok(HttpResponse::Created().json(response))
}

#[get("/merchants/{merchant_id}/api-keys")]
pub async fn list_api_keys(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let extensions = req.extensions();
    let claim = extensions
        .get::<UserClaims>()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let merchant_id = path.into_inner();

    // Check if merchant exists and belongs to user
    let _merchant = merchant::Entity::find()
        .filter(merchant::Column::Id.eq(&merchant_id))
        .filter(merchant::Column::UserId.eq(&claim.sub))
        .one(&app_state.db)
        .await
        .map_err(AppError::from)?
        .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

    let api_keys = api_key::Entity::find()
        .filter(api_key::Column::MerchantId.eq(&merchant_id))
        .all(&app_state.db)
        .await
        .map_err(AppError::from)?;

    let response: Vec<ApiKeyResponse> = api_keys
        .into_iter()
        .map(|key| ApiKeyResponse {
            id: key.id,
            name: key.name,
            key_prefix: key.key_prefix,
            api_key: key.api_key, // This is now Option<String> and will be populated
            secret_key: key.secret_key, // This is now Option<String> and will be populated
            environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(
                key.environment_type,
            ),
            is_active: key.is_active,
            created_at: key.created_at.to_string(),
            last_used: key.last_used.map(|dt| dt.to_string()),
        })
        .collect();

    Ok(HttpResponse::Ok().json(response))
}

#[get("/merchants/{merchant_id}/api-keys/{api_key_id}/reveal")]
pub async fn reveal_api_key(
    app_state: web::Data<AppState>,
    path: web::Path<(String, String)>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let extensions = req.extensions();
    let claim = extensions
        .get::<UserClaims>()
        .ok_or(AppError::Unauthorized("Unauthorized".to_string()))?;
    let (merchant_id, api_key_id) = path.into_inner();

    // Get API key values using the service
    let (api_key, secret_key) =
        ApiKeyService::get_api_key_values(&app_state, &api_key_id, &merchant_id, &claim.sub)
            .await?;

    let response = serde_json::json!({
        "api_key": api_key,
        "secret_key": secret_key,
        "warning": "These keys will only be shown once. Please store them securely."
    });

    Ok(HttpResponse::Ok().json(response))
}
