use crate::merchant::models::merchant::{ApiKeyResponse, CreateApiKeyRequest};
use crate::shared::entities::{api_key, merchant, prelude::*};
use crate::shared::utils::errors::AppError;
use crate::shared::AppState;
use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::{distributions::Alphanumeric, Rng};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uuid::Uuid;

/// API Key Service
/// Handles secure API key generation, validation, and management
pub struct ApiKeyService;

impl ApiKeyService {
    /// Create a new API key for a merchant
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant_id` - ID of the merchant
    /// * `user_id` - ID of the requesting user (for ownership validation)
    /// * `request` - API key creation request
    ///
    /// # Returns
    /// * `Result<(ApiKeyResponse, String, String), AppError>` - (key_info, api_key, secret_key)
    pub async fn create_api_key(
        app_state: &AppState,
        merchant_id: &str,
        user_id: &str,
        request: CreateApiKeyRequest,
    ) -> Result<(ApiKeyResponse, String, String), AppError> {
        // Validate merchant exists and belongs to user
        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id))
            .one(&app_state.db)
            .await?;

        if merchant.is_none() {
            return Err(AppError::NotFound("Merchant not found".to_string()));
        }

        // Check if merchant already has an API key with this name
        let existing_key = ApiKey::find()
            .filter(api_key::Column::MerchantId.eq(merchant_id))
            .filter(api_key::Column::Name.eq(&request.name))
            .one(&app_state.db)
            .await?;

        if existing_key.is_some() {
            return Err(AppError::ValidationError(
                "API key with this name already exists for this merchant".to_string(),
            ));
        }

        // Generate API key and secret
        let (api_key_value, secret_key_value) = Self::generate_api_key_pair();

        // Hash the keys for storage
        let key_hash = Self::hash_key(&api_key_value)?;
        let secret_hash = Self::hash_key(&secret_key_value)?;

        // Extract key prefix for fast lookup (first 8 chars after prefix)
        let key_prefix = &api_key_value[3..11]; // Skip 'ak_' prefix, take next 8 chars

        // Store plain text keys directly

        // Create API key record
        let api_key_id = Uuid::new_v4().to_string();
        let environment_type = request.environment_type.unwrap_or_default();

        let new_api_key = api_key::ActiveModel {
            id: Set(api_key_id.clone()),
            merchant_id: Set(merchant_id.to_string()),
            name: Set(request.name.clone()),
            key_prefix: Set(key_prefix.to_string()),
            key_hash: Set(key_hash),
            secret_hash: Set(secret_hash),
            encrypted_api_key: Set(None),    // Not using encryption
            encrypted_secret_key: Set(None), // Not using encryption
            api_key: Set(Some(api_key_value.clone())), // Store plain text API key
            secret_key: Set(Some(secret_key_value.clone())), // Store plain text secret key
            permissions: Set("[]".to_string()), // Default empty permissions
            environment_type: Set(String::from(environment_type.clone())),
            expires_at: Set(None),
            is_active: Set(true),
            last_used: Set(None),
            rate_limit_per_minute: Set(Some(1000)), // Default rate limit
            rate_limit_per_hour: Set(Some(10000)),  // Default rate limit
            last_revoked_at: Set(None),
            created_at: Set(chrono::Utc::now().into()),
        };

        let saved_api_key = new_api_key.insert(&app_state.db).await?;

        println!(
            "✅ API key created successfully: {} (ID: {})",
            saved_api_key.name, saved_api_key.id
        );

        let response = ApiKeyResponse {
            id: saved_api_key.id,
            name: saved_api_key.name,
            key_prefix: saved_api_key.key_prefix,
            api_key: saved_api_key.api_key,
            secret_key: saved_api_key.secret_key,
            environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(
                saved_api_key.environment_type,
            ),
            is_active: saved_api_key.is_active,
            last_used: saved_api_key.last_used.map(|dt| dt.to_rfc3339()),
            created_at: saved_api_key.created_at.to_rfc3339(),
        };

        Ok((response, api_key_value, secret_key_value))
    }

    /// Get all API keys for a merchant
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant_id` - ID of the merchant
    /// * `user_id` - ID of the requesting user (for ownership validation)
    ///
    /// # Returns
    /// * `Result<Vec<ApiKeyResponse>, AppError>` - List of API keys (without sensitive data)
    pub async fn get_merchant_api_keys(
        app_state: &AppState,
        merchant_id: &str,
        user_id: &str,
    ) -> Result<Vec<ApiKeyResponse>, AppError> {
        // Validate merchant exists and belongs to user
        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id))
            .one(&app_state.db)
            .await?;

        if merchant.is_none() {
            return Err(AppError::NotFound("Merchant not found".to_string()));
        }

        // Get API keys
        let api_keys = ApiKey::find()
            .filter(api_key::Column::MerchantId.eq(merchant_id))
            .order_by_desc(api_key::Column::CreatedAt)
            .all(&app_state.db)
            .await?;

        let api_key_responses: Vec<ApiKeyResponse> = api_keys
            .into_iter()
            .map(|key| ApiKeyResponse {
                id: key.id,
                name: key.name,
                key_prefix: key.key_prefix,
                api_key: key.api_key,       // Full plain text API key
                secret_key: key.secret_key, // Full plain text secret key
                environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(
                    key.environment_type,
                ),
                is_active: key.is_active,
                last_used: key.last_used.map(|dt| dt.to_rfc3339()),
                created_at: key.created_at.to_rfc3339(),
            })
            .collect();

        Ok(api_key_responses)
    }

    /// Validate API key and return merchant info
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `api_key` - API key to validate
    ///
    /// # Returns
    /// * `Result<(api_key::Model, merchant::Model), AppError>` - (api_key, merchant) if valid
    pub async fn validate_api_key(
        app_state: &AppState,
        api_key_value: &str,
    ) -> Result<(api_key::Model, merchant::Model), AppError> {
        // Extract key prefix for fast lookup
        let key_prefix = if api_key_value.starts_with("ak_") && api_key_value.len() >= 11 {
            &api_key_value[3..11]
        } else {
            return Err(AppError::Unauthorized("Invalid API key format".to_string()));
        };

        // Find API key by prefix
        let api_key_record = ApiKey::find()
            .filter(api_key::Column::KeyPrefix.eq(key_prefix))
            .filter(api_key::Column::IsActive.eq(true))
            .one(&app_state.db)
            .await?;

        let api_key_record = match api_key_record {
            Some(key) => key,
            None => return Err(AppError::Unauthorized("Invalid API key".to_string())),
        };

        // Expiration check removed - API keys never expire

        // Verify the API key hash
        if !Self::verify_key(api_key_value, &api_key_record.key_hash)? {
            return Err(AppError::Unauthorized("Invalid API key".to_string()));
        }

        // Get merchant info
        let merchant = Merchant::find_by_id(&api_key_record.merchant_id)
            .one(&app_state.db)
            .await?;

        let merchant = match merchant {
            Some(m) => m,
            None => return Err(AppError::NotFound("Merchant not found".to_string())),
        };

        // Check if merchant is active
        if !merchant.is_active {
            return Err(AppError::Unauthorized(
                "Merchant account is inactive".to_string(),
            ));
        }

        // Update last used timestamp (fire and forget)
        let _ = Self::update_last_used(app_state, &api_key_record.id).await;

        Ok((api_key_record, merchant))
    }

    /// Deactivate an API key
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `api_key_id` - ID of the API key to deactivate
    /// * `merchant_id` - ID of the merchant (for ownership validation)
    /// * `user_id` - ID of the requesting user (for ownership validation)
    ///
    /// # Returns
    /// * `Result<(), AppError>` - Success or error
    pub async fn deactivate_api_key(
        app_state: &AppState,
        api_key_id: &str,
        merchant_id: &str,
        user_id: &str,
    ) -> Result<(), AppError> {
        // Validate merchant exists and belongs to user
        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id))
            .one(&app_state.db)
            .await?;

        if merchant.is_none() {
            return Err(AppError::NotFound("Merchant not found".to_string()));
        }

        // Find and deactivate API key
        let api_key_record = ApiKey::find()
            .filter(api_key::Column::Id.eq(api_key_id))
            .filter(api_key::Column::MerchantId.eq(merchant_id))
            .one(&app_state.db)
            .await?;

        let api_key_record = match api_key_record {
            Some(key) => key,
            None => return Err(AppError::NotFound("API key not found".to_string())),
        };

        // Deactivate the key
        let mut update_model: api_key::ActiveModel = api_key_record.into();
        update_model.is_active = Set(false);

        update_model.update(&app_state.db).await?;

        println!("✅ API key deactivated successfully: {}", api_key_id);

        Ok(())
    }

    /// Generate API key and secret key pair
    ///
    /// Format:
    /// - API Key: ak_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx (36 chars total)
    /// - Secret: sk_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx (67 chars total)
    fn generate_api_key_pair() -> (String, String) {
        let api_key = format!(
            "ak_{}",
            rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(32)
                .map(char::from)
                .collect::<String>()
        );

        let secret_key = format!(
            "sk_{}",
            rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(64)
                .map(char::from)
                .collect::<String>()
        );

        (api_key, secret_key)
    }

    /// Hash a key using Argon2
    fn hash_key(key: &str) -> Result<String, AppError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        argon2
            .hash_password(key.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| AppError::HashingError)
    }

    /// Verify a key against its hash
    fn verify_key(key: &str, hash: &str) -> Result<bool, AppError> {
        let parsed_hash = PasswordHash::new(hash).map_err(|_| AppError::HashingError)?;

        let argon2 = Argon2::default();

        Ok(argon2.verify_password(key.as_bytes(), &parsed_hash).is_ok())
    }

    /// Update last used timestamp for an API key
    async fn update_last_used(app_state: &AppState, api_key_id: &str) -> Result<(), AppError> {
        let api_key_record = ApiKey::find_by_id(api_key_id).one(&app_state.db).await?;

        if let Some(key) = api_key_record {
            let mut update_model: api_key::ActiveModel = key.into();
            update_model.last_used = Set(Some(chrono::Utc::now().into()));
            update_model.update(&app_state.db).await?;
        }

        Ok(())
    }

    /// Get API key values (for display in UI)
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `api_key_id` - ID of the API key to retrieve
    /// * `merchant_id` - ID of the merchant (for ownership validation)
    /// * `user_id` - ID of the requesting user (for ownership validation)
    ///
    /// # Returns
    /// * `Result<(String, String), AppError>` - (api_key, secret_key) if valid
    pub async fn get_api_key_values(
        app_state: &AppState,
        api_key_id: &str,
        merchant_id: &str,
        user_id: &str,
    ) -> Result<(String, String), AppError> {
        // Validate merchant exists and belongs to user
        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id))
            .one(&app_state.db)
            .await?;

        if merchant.is_none() {
            return Err(AppError::NotFound("Merchant not found".to_string()));
        }

        // Find API key
        let api_key_record = ApiKey::find()
            .filter(api_key::Column::Id.eq(api_key_id))
            .filter(api_key::Column::MerchantId.eq(merchant_id))
            .one(&app_state.db)
            .await?;

        let api_key_record = match api_key_record {
            Some(key) => key,
            None => return Err(AppError::NotFound("API key not found".to_string())),
        };

        // Get the plain text keys
        let api_key_value = api_key_record
            .api_key
            .ok_or_else(|| AppError::NotFound("API key value not available".to_string()))?;

        let secret_key_value = api_key_record
            .secret_key
            .ok_or_else(|| AppError::NotFound("Secret key value not available".to_string()))?;

        Ok((api_key_value, secret_key_value))
    }

    // Permissions system removed - all API keys have full access
}
