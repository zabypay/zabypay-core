use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, PaginatorTrait, QueryOrder, QuerySelect};
use uuid::Uuid;
use crate::shared::AppState;
use crate::shared::entities::{merchant, prelude::*};
use crate::shared::utils::errors::AppError;
use crate::merchant::models::merchant::{CreateMerchantRequest, UpdateMerchantRequest, MerchantResponse};

/// Merchant Business Service
/// Handles all merchant-related business logic with proper validation and error handling
pub struct MerchantService;

impl MerchantService {
    /// Create a new merchant account
    /// 
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `user_id` - ID of the user creating the merchant
    /// * `request` - Merchant creation request data
    /// 
    /// # Returns
    /// * `Result<MerchantResponse, AppError>` - Created merchant or error
    pub async fn create_merchant(
        app_state: &AppState,
        user_id: &str,
        request: CreateMerchantRequest,
    ) -> Result<MerchantResponse, AppError> {
        // Validate user exists
        let user = User::find_by_id(user_id)
            .one(&app_state.db)
            .await?;
            
        if user.is_none() {
            return Err(AppError::NotFound("User not found".to_string()));
        }

        // Check if user already has a merchant with this name
        let existing_merchant = Merchant::find()
            .filter(merchant::Column::UserId.eq(user_id))
            .filter(merchant::Column::Name.eq(&request.name))
            .one(&app_state.db)
            .await?;

        if existing_merchant.is_some() {
            return Err(AppError::ValidationError(
                "Merchant with this name already exists for this user".to_string()
            ));
        }

        // Generate webhook secret for HMAC signing
        let webhook_secret = Self::generate_webhook_secret();
        
        // Create new merchant
        let merchant_id = Uuid::new_v4().to_string();
        let environment_type = request.environment_type.unwrap_or_default();
        let preferred_networks = request.preferred_networks.unwrap_or_default();

        let new_merchant = merchant::ActiveModel {
            id: Set(merchant_id.clone()),
            user_id: Set(user_id.to_string()),
            name: Set(request.name.clone()),
            description: Set(request.description),
            website_url: Set(request.website_url),
            webhook_url: Set(request.webhook_url),
            webhook_secret: Set(Some(webhook_secret.clone())),
            environment_type: Set(String::from(environment_type.clone())),
            preferred_networks: Set(Some(serde_json::to_string(&preferred_networks).unwrap_or_default())),
            is_active: Set(true),
            created_at: Set(chrono::Utc::now().into()),
            updated_at: Set(Some(chrono::Utc::now().into())),
            token_version: Set(1), // Default token version
        };

        let saved_merchant = new_merchant.insert(&app_state.db).await?;

        println!("✅ Merchant created successfully: {} (ID: {})", saved_merchant.name, saved_merchant.id);

        let preferred_networks = saved_merchant.preferred_networks.as_ref()
            .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
            .unwrap_or_default();

        Ok(MerchantResponse {
            id: saved_merchant.id,
            name: saved_merchant.name,
            description: saved_merchant.description,
            website_url: saved_merchant.website_url,
            webhook_url: saved_merchant.webhook_url,
            environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(saved_merchant.environment_type),
            preferred_networks,
            is_active: saved_merchant.is_active,
            created_at: saved_merchant.created_at.to_rfc3339(),
        })
    }

