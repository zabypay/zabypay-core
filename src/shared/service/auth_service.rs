use argon2::{Argon2, PasswordHash, PasswordVerifier};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::env;
use uuid::Uuid;

use crate::shared::{
    entities::{api_key, merchant, user},
    models::auth::{AuthContext, EnhancedClaims, TokenType, TokenValidation},
    utils::errors::AppError,
};

pub struct AuthService {
    jwt_secret: String,
    issuer: String,
    access_token_duration: Duration,
    refresh_token_duration: Duration,
}

impl AuthService {
    pub fn new() -> Self {
        Self {
            jwt_secret: env::var("JWT_SECRET").unwrap_or_else(|_| {
                log::warn!("JWT_SECRET not set, using default (INSECURE!)");
                "your-256-bit-secret-change-this-immediately".to_string()
            }),
            issuer: env::var("JWT_ISSUER").unwrap_or_else(|_| "api-crypto".to_string()),
            access_token_duration: Duration::minutes(15), // Short-lived access tokens
            refresh_token_duration: Duration::days(7),    // Longer refresh tokens
        }
    }

    /// Generate access token with enhanced claims
    pub async fn generate_access_token(
        &self,
        db: &DatabaseConnection,
        user_id: &str,
        merchant_id: Option<&str>,
        environment: &str,
        session_id: Option<&str>,
        ip_address: Option<&str>,
    ) -> Result<String, AppError> {
        // Load user from DB to get current token_version
        let user = user::Entity::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("User not found".to_string()))?;

        // Check if user is active
        if !user.emailverified.unwrap_or(false) {
            return Err(AppError::Unauthorized("Email not verified".to_string()));
        }

        // Get merchant token version if applicable
        let mut merchant_token_version = 0;
        if let Some(m_id) = merchant_id {
            let merchant = merchant::Entity::find_by_id(m_id)
                .one(db)
                .await?
                .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

            if !merchant.is_active {
                return Err(AppError::Unauthorized("Merchant is inactive".to_string()));
            }

            merchant_token_version = merchant.token_version;
        }

        let now = Utc::now();
        let exp = now + self.access_token_duration;

        let claims = EnhancedClaims {
            sub: user_id.to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
            nbf: now.timestamp(),
            jti: Uuid::new_v4().to_string(),
            iss: self.issuer.clone(),
            aud: vec!["api".to_string()],
            email: user.email.clone(),
            user_id: user_id.to_string(),
            token_version: user.token_version,
            merchant_id: merchant_id.map(|s| s.to_string()),
            environment: environment.to_string(),
            token_type: TokenType::Access,
            scopes: self.get_user_scopes(&user, merchant_id.is_some()),
            session_id: session_id.map(|s| s.to_string()),
            ip_address: ip_address.map(|s| s.to_string()),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    /// Generate refresh token
    pub async fn generate_refresh_token(
        &self,
        db: &DatabaseConnection,
        user_id: &str,
        session_id: &str,
    ) -> Result<String, AppError> {
        let user = user::Entity::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("User not found".to_string()))?;

        let now = Utc::now();
        let exp = now + self.refresh_token_duration;

        let claims = EnhancedClaims {
            sub: user_id.to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
            nbf: now.timestamp(),
            jti: Uuid::new_v4().to_string(),
            iss: self.issuer.clone(),
            aud: vec!["refresh".to_string()],
            email: user.email.clone(),
            user_id: user_id.to_string(),
            token_version: user.token_version,
            merchant_id: None,
            environment: "".to_string(),
            token_type: TokenType::Refresh,
            scopes: vec![],
            session_id: Some(session_id.to_string()),
            ip_address: None,
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    /// Validate token with DB checks
    pub async fn validate_token(
        &self,
        db: &DatabaseConnection,
        token: &str,
    ) -> Result<AuthContext, AppError> {
        // Decode token
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&self.issuer]);
        validation.validate_exp = true;
        validation.validate_nbf = true;

        let token_data = decode::<EnhancedClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &validation,
        )
        .map_err(|e| {
            log::debug!("Token decode error: {:?}", e);
            AppError::Unauthorized("Invalid token".to_string())
        })?;

        let claims = token_data.claims;

