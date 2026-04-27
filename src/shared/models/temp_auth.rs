use serde::{Deserialize, Serialize};

/// Temporary UserClaims type to replace client module dependency
/// This maintains compatibility while client module is disabled
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserClaims {
    pub sub: String,
    pub email: String,
    pub id: String,
    pub exp: usize,
    pub iat: usize,
    pub merchant_id: Option<String>,
}

/// Temporary TempUser type to replace client module dependency
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TempUser {
    pub id: String,
    pub name: String,
    pub email: String,
    pub password: String,
    pub verification_code: String,
}