    /// Get merchant by ID
    /// 
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant_id` - ID of the merchant to retrieve
    /// * `user_id` - ID of the requesting user (for ownership validation)
    /// 
    /// # Returns
    /// * `Result<MerchantResponse, AppError>` - Merchant data or error
    pub async fn get_merchant(
        app_state: &AppState,
        merchant_id: &str,
        user_id: &str,
    ) -> Result<MerchantResponse, AppError> {
        let merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id)) // Ensure user owns this merchant
            .one(&app_state.db)
            .await?;

        match merchant {
            Some(m) => {
                let preferred_networks = m.preferred_networks.as_ref()
                    .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
                    .unwrap_or_default();

                Ok(MerchantResponse {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    website_url: m.website_url,
                    webhook_url: m.webhook_url,
                    environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(m.environment_type),
                    preferred_networks,
                    is_active: m.is_active,
                    created_at: m.created_at.to_rfc3339(),
                })
            },
            None => Err(AppError::NotFound("Merchant not found".to_string())),
        }
    }

    /// Get all merchants for a user with pagination
    /// 
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `user_id` - ID of the user
    /// * `page` - Page number (default: 1)
    /// * `limit` - Items per page (default: 10, max: 100)
    /// 
    /// # Returns
    /// * `Result<(Vec<MerchantResponse>, u64), AppError>` - (merchants, total_count)
    pub async fn get_user_merchants(
        app_state: &AppState,
        user_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<(Vec<MerchantResponse>, u64), AppError> {
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(10).min(100); // Max 100 items per page
        let offset = (page - 1) * limit;

        // Get total count
        let total_count = Merchant::find()
            .filter(merchant::Column::UserId.eq(user_id))
            .count(&app_state.db)
            .await?;

        // Get merchants with pagination
        let merchants = Merchant::find()
            .filter(merchant::Column::UserId.eq(user_id))
            .order_by_desc(merchant::Column::CreatedAt)
            .offset(offset)
            .limit(limit)
            .all(&app_state.db)
            .await?;

        let merchant_responses: Vec<MerchantResponse> = merchants
            .into_iter()
            .map(|m| {
                let preferred_networks = m.preferred_networks.as_ref()
                    .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
                    .unwrap_or_default();

                MerchantResponse {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    website_url: m.website_url,
                    webhook_url: m.webhook_url,
                    environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(m.environment_type),
                    preferred_networks,
                    is_active: m.is_active,
                    created_at: m.created_at.to_rfc3339(),
                }
            })
            .collect();

        Ok((merchant_responses, total_count))
    }

    /// Update merchant information
    /// 
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant_id` - ID of the merchant to update
    /// * `user_id` - ID of the requesting user (for ownership validation)
    /// * `request` - Update request data
    /// 
    /// # Returns
    /// * `Result<MerchantResponse, AppError>` - Updated merchant or error
    pub async fn update_merchant(
        app_state: &AppState,
        merchant_id: &str,
        user_id: &str,
        request: UpdateMerchantRequest,
    ) -> Result<MerchantResponse, AppError> {
        // Find existing merchant
        let existing_merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id))
            .one(&app_state.db)
            .await?;

        let existing_merchant = match existing_merchant {
            Some(m) => m,
            None => return Err(AppError::NotFound("Merchant not found".to_string())),
        };

        // Build update model
        let mut update_model: merchant::ActiveModel = existing_merchant.into();
        
        if let Some(name) = request.name {
            update_model.name = Set(name);
        }
        if let Some(description) = request.description {
            update_model.description = Set(Some(description));
        }
        if let Some(website_url) = request.website_url {
            update_model.website_url = Set(Some(website_url));
        }
        if let Some(webhook_url) = request.webhook_url {
            update_model.webhook_url = Set(Some(webhook_url));
        }
        if let Some(environment_type) = request.environment_type {
            update_model.environment_type = Set(String::from(environment_type));
        }
        if let Some(preferred_networks) = request.preferred_networks {
            update_model.preferred_networks = Set(Some(serde_json::to_string(&preferred_networks).unwrap_or_default()));
        }
        if let Some(is_active) = request.is_active {
            update_model.is_active = Set(is_active);
        }
        
        update_model.updated_at = Set(Some(chrono::Utc::now().into()));

        let updated_merchant = update_model.update(&app_state.db).await?;

        println!("✅ Merchant updated successfully: {} (ID: {})", updated_merchant.name, updated_merchant.id);

        let preferred_networks = updated_merchant.preferred_networks.as_ref()
            .and_then(|pn| serde_json::from_str::<Vec<String>>(pn).ok())
            .unwrap_or_default();

        Ok(MerchantResponse {
            id: updated_merchant.id,
            name: updated_merchant.name,
            description: updated_merchant.description,
            website_url: updated_merchant.website_url,
            webhook_url: updated_merchant.webhook_url,
            environment_type: crate::merchant::models::merchant::MerchantEnvironmentType::from(updated_merchant.environment_type),
            preferred_networks,
            is_active: updated_merchant.is_active,
            created_at: updated_merchant.created_at.to_rfc3339(),
        })
    }

    /// Delete merchant (soft delete by setting is_active to false)
    /// 
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant_id` - ID of the merchant to delete
    /// * `user_id` - ID of the requesting user (for ownership validation)
    /// 
    /// # Returns
    /// * `Result<(), AppError>` - Success or error
    pub async fn delete_merchant(
        app_state: &AppState,
        merchant_id: &str,
        user_id: &str,
    ) -> Result<(), AppError> {
        let existing_merchant = Merchant::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .filter(merchant::Column::UserId.eq(user_id))
            .one(&app_state.db)
            .await?;

        let existing_merchant = match existing_merchant {
            Some(m) => m,
            None => return Err(AppError::NotFound("Merchant not found".to_string())),
        };

        // Soft delete by setting is_active to false
        let mut update_model: merchant::ActiveModel = existing_merchant.into();
        update_model.is_active = Set(false);
        update_model.updated_at = Set(Some(chrono::Utc::now().into()));

        update_model.update(&app_state.db).await?;

        println!("✅ Merchant deactivated successfully: {}", merchant_id);

        Ok(())
    }

    /// Generate a secure webhook secret for HMAC signing
    fn generate_webhook_secret() -> String {
        use rand::{distributions::Alphanumeric, Rng};
        
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(64) // 64 character secret
            .map(char::from)
            .collect()
    }

    /// Validate merchant webhook URL format
    pub fn validate_webhook_url(url: &str) -> Result<(), AppError> {
        if url.is_empty() {
            return Ok(()); // Empty webhook URL is allowed
        }

        if !url.starts_with("https://") {
            return Err(AppError::ValidationError(
                "Webhook URL must use HTTPS for security".to_string()
            ));
        }

        // Basic URL validation
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Webhook URL is too long (max 2048 characters)".to_string()
            ));
        }

        Ok(())
    }

    /// Validate website URL format
    pub fn validate_website_url(url: &str) -> Result<(), AppError> {
        if url.is_empty() {
            return Ok(()); // Empty website URL is allowed
        }

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(AppError::ValidationError(
                "Website URL must start with http:// or https://".to_string()
            ));
        }

        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Website URL is too long (max 2048 characters)".to_string()
            ));
        }

        Ok(())
    }
} 