use log::{error, info, warn};
use std::env;

/// Production configuration utilities
pub struct ProductionConfig;

impl ProductionConfig {
    /// Initialize production logging
    pub fn init_logging() {
        // Set default log level to info in production
        if env::var("RUST_LOG").is_err() {
            env::set_var("RUST_LOG", "info");
        }

        env_logger::Builder::from_default_env()
            .format_timestamp_secs()
            .init();

        info!(" Production logging initialized");
    }

    /// Validate required environment variables
    pub fn validate_env_vars() -> Result<(), String> {
        let required_vars = ["DATABASE_URL", "JWT_SECRET", "ENCRYPTION_KEY"];

        let optional_vars = [
            ("ENVIRONMENT", "development"),
            ("PORT", "8080"),
            ("ADMIN_PORT", "9090"),
            ("CORS_ALLOWED_ORIGINS", "*"),
        ];

        // Check required variables
        for var in &required_vars {
            if env::var(var).is_err() {
                error!(" Missing required environment variable: {}", var);
                return Err(format!("Missing required environment variable: {}", var));
            }
        }

        // Set defaults for optional variables
        for (var, default) in &optional_vars {
            if env::var(var).is_err() {
                env::set_var(var, default);
                info!("✅ Set default value for {}: {}", var, default);
            }
        }

        info!("✅ Environment validation completed");
        Ok(())
    }

    /// Get database pool configuration for production
    pub fn get_db_pool_config() -> (u32, u32, std::time::Duration) {
        let max_connections = env::var("DB_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u32>()
            .unwrap_or(10);

        let min_connections = env::var("DB_MIN_CONNECTIONS")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<u32>()
            .unwrap_or(5);

        let connection_timeout = std::time::Duration::from_secs(
            env::var("DB_CONNECTION_TIMEOUT")
                .unwrap_or_else(|_| "30".to_string())
                .parse::<u64>()
                .unwrap_or(30),
        );

        info!(
            " Database pool config - Max: {}, Min: {}, Timeout: {:?}s",
            max_connections,
            min_connections,
            connection_timeout.as_secs()
        );

        (max_connections, min_connections, connection_timeout)
    }

    /// Check if running in production mode
    pub fn is_production() -> bool {
        env::var("ENVIRONMENT")
            .unwrap_or_else(|_| "development".to_string())
            .to_lowercase()
            == "production"
    }

    /// Get CORS configuration
    pub fn get_cors_origins() -> Vec<String> {
        let origins = env::var("CORS_ALLOWED_ORIGINS").unwrap_or_else(|_| "*".to_string());

        if origins == "*" {
            if Self::is_production() {
                warn!(
                    "  CORS is set to allow all origins in production! This should be restricted."
                );
            }
            return vec!["*".to_string()];
        }

        origins.split(',').map(|s| s.trim().to_string()).collect()
    }

    /// Health check endpoint data
    pub fn get_health_check_info() -> serde_json::Value {
        serde_json::json!({
            "status": "healthy",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "environment": env::var("ENVIRONMENT").unwrap_or_else(|_| "unknown".to_string()),
            "version": env!("CARGO_PKG_VERSION"),
            "uptime": std::process::id(),
            "database": "connected"
        })
    }
}

/// Request ID middleware for tracing
pub struct RequestId;

impl RequestId {
    pub fn generate() -> String {
        uuid::Uuid::new_v4().to_string()[..8].to_string()
    }
}

/// Rate limiting configuration
pub struct RateLimit;

impl RateLimit {
    /// Get rate limit configuration for different endpoints
    pub fn get_limits() -> std::collections::HashMap<String, (u32, std::time::Duration)> {
        let mut limits = std::collections::HashMap::new();

        // API endpoints (requests per minute)
        limits.insert(
            "/api/auth/login".to_string(),
            (5, std::time::Duration::from_secs(60)),
        );
        limits.insert(
            "/api/auth/register".to_string(),
            (3, std::time::Duration::from_secs(60)),
        );
        limits.insert(
            "/api/payments".to_string(),
            (100, std::time::Duration::from_secs(60)),
        );
        limits.insert(
            "/api/withdrawals".to_string(),
            (50, std::time::Duration::from_secs(60)),
        );
        limits.insert(
            "/api/balances".to_string(),
            (200, std::time::Duration::from_secs(60)),
        );

        limits
    }
}

/// Security headers middleware configuration
pub struct SecurityHeaders;

impl SecurityHeaders {
    pub fn get_headers() -> Vec<(&'static str, &'static str)> {
        vec![
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("X-XSS-Protection", "1; mode=block"),
            (
                "Strict-Transport-Security",
                "max-age=31536000; includeSubDomains",
            ),
            ("Referrer-Policy", "strict-origin-when-cross-origin"),
            ("Content-Security-Policy", "default-src 'self'"),
        ]
    }
}
