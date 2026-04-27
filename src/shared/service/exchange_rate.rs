use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};
use crate::shared::utils::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeRate {
    pub from_currency: String,
    pub to_currency: String,
    pub rate: f64,
    pub timestamp: DateTime<Utc>,
    pub provider: String,
    pub volume_24h: Option<f64>,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLock {
    pub id: String,
    pub from_currency: String,
    pub to_currency: String,
    pub rate: f64,
    pub amount: f64,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait RateProvider: Send + Sync {
    async fn get_rate(&self, from: &str, to: &str) -> Result<ExchangeRate, AppError>;
    async fn get_rates_bulk(&self, pairs: &[(String, String)]) -> Result<Vec<ExchangeRate>, AppError>;
    fn get_provider_name(&self) -> &str;
}

pub struct ExchangeRateService {
    providers: Vec<Box<dyn RateProvider>>,
    rate_cache: Arc<RwLock<HashMap<String, ExchangeRate>>>,
    rate_locks: Arc<RwLock<HashMap<String, RateLock>>>,
}

impl ExchangeRateService {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            rate_cache: Arc::new(RwLock::new(HashMap::new())),
            rate_locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn RateProvider>) {
        self.providers.push(provider);
    }

    pub async fn get_best_rate(&self, from: &str, to: &str) -> Result<ExchangeRate, AppError> {
        let cache_key = format!("{}:{}", from.to_uppercase(), to.to_uppercase());
        
        // Check cache first
        {
            let cache = self.rate_cache.read().await;
            if let Some(cached_rate) = cache.get(&cache_key) {
                // Check if cache is still valid (less than 30 seconds old)
                if Utc::now().signed_duration_since(cached_rate.timestamp).num_seconds() < 30 {
                    return Ok(cached_rate.clone());
                }
            }
        }

        // Fetch from all providers and get the best rate
        let mut rates = Vec::new();
        for provider in &self.providers {
            match provider.get_rate(from, to).await {
                Ok(rate) => rates.push(rate),
                Err(e) => {
                    log::warn!("Failed to get rate from {}: {}", provider.get_provider_name(), e);
                }
            }
        }

        if rates.is_empty() {
            return Err(AppError::InternalServerError(
                "No exchange rate providers available".to_string()
            ));
        }

        // Get the best rate (highest rate for crypto-to-fiat, lowest for fiat-to-crypto)
        let best_rate = if from.to_uppercase() == "USD" || from.to_uppercase() == "EUR" {
            // Fiat to crypto - get highest rate
            rates.into_iter().max_by(|a, b| a.rate.partial_cmp(&b.rate).unwrap_or(std::cmp::Ordering::Equal))
        } else {
            // Crypto to fiat - get lowest rate
            rates.into_iter().min_by(|a, b| a.rate.partial_cmp(&b.rate).unwrap_or(std::cmp::Ordering::Equal))
        }.ok_or(AppError::InternalServerError("No valid rates found".to_string()))?;

        // Cache the rate
        {
            let mut cache = self.rate_cache.write().await;
            cache.insert(cache_key, best_rate.clone());
        }

        Ok(best_rate)
    }

    pub async fn lock_rate(&self, from: &str, to: &str, amount: f64, lock_duration_seconds: i64) -> Result<RateLock, AppError> {
        let rate = self.get_best_rate(from, to).await?;
        
        let lock_id = uuid::Uuid::new_v4().to_string();
        let expires_at = Utc::now() + chrono::Duration::seconds(lock_duration_seconds);
        
        let rate_lock = RateLock {
            id: lock_id.clone(),
            from_currency: from.to_string(),
            to_currency: to.to_string(),
            rate: rate.rate,
            amount,
            expires_at,
            created_at: Utc::now(),
        };

        {
            let mut locks = self.rate_locks.write().await;
            locks.insert(lock_id.clone(), rate_lock.clone());
        }

        Ok(rate_lock)
    }

    pub async fn get_locked_rate(&self, lock_id: &str) -> Result<RateLock, AppError> {
        let locks = self.rate_locks.read().await;
        let rate_lock = locks.get(lock_id)
            .ok_or(AppError::NotFound("Rate lock not found".to_string()))?;

        if rate_lock.expires_at < Utc::now() {
            return Err(AppError::ValidationError("Rate lock has expired".to_string()));
        }

        Ok(rate_lock.clone())
    }

    pub async fn cleanup_expired_locks(&self) -> Result<u64, AppError> {
        let now = Utc::now();
        let mut locks = self.rate_locks.write().await;
        
        let initial_count = locks.len();
        locks.retain(|_, lock| lock.expires_at > now);
        
        Ok((initial_count - locks.len()) as u64)
    }
}

// Kraken Provider Implementation
pub struct KrakenProvider {
    base_url: String,
    client: reqwest::Client,
}

impl KrakenProvider {
    pub fn new() -> Self {
        Self {
            base_url: "https://api.kraken.com".to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl RateProvider for KrakenProvider {
    async fn get_rate(&self, from: &str, to: &str) -> Result<ExchangeRate, AppError> {
        let pair = format!("{}{}", from.to_uppercase(), to.to_uppercase());
        let url = format!("{}/0/public/Ticker?pair={}", self.base_url, pair);
        
        let response = self.client.get(&url).send().await
            .map_err(|e| AppError::InternalServerError(format!("Kraken API error: {}", e)))?;
        
        let data: serde_json::Value = response.json().await
            .map_err(|e| AppError::InternalServerError(format!("Failed to parse Kraken response: {}", e)))?;
        
        if let Some(result) = data.get("result") {
            if let Some(pair_data) = result.get(&pair) {
                let price = pair_data.get("c")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.get(0))
                    .and_then(|p| p.as_str())
                    .ok_or(AppError::InternalServerError("Invalid Kraken response format".to_string()))?;
                
                let rate: f64 = price.parse()
                    .map_err(|_| AppError::InternalServerError("Invalid rate format".to_string()))?;
                
                return Ok(ExchangeRate {
                    from_currency: from.to_string(),
                    to_currency: to.to_string(),
                    rate,
                    timestamp: Utc::now(),
                    provider: "kraken".to_string(),
                    volume_24h: None,
                    last_updated: Utc::now(),
                });
            }
        }
        
        Err(AppError::InternalServerError("Failed to extract rate from Kraken response".to_string()))
    }

    async fn get_rates_bulk(&self, pairs: &[(String, String)]) -> Result<Vec<ExchangeRate>, AppError> {
        let mut rates = Vec::new();
        for (from, to) in pairs {
            match self.get_rate(from, to).await {
                Ok(rate) => rates.push(rate),
                Err(e) => {
                    log::warn!("Failed to get rate for {}/{}: {}", from, to, e);
                }
            }
        }
        Ok(rates)
    }

    fn get_provider_name(&self) -> &str {
        "kraken"
    }
}

// Binance Provider Implementation
pub struct BinanceProvider {
    base_url: String,
    client: reqwest::Client,
}

impl BinanceProvider {
    pub fn new() -> Self {
        Self {
            base_url: "https://api.binance.com".to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl RateProvider for BinanceProvider {
    async fn get_rate(&self, from: &str, to: &str) -> Result<ExchangeRate, AppError> {
        let symbol = format!("{}{}", from.to_uppercase(), to.to_uppercase());
        let url = format!("{}/api/v3/ticker/price?symbol={}", self.base_url, symbol);
        
        let response = self.client.get(&url).send().await
            .map_err(|e| AppError::InternalServerError(format!("Binance API error: {}", e)))?;
        
        let data: serde_json::Value = response.json().await
            .map_err(|e| AppError::InternalServerError(format!("Failed to parse Binance response: {}", e)))?;
        
        if let Some(price) = data.get("price").and_then(|p| p.as_str()) {
            let rate: f64 = price.parse()
                .map_err(|_| AppError::InternalServerError("Invalid rate format".to_string()))?;
            
            return Ok(ExchangeRate {
                from_currency: from.to_string(),
                to_currency: to.to_string(),
                rate,
                timestamp: Utc::now(),
                provider: "binance".to_string(),
                volume_24h: None,
                last_updated: Utc::now(),
            });
        }
        
        Err(AppError::InternalServerError("Failed to extract rate from Binance response".to_string()))
    }

    async fn get_rates_bulk(&self, pairs: &[(String, String)]) -> Result<Vec<ExchangeRate>, AppError> {
        let mut rates = Vec::new();
        for (from, to) in pairs {
            match self.get_rate(from, to).await {
                Ok(rate) => rates.push(rate),
                Err(e) => {
                    log::warn!("Failed to get rate for {}/{}: {}", from, to, e);
                }
            }
        }
        Ok(rates)
    }

    fn get_provider_name(&self) -> &str {
        "binance"
    }
} 