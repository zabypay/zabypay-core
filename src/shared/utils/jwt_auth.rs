use crate::shared::models::temp_auth::UserClaims;
use crate::shared::utils::constants::JWT_SECRET;
use crate::shared::utils::errors::AppError;
use actix_web::HttpRequest;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

pub fn decode_token(token: &str) -> Result<UserClaims, AppError> {
    let key = DecodingKey::from_secret(JWT_SECRET.as_ref());
    let validation = Validation::new(Algorithm::HS256);

    match decode::<UserClaims>(token, &key, &validation) {
        Ok(token_data) => Ok(token_data.claims),
        Err(_) => Err(AppError::Unauthorized("Unauthorized".to_string())),
    }
}

pub fn extract_token_from_header(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("Authorization")
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_str| {
            if auth_str.starts_with("Bearer ") {
                Some(auth_str[7..].to_string())
            } else {
                None
            }
        })
}

pub fn extract_token_from_cookie(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("Cookie")
        .and_then(|cookie_header| cookie_header.to_str().ok())
        .and_then(|cookie_str| {
            cookie_str
                .split(';')
                .find(|cookie| cookie.trim().starts_with("token="))
                .and_then(|token_cookie| {
                    token_cookie
                        .split('=')
                        .nth(1)
                        .map(|token| token.trim().to_string())
                })
        })
}
