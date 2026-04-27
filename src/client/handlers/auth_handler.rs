use crate::client::models::auth::{LoginRequest, RegisterRequest};
use crate::client::services::auth_service::AuthService;
use crate::shared::utils::errors::AppError;
use crate::shared::AppState;
use actix_web::{post, web, HttpResponse};
use validator::Validate;

#[post("/register")]
pub async fn register(
    app_state: web::Data<AppState>,
    body: web::Json<RegisterRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let request_data = body.into_inner();
    let user_email = request_data.email.clone();

    AuthService::register_user(&app_state.db, request_data).await?;

    log::info!("User registered: {}", user_email);

    Ok(HttpResponse::Created().json(serde_json::json!({
        "status_code": 201,
        "message": "User registered successfully. You can log in immediately.",
        "success": true
    })))
}

#[post("/login")]
pub async fn login(
    app_state: web::Data<AppState>,
    body: web::Json<LoginRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let token = AuthService::login_user(&app_state.db, body.into_inner()).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "token": token,
        "message": "Login successful",
        "success": true,
        "status_code": 200
    })))
}

// Kept for backward compatibility — same behaviour as `/login` now that
// email verification has been removed.
#[post("/test-login")]
pub async fn test_login(
    app_state: web::Data<AppState>,
    body: web::Json<LoginRequest>,
) -> Result<HttpResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let request_data = body.into_inner();
    let user = AuthService::test_login_user(&app_state.db, request_data).await?;
    let token = AuthService::generate_jwt_token(user.email.clone(), user.id.clone())?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "token": token,
        "message": "Test login successful!",
        "success": true,
        "status_code": 200
    })))
}