        // Validate user exists and is active
        let user = user::Entity::find_by_id(&claims.user_id)
            .one(db)
            .await?
            .ok_or_else(|| {
                log::warn!("Token validation failed: User {} not found", claims.user_id);
                AppError::Unauthorized("User not found".to_string())
            })?;

        // Check if user is active
        let is_active = user.is_active;
        if !is_active {
            log::warn!(
                "Token validation failed: User {} is inactive",
                claims.user_id
            );
            return Err(AppError::Unauthorized(
                "User account is disabled".to_string(),
            ));
        }

        // Check email verification
        if !user.emailverified.unwrap_or(false) {
            log::warn!(
                "Token validation failed: User {} email not verified",
                claims.user_id
            );
            return Err(AppError::Unauthorized("Email not verified".to_string()));
        }

        // Validate token version matches DB
        let user_token_version = user.token_version;
        if claims.token_version != user_token_version {
            log::warn!(
                "Token validation failed: Version mismatch for user {} (token: {}, db: {})",
                claims.user_id,
                claims.token_version,
                user_token_version
            );
            return Err(AppError::Unauthorized(
                "Token has been invalidated".to_string(),
            ));
        }

        // Validate merchant if present
        if let Some(merchant_id) = &claims.merchant_id {
            let merchant = merchant::Entity::find_by_id(merchant_id)
                .one(db)
                .await?
                .ok_or_else(|| {
                    log::warn!(
                        "Token validation failed: Merchant {} not found",
                        merchant_id
                    );
                    AppError::Unauthorized("Merchant not found".to_string())
                })?;

            // Check merchant is active
            if !merchant.is_active {
                log::warn!(
                    "Token validation failed: Merchant {} is inactive",
                    merchant_id
                );
                return Err(AppError::Unauthorized("Merchant is inactive".to_string()));
            }

            // Check merchant token version
            let merchant_token_version = merchant.token_version;
            // Note: We could track merchant token version separately in claims if needed

            // Validate environment matches
            if merchant.environment_type != claims.environment {
                log::warn!(
                    "Token validation failed: Environment mismatch for merchant {} (token: {}, db: {})",
                    merchant_id, claims.environment, merchant.environment_type
                );
                return Err(AppError::Unauthorized("Environment mismatch".to_string()));
            }
        }

        // Check for refresh token type in wrong context
        if claims.token_type == TokenType::Refresh {
            return Err(AppError::Unauthorized(
                "Cannot use refresh token for API access".to_string(),
            ));
        }

