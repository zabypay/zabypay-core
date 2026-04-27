use crate::shared::utils::errors::AppError;
use redis::{Commands, Connection};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct RateLimiter {
    redis_client: redis::Client,
}

impl RateLimiter {
    pub fn new(redis_url: &str) -> Result<Self, AppError> {
        let client = redis::Client::open(redis_url).map_err(|e| {
            AppError::InternalServerError(format!("Failed to connect to Redis: {}", e))
        })?;

        Ok(Self {
            redis_client: client,
        })
    }

    /// Check if a request should be rate limited
    /// Returns Ok(true) if request is allowed, Ok(false) if rate limited
    pub async fn check_rate_limit(
        &self,
        key: &str,
        max_requests: u32,
        window_seconds: u64,
    ) -> Result<bool, AppError> {
        let mut conn = self
            .redis_client
            .get_connection()
            .map_err(|e| AppError::InternalServerError(format!("Redis connection error: {}", e)))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let window_start = now - window_seconds;
        let redis_key = format!("rate_limit:{}", key);

        // Use sliding window log approach
        // Remove old entries
        let _: () = conn
            .zrembyscore(&redis_key, "-inf", window_start as f64)
            .map_err(|e| {
                AppError::InternalServerError(format!("Redis zrembyscore error: {}", e))
            })?;

        // Count current entries
        let count: u32 = conn
            .zcard(&redis_key)
            .map_err(|e| AppError::InternalServerError(format!("Redis zcard error: {}", e)))?;

        if count >= max_requests {
            log::warn!(
                "Rate limit exceeded for key: {} (count: {}, max: {}, window: {}s)",
                key,
                count,
                max_requests,
                window_seconds
            );
            return Ok(false);
        }

        // Add current request
        let _: () = conn
            .zadd(&redis_key, now as f64, now)
            .map_err(|e| AppError::InternalServerError(format!("Redis zadd error: {}", e)))?;

        // Set expiration to window size + some buffer
        let _: () = conn
            .expire(&redis_key, (window_seconds + 60) as i64)
            .map_err(|e| AppError::InternalServerError(format!("Redis expire error: {}", e)))?;

        Ok(true)
    }

    /// Check API key rate limits with different tiers
    pub async fn check_api_key_rate_limit(
        &self,
        api_key_id: &str,
        merchant_id: &str,
        environment: &str,
    ) -> Result<bool, AppError> {
        // Different rate limits for different contexts
        let api_key_limit = self
            .check_rate_limit(
                &format!("api_key:{}:{}", environment, api_key_id),
                100, // 100 requests per minute for API key
                60,
            )
            .await?;

        if !api_key_limit {
            return Ok(false);
        }

        // Merchant-level rate limiting (broader limit)
        let merchant_limit = self
            .check_rate_limit(
                &format!("merchant:{}:{}", environment, merchant_id),
                1000, // 1000 requests per minute for merchant
                60,
            )
            .await?;

        Ok(merchant_limit)
    }

    /// Check JWT token rate limits (more generous for authenticated users)
    pub async fn check_jwt_rate_limit(
        &self,
        user_id: &str,
        merchant_id: Option<&str>,
        environment: &str,
    ) -> Result<bool, AppError> {
        // User-level rate limiting
        let user_key = format!(
            "jwt:{}:{}:{}",
            environment,
            user_id,
            merchant_id.unwrap_or("none")
        );

        let user_limit = self
            .check_rate_limit(
                &user_key, 500, // 500 requests per minute for authenticated users
                60,
            )
            .await?;

        Ok(user_limit)
    }

    /// Get remaining requests for a key
    pub async fn get_remaining_requests(
        &self,
        key: &str,
        max_requests: u32,
        window_seconds: u64,
    ) -> Result<u32, AppError> {
        let mut conn = self
            .redis_client
            .get_connection()
            .map_err(|e| AppError::InternalServerError(format!("Redis connection error: {}", e)))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let window_start = now - window_seconds;
        let redis_key = format!("rate_limit:{}", key);

        // Remove old entries
        let _: () = conn
            .zrembyscore(&redis_key, "-inf", window_start as f64)
            .map_err(|e| {
                AppError::InternalServerError(format!("Redis zrembyscore error: {}", e))
            })?;

        // Count current entries
        let count: u32 = conn
            .zcard(&redis_key)
            .map_err(|e| AppError::InternalServerError(format!("Redis zcard error: {}", e)))?;

        Ok(max_requests.saturating_sub(count))
    }

    /// Check if an IP should be rate limited (basic DDoS protection)
    pub async fn check_ip_rate_limit(&self, ip: &str) -> Result<bool, AppError> {
        self.check_rate_limit(
            &format!("ip:{}", ip),
            1000, // 1000 requests per minute per IP
            60,
        )
        .await
    }
}
