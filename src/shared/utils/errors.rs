use actix_web::{HttpResponse, ResponseError};
use sea_orm::DbErr;
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    // Database related errors
    DatabaseError(String),
    NotFound(String),
    Conflict(String),

    // Authentication related errors
    Unauthorised,
    Unauthorized(String),
    TokenExpired,
    InvalidToken,

    // Validation related errors
    ValidationError(String),
    InvalidInput(String),
    BadRequest(String),
    Forbidden(String),

    // Server related errors
    InternalServerError(String),
    ServiceUnavailable(String),

    // External service errors
    ExternalServiceError(String),

    // Database connection errors
    DbConnectionError,

    // Password related errors
    BcryptError,

    // Email verification errors
    VerificationFailed,
    EmailSendError,

    // JWT related errors
    JwtCreationError(String),
    JwtValidationError(String),

    // Hashing errors
    HashingError,

    // Transaction/Wallet related errors
    InsufficientBalance(String),
    TransactionFailed(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            AppError::NotFound(msg) => write!(f, "Not found: {}", msg),
            AppError::Conflict(msg) => write!(f, "Conflict: {}", msg),
            AppError::Unauthorised => write!(f, "Unauthorised"),
            AppError::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
            AppError::TokenExpired => write!(f, "Token expired"),
            AppError::InvalidToken => write!(f, "Invalid token"),
            AppError::ValidationError(msg) => write!(f, "Validation error: {}", msg),
            AppError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            AppError::BadRequest(msg) => write!(f, "Bad request: {}", msg),
            AppError::Forbidden(msg) => write!(f, "Forbidden: {}", msg),
            AppError::InternalServerError(msg) => write!(f, "Internal server error: {}", msg),
            AppError::ServiceUnavailable(msg) => write!(f, "Service unavailable: {}", msg),
            AppError::ExternalServiceError(msg) => write!(f, "External service error: {}", msg),
            AppError::DbConnectionError => write!(f, "Database connection error"),
            AppError::BcryptError => write!(f, "Bcrypt error"),
            AppError::VerificationFailed => write!(f, "Verification failed"),
            AppError::EmailSendError => write!(f, "Email send error"),
            AppError::JwtCreationError(msg) => write!(f, "JWT creation error: {}", msg),
            AppError::JwtValidationError(msg) => write!(f, "JWT validation error: {}", msg),
            AppError::HashingError => write!(f, "Hashing error"),
            AppError::InsufficientBalance(msg) => write!(f, "Insufficient balance: {}", msg),
            AppError::TransactionFailed(msg) => write!(f, "Transaction failed: {}", msg),
        }
    }
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        match self {
            AppError::DatabaseError(msg) => {
                // Extract meaningful information from database errors
                if msg.contains("SQLSTATE: 28P01") || msg.contains("password authentication failed")
                {
                    HttpResponse::InternalServerError()
                        .json("Database connection failed - please check server configuration")
                } else if msg.contains("database") && msg.contains("does not exist") {
                    HttpResponse::InternalServerError()
                        .json("Database setup required - please run migrations")
                } else if msg.contains("connection") || msg.contains("refused") {
                    HttpResponse::InternalServerError()
                        .json("Database server unavailable - please check if PostgreSQL is running")
                } else if msg.contains("foreign key constraint") {
                    HttpResponse::BadRequest()
                        .json("Invalid data reference - related record not found")
                } else if msg.contains("unique constraint") {
                    HttpResponse::Conflict().json("Duplicate entry - record already exists")
                } else {
                    // For other database errors, show a generic but helpful message
                    HttpResponse::InternalServerError()
                        .json("Database operation failed - please try again")
                }
            }
            AppError::NotFound(msg) => HttpResponse::NotFound().json(msg),
            AppError::Conflict(msg) => HttpResponse::Conflict().json(msg),
            AppError::Unauthorised => HttpResponse::Unauthorized().json("Unauthorised"),
            AppError::Unauthorized(msg) => HttpResponse::Unauthorized().json(msg),
            AppError::TokenExpired => HttpResponse::Unauthorized().json("Token expired"),
            AppError::InvalidToken => HttpResponse::Unauthorized().json("Invalid token"),
            AppError::ValidationError(msg) => HttpResponse::BadRequest().json(msg),
            AppError::InvalidInput(msg) => HttpResponse::BadRequest().json(msg),
            AppError::BadRequest(msg) => HttpResponse::BadRequest().json(msg),
            AppError::Forbidden(msg) => HttpResponse::Forbidden().json(msg),
            AppError::InternalServerError(msg) => {
                // Show helpful internal server error messages when appropriate
                if msg.contains("wallet") || msg.contains("address") {
                    HttpResponse::InternalServerError()
                        .json(format!("Wallet generation failed: {}", msg))
                } else if msg.contains("currency") {
                    HttpResponse::BadRequest().json(format!("Currency error: {}", msg))
                } else if msg.contains("Failed to prepare wallet") {
                    HttpResponse::InternalServerError()
                        .json("Failed to generate payment wallet - please try again")
                } else {
                    HttpResponse::InternalServerError()
                        .json("Internal server error - please try again")
                }
            }
            AppError::ServiceUnavailable(msg) => HttpResponse::ServiceUnavailable().json(msg),
            AppError::ExternalServiceError(_) => {
                HttpResponse::BadGateway().json("External service error")
            }
            AppError::DbConnectionError => {
                HttpResponse::InternalServerError().json("Database connection error")
            }
            AppError::BcryptError => {
                HttpResponse::InternalServerError().json("Password hashing error")
            }
            AppError::VerificationFailed => HttpResponse::BadRequest().json("Verification failed"),
            AppError::EmailSendError => {
                HttpResponse::InternalServerError().json("Email send error")
            }
            AppError::JwtCreationError(_) => {
                HttpResponse::InternalServerError().json("JWT creation error")
            }
            AppError::JwtValidationError(_) => {
                HttpResponse::Unauthorized().json("JWT validation error")
            }
            AppError::HashingError => HttpResponse::InternalServerError().json("Hashing error"),
            AppError::InsufficientBalance(msg) => HttpResponse::BadRequest().json(msg),
            AppError::TransactionFailed(msg) => HttpResponse::InternalServerError().json(msg),
        }
    }
}

impl From<DbErr> for AppError {
    fn from(err: DbErr) -> Self {
        AppError::DatabaseError(err.to_string())
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(err: validator::ValidationErrors) -> Self {
        let mut error_messages = Vec::new();
        for (field, errors) in err.field_errors() {
            for error in errors {
                if let Some(message) = &error.message {
                    error_messages.push(format!("{}: {}", field, message));
                } else {
                    error_messages.push(format!("{}: validation failed", field));
                }
            }
        }
        AppError::ValidationError(error_messages.join(", "))
    }
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        AppError::JwtCreationError(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::InternalServerError(format!("JSON error: {}", err))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::ExternalServiceError(format!("HTTP request error: {}", err))
    }
}