        Ok(AuthContext {
            user_id: claims.user_id,
            email: claims.email,
            merchant_id: claims.merchant_id,
            environment: claims.environment,
            token_version: claims.token_version,
            scopes: claims.scopes,
            api_key_id: None,
            session_id: claims.session_id,
        })
    }

    /// Invalidate all tokens for a user (increment token_version)
    pub async fn invalidate_user_tokens(
        &self,
        db: &DatabaseConnection,
        user_id: &str,
    ) -> Result<(), AppError> {
        let user = user::Entity::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("User not found".to_string()))?;

        let mut user_active: user::ActiveModel = user.into();
        let new_version = user_active.token_version.as_ref() + 1;
        user_active.token_version = Set(new_version);
        user_active.update(db).await?;

        log::info!(
            "Invalidated all tokens for user {} (new version: {})",
            user_id,
            new_version
        );
        Ok(())
    }

    /// Invalidate all tokens for a merchant
    pub async fn invalidate_merchant_tokens(
        &self,
        db: &DatabaseConnection,
        merchant_id: &str,
    ) -> Result<(), AppError> {
        let merchant = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("Merchant not found".to_string()))?;

        let mut merchant_active: merchant::ActiveModel = merchant.into();
        let new_version = merchant_active.token_version.as_ref() + 1;
        merchant_active.token_version = Set(new_version);
        merchant_active.update(db).await?;

        log::info!(
            "Invalidated all tokens for merchant {} (new version: {})",
            merchant_id,
            new_version
        );
        Ok(())
    }

    /// Get user scopes based on role
    fn get_user_scopes(&self, user: &user::Model, has_merchant: bool) -> Vec<String> {
        let mut scopes = vec!["user:read".to_string(), "user:update".to_string()];

        if has_merchant {
            scopes.extend(vec![
                "merchant:read".to_string(),
                "merchant:update".to_string(),
                "payment:create".to_string(),
                "payment:read".to_string(),
                "withdrawal:create".to_string(),
                "withdrawal:read".to_string(),
            ]);
        }

        scopes
    }

    /// Validate API key and signature
    pub async fn validate_api_key_signature(
        &self,
        db: &DatabaseConnection,
        api_key_str: &str,
        signature: &str,
        timestamp: i64,
        nonce: &str,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<(String, String), AppError> {
        // Check timestamp is within acceptable window (5 minutes)
        let now = Utc::now().timestamp();
        let time_diff = (now - timestamp).abs();
        if time_diff > 300 {
            log::warn!(
                "API signature timestamp outside window: {} seconds",
                time_diff
            );
            return Err(AppError::Unauthorized(
                "Request timestamp too old".to_string(),
            ));
        }

        // Find API key by prefix
        let key_prefix = if api_key_str.len() >= 8 {
            &api_key_str[0..8]
        } else {
            return Err(AppError::Unauthorized("Invalid API key format".to_string()));
        };

        let api_key = api_key::Entity::find()
            .filter(api_key::Column::KeyPrefix.eq(key_prefix))
            .filter(api_key::Column::IsActive.eq(true))
            .one(db)
            .await?
            .ok_or_else(|| {
                log::warn!("API key not found with prefix: {}", key_prefix);
                AppError::Unauthorized("Invalid API key".to_string())
            })?;

        // Verify the full API key hash
        if let key_hash = &api_key.key_hash {
            let parsed_hash = PasswordHash::new(key_hash).map_err(|_| {
                AppError::InternalServerError("Invalid key hash format".to_string())
            })?;

            Argon2::default()
                .verify_password(api_key_str.as_bytes(), &parsed_hash)
                .map_err(|_| {
                    log::warn!("API key verification failed for key {}", api_key.id);
                    AppError::Unauthorized("Invalid API key".to_string())
                })?;
        }

        // Check for nonce replay (would need Redis or DB check)
        // For now, we'll store in DB
        use crate::shared::entities::api_key_usage;
        use sea_orm::ActiveValue::NotSet;

        let usage_id = Uuid::new_v4().to_string();
        let usage = api_key_usage::ActiveModel {
            id: Set(usage_id),
            api_key_id: Set(api_key.id.clone()),
            nonce: Set(nonce.to_string()),
            timestamp: Set(timestamp),
            method: Set(method.to_string()),
            path: Set(path.to_string()),
            ip_address: NotSet,
            created_at: Set(Utc::now().into()),
        };

        // Try to insert - will fail if nonce already exists due to unique constraint
        usage.insert(db).await.map_err(|_| {
            log::warn!("Nonce replay detected: {}", nonce);
            AppError::Unauthorized("Nonce already used".to_string())
        })?;

        // Verify HMAC signature
        let secret = api_key
            .secret_key
            .as_ref()
            .or(api_key.encrypted_secret_key.as_ref())
            .ok_or(AppError::InternalServerError(
                "API secret not found".to_string(),
            ))?;

        let message = format!("{}|{}|{}|{}|{}", method, path, body, timestamp, nonce);
        let expected_signature = self.compute_hmac(secret, &message)?;

        if signature != expected_signature {
            log::warn!("HMAC signature mismatch for API key {}", api_key.id);
            return Err(AppError::Unauthorized("Invalid signature".to_string()));
        }

        // Update last used timestamp
        let mut api_key_active: api_key::ActiveModel = api_key.clone().into();
        api_key_active.last_used = Set(Some(Utc::now().into()));
        api_key_active.update(db).await?;

        Ok((api_key.id, api_key.merchant_id))
    }

    /// Compute HMAC-SHA256
    fn compute_hmac(&self, secret: &str, message: &str) -> Result<String, AppError> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|_| AppError::InternalServerError("Invalid HMAC key".to_string()))?;

        mac.update(message.as_bytes());
        let result = mac.finalize();
        let code = result.into_bytes();

        Ok(hex::encode(code))
    }
}
