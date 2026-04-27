use crate::client::models::auth::{LoginRequest, RegisterRequest, UserClaims};
use crate::shared::entities::user;
use crate::shared::utils::constants;
use crate::shared::utils::errors::AppError;
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

pub struct AuthService;

impl AuthService {
    pub async fn register_user(
        db: &DatabaseConnection,
        request: RegisterRequest,
    ) -> Result<(), AppError> {
        if request.password != request.confirm_password {
            return Err(AppError::ValidationError(
                "Passwords do not match".to_string(),
            ));
        }

        let existing_user = user::Entity::find()
            .filter(user::Column::Email.eq(&request.email))
            .one(db)
            .await
            .map_err(AppError::from)?;
        if existing_user.is_some() {
            return Err(AppError::BadRequest("User already exists".to_string()));
        }

        let hashed_password = hash(request.password, DEFAULT_COST).map_err(|e| {
            AppError::InternalServerError(format!("Password hashing failed: {}", e))
        })?;

        let new_user = user::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            name: Set(request.name),
            email: Set(request.email.clone()),
            password: Set(hashed_password),
            emailverified: Set(Some(true)),
            verifiedcode: Set(Some(String::new())),
            is_active: Set(true),
            last_password_change: Set(None),
            token_version: Set(1),
            created_at: Set(chrono::Utc::now().into()),
        };

        user::Entity::insert(new_user)
            .exec(db)
            .await
            .map_err(AppError::from)?;

        log::info!("Registered user: {}", request.email);
        Ok(())
    }

    pub async fn login_user(
        db: &DatabaseConnection,
        request: LoginRequest,
    ) -> Result<String, AppError> {
        let user_record = user::Entity::find()
            .filter(user::Column::Email.eq(&request.email))
            .one(db)
            .await
            .map_err(AppError::from)?
            .ok_or(AppError::BadRequest("Invalid credentials".to_string()))?;

        if !verify(&request.password, &user_record.password).map_err(|e| {
            AppError::InternalServerError(format!("Password verification error: {}", e))
        })? {
            return Err(AppError::BadRequest("Invalid credentials".to_string()));
        }

        Self::generate_jwt_token(user_record.email, user_record.id)
    }

    pub async fn test_login_user(
        db: &DatabaseConnection,
        request: LoginRequest,
    ) -> Result<user::Model, AppError> {
        let user_record = user::Entity::find()
            .filter(user::Column::Email.eq(&request.email))
            .one(db)
            .await
            .map_err(AppError::from)?
            .ok_or(AppError::BadRequest("Invalid credentials".to_string()))?;

        if !verify(&request.password, &user_record.password).map_err(|e| {
            AppError::InternalServerError(format!("Password verification error: {}", e))
        })? {
            return Err(AppError::BadRequest("Invalid credentials".to_string()));
        }

        Ok(user_record)
    }

    pub fn generate_jwt_token(email: String, id: String) -> Result<String, AppError> {
        let secret: String = (*constants::JWT_SECRET).clone();
        let now = chrono::Utc::now();
        let expire = chrono::Duration::hours(24);
        let exp = (now + expire).timestamp() as usize;
        let iat = now.timestamp() as usize;

        let claim = UserClaims {
            sub: email.clone(),
            exp,
            iat,
            email,
            id,
        };

        encode(
            &Header::default(),
            &claim,
            &EncodingKey::from_secret(secret.as_ref()),
        )
        .map_err(|e| AppError::InternalServerError(format!("JWT encoding error: {}", e)))
    }

    pub fn decode_jwt_token(token: &str) -> Result<TokenData<UserClaims>, AppError> {
        let secret: String = (*constants::JWT_SECRET).clone();
        decode::<UserClaims>(
            token,
            &DecodingKey::from_secret(secret.as_ref()),
            &Validation::new(Algorithm::HS256),
        )
        .map_err(|e| {
            println!("🔐 JWT decode error: {:?}", e);
            AppError::InternalServerError(format!("JWT decode error: {}", e))
        })
    }

    pub fn generate_tokens(user_claims: &UserClaims) -> Result<String, AppError> {
        let secret: String = (*constants::JWT_SECRET).clone();
        encode(
            &Header::default(),
            user_claims,
            &EncodingKey::from_secret(secret.as_ref()),
        )
        .map_err(|_| AppError::InternalServerError("Token generation failed".to_string()))
    }

    pub fn validate_token(token: &str) -> Result<UserClaims, AppError> {
        let secret: String = (*constants::JWT_SECRET).clone();
        let token_data = decode::<UserClaims>(
            token,
            &DecodingKey::from_secret(secret.as_ref()),
            &Validation::default(),
        )
        .map_err(|_| AppError::Unauthorized("Unauthorized".to_string()))?;
        Ok(token_data.claims)
    }
}
