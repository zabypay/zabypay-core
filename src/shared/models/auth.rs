use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Enhanced JWT claims with security fields
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnhancedClaims {
    // Standard claims
    pub sub: String,      // Subject (user_id)
    pub exp: i64,         // Expiration time
    pub iat: i64,         // Issued at
    pub nbf: i64,         // Not before
    pub jti: String,      // JWT ID (unique identifier)
    pub iss: String,      // Issuer
    pub aud: Vec<String>, // Audience

    // User info
    pub email: String,
    pub user_id: String,

    // Security fields
    pub token_version: i32, // Must match DB token_version
    pub merchant_id: Option<String>,
    pub environment: String,   // "mainnet" or "testnet"
    pub token_type: TokenType, // "access" or "refresh"
    pub scopes: Vec<String>,   // Permissions/scopes

    // Session info
    pub session_id: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    Access,
    Refresh,
}

/// Token validation result with detailed info
#[derive(Debug)]
pub struct TokenValidation {
    pub is_valid: bool,
    pub user_id: Option<String>,
    pub merchant_id: Option<String>,
    pub environment: Option<String>,
    pub reason: Option<String>,
}

/// API key signature request
#[derive(Debug, Deserialize, Clone)]
pub struct ApiKeySignature {
    pub api_key: String,
    pub signature: String,
    pub timestamp: i64,
    pub nonce: String,
}

/// Session information
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub user_id: String,
    pub refresh_token: String,
    pub access_token_jti: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub is_active: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Auth context passed through middleware
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub email: String,
    pub merchant_id: Option<String>,
    pub environment: String,
    pub token_version: i32,
    pub scopes: Vec<String>,
    pub api_key_id: Option<String>,
    pub session_id: Option<String>,
}
