use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(length(min = 1, message = "Invalid Name"))]
    pub name: String,

    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at 8 characters long"))]
    pub password: String,

    #[validate(length(min = 8, message = "Password should match"))]
    pub confirm_password: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password incorrect"))]
    pub password: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct VerifyEmailRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    pub code: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub status_code: u16,
    pub message: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserClaims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    pub email: String,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct TempUser {
    pub name: String,
    pub email: String,
    pub password: String,
}
