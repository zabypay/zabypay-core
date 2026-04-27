use chrono::{DateTime, Utc};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use crate::shared::utils::errors::AppError;

/// Price oracle service for cryptocurrency USD conversions
pub struct PriceOracle {
    cache: Arc<RwLock<HashMap<String, CachedPrice>>>,
    cache_duration: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPrice {
    price: Decimal,
    last_updated: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CoinGeckoSimplePrice {
    #[serde(flatten)]
    prices: HashMap<String, HashMap<String, f64>>,
}

impl PriceOracle {
    /// Create a new price oracle with default cache duration of 5 minutes
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_duration: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Create a new price oracle with custom cache duration
    pub fn with_cache_duration(cache_seconds: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_duration: Duration::from_secs(cache_seconds),
        }
    }

    /// Get USD price for a cryptocurrency with caching
    pub async fn get_usd_price(&self, currency: &str) -> Result<Decimal, AppError> {
        let currency_key = currency.to_lowercase();

        // For stablecoins, always return exactly 1.00 USD (they're pegged to USD)
        if self.is_stablecoin(&currency_key) {
            log::debug!("Using fixed price for stablecoin {}: $1.00", currency);
            return Ok(Decimal::ONE);
        }

        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached_price) = cache.get(&currency_key) {
                let cache_age = Utc::now() - cached_price.last_updated;
                if cache_age.to_std().unwrap_or(Duration::MAX) < self.cache_duration {
                    log::debug!(
                        "Using cached price for {}: ${}",
                        currency,
                        cached_price.price
                    );
                    return Ok(cached_price.price);
                }
            }
        }

        // Cache miss or expired, fetch fresh price
        log::info!("Fetching fresh USD price for {}", currency);
        let fresh_price = self.fetch_price_from_api(&currency_key).await?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(
                currency_key.clone(),
                CachedPrice {
                    price: fresh_price,
                    last_updated: Utc::now(),
                },
            );
        }

        log::info!("Updated price for {}: ${}", currency, fresh_price);
        Ok(fresh_price)
    }

    /// Get multiple USD prices at once for efficiency
    pub async fn get_multiple_usd_prices(
        &self,
        currencies: &[&str],
    ) -> Result<HashMap<String, Decimal>, AppError> {
        let mut results = HashMap::new();
        let mut currencies_to_fetch = Vec::new();

        // Check cache for all currencies first
        {
            let cache = self.cache.read().await;
            for currency in currencies {
                let currency_key = currency.to_lowercase();

                // For stablecoins, always return exactly 1.00 USD
                if self.is_stablecoin(&currency_key) {
                    log::debug!("Using fixed price for stablecoin {}: $1.00", currency);
                    results.insert(currency_key, Decimal::ONE);
                    continue;
                }

                if let Some(cached_price) = cache.get(&currency_key) {
                    let cache_age = Utc::now() - cached_price.last_updated;
                    if cache_age.to_std().unwrap_or(Duration::MAX) < self.cache_duration {
                        results.insert(currency_key, cached_price.price);
                        continue;
                    }
                }
                currencies_to_fetch.push(currency_key);
            }
        }

        // Fetch any missing or expired prices
        if !currencies_to_fetch.is_empty() {
            log::info!(
                "Fetching fresh prices for {} currencies",
                currencies_to_fetch.len()
            );
            let fresh_prices = self
                .fetch_multiple_prices_from_api(&currencies_to_fetch)
                .await?;

            // Update cache and results
            {
                let mut cache = self.cache.write().await;
                for (currency, price) in fresh_prices {
                    cache.insert(
                        currency.clone(),
                        CachedPrice {
                            price,
                            last_updated: Utc::now(),
                        },
                    );
                    results.insert(currency, price);
                }
            }
        }

        Ok(results)
    }

    /// Fetch price from external API (CoinGecko as primary)
    async fn fetch_price_from_api(&self, currency: &str) -> Result<Decimal, AppError> {
        // Map common currency aliases to CoinGecko IDs
        let coingecko_id = self.map_currency_to_coingecko_id(currency);

        // Try CoinGecko API first
        match self.fetch_from_coingecko(&coingecko_id).await {
            Ok(price) => Ok(price),
            Err(e) => {
                log::warn!(
                    "CoinGecko API failed for {}: {}, using fallback",
                    currency,
                    e
                );
                self.get_fallback_price(currency)
            }
        }
    }

    /// Fetch multiple prices from CoinGecko API
    async fn fetch_multiple_prices_from_api(
        &self,
        currencies: &[String],
    ) -> Result<HashMap<String, Decimal>, AppError> {
        let coingecko_ids: Vec<String> = currencies
            .iter()
            .map(|c| self.map_currency_to_coingecko_id(c))
            .collect();

        // Try CoinGecko API
        match self.fetch_multiple_from_coingecko(&coingecko_ids).await {
            Ok(prices) => {
                // Map back to original currency names
                let mut result = HashMap::new();
                for (i, currency) in currencies.iter().enumerate() {
                    if let Some(coingecko_id) = coingecko_ids.get(i) {
                        if let Some(price) = prices.get(coingecko_id) {
                            result.insert(currency.clone(), *price);
                        }
                    }
                }
                Ok(result)
            }
            Err(e) => {
                log::warn!("CoinGecko bulk API failed: {}, using fallback prices", e);
                // Fallback to individual prices
                let mut result = HashMap::new();
                for currency in currencies {
                    if let Ok(price) = self.get_fallback_price(currency) {
                        result.insert(currency.clone(), price);
                    }
                }
                Ok(result)
            }
        }
    }

    /// Fetch price from CoinGecko API
    async fn fetch_from_coingecko(&self, coingecko_id: &str) -> Result<Decimal, AppError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::ExternalServiceError(format!("HTTP client error: {}", e)))?;

        let url = format!(
            "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
            coingecko_id
        );

        log::debug!("Fetching price from CoinGecko: {}", url);

        let response = client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                AppError::ExternalServiceError(format!("CoinGecko API request failed: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(AppError::ExternalServiceError(format!(
                "CoinGecko API returned status: {}",
                response.status()
            )));
        }

        let price_data: CoinGeckoSimplePrice = response.json().await.map_err(|e| {
            AppError::ExternalServiceError(format!("Failed to parse CoinGecko response: {}", e))
        })?;

        if let Some(currency_data) = price_data.prices.get(coingecko_id) {
            if let Some(usd_price) = currency_data.get("usd") {
                return Decimal::from_f64(*usd_price).ok_or_else(|| {
                    AppError::ExternalServiceError("Invalid price value from CoinGecko".to_string())
                });
            }
        }

        Err(AppError::ExternalServiceError(format!(
            "Price not found for {}",
            coingecko_id
        )))
    }

    /// Fetch multiple prices from CoinGecko API
    async fn fetch_multiple_from_coingecko(
        &self,
        coingecko_ids: &[String],
    ) -> Result<HashMap<String, Decimal>, AppError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| AppError::ExternalServiceError(format!("HTTP client error: {}", e)))?;

        let ids_param = coingecko_ids.join(",");
        let url = format!(
            "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
            ids_param
        );

        log::debug!("Fetching multiple prices from CoinGecko: {}", url);

        let response = client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                AppError::ExternalServiceError(format!("CoinGecko bulk API request failed: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(AppError::ExternalServiceError(format!(
                "CoinGecko bulk API returned status: {}",
                response.status()
            )));
        }

        let price_data: CoinGeckoSimplePrice = response.json().await.map_err(|e| {
            AppError::ExternalServiceError(format!(
                "Failed to parse CoinGecko bulk response: {}",
                e
            ))
        })?;

        let mut results = HashMap::new();
        for (currency_id, currency_data) in price_data.prices {
            if let Some(usd_price) = currency_data.get("usd") {
                if let Some(decimal_price) = Decimal::from_f64(*usd_price) {
                    results.insert(currency_id, decimal_price);
                }
            }
        }

        Ok(results)
    }

    /// Map common currency names to CoinGecko IDs
    fn map_currency_to_coingecko_id(&self, currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => "bitcoin".to_string(),
            "ethereum" | "eth" => "ethereum".to_string(),
            "solana" | "sol" => "solana".to_string(),
            "bnb" | "binance" => "binancecoin".to_string(),
            "usdt" | "tether" => "tether".to_string(),
            "usdc" | "usd-coin" => "usd-coin".to_string(),
            "cardano" | "ada" => "cardano".to_string(),
            "polygon" | "matic" => "matic-network".to_string(),
            "avalanche" | "avax" => "avalanche-2".to_string(),
            "chainlink" | "link" => "chainlink".to_string(),
            _ => currency.to_string(), // Pass through unknown currencies
        }
    }

    /// Get fallback price when API is unavailable
    fn get_fallback_price(&self, currency: &str) -> Result<Decimal, AppError> {
        // These are approximate prices as of late 2024/early 2025
        // In production, you might want to store these in database or config
        let fallback_prices = match currency.to_lowercase().as_str() {
            "bitcoin" | "btc" => Decimal::from(65000),
            "ethereum" | "eth" => Decimal::from(2400),
            "solana" | "sol" => Decimal::from(140),
            "bnb" | "binance" => Decimal::from(550),
            "usdt" | "tether" | "usdc" | "usd-coin" => Decimal::ONE,
            "cardano" | "ada" => Decimal::from_f64(0.45).unwrap_or_default(),
            "polygon" | "matic" => Decimal::from_f64(0.90).unwrap_or_default(),
            "avalanche" | "avax" => Decimal::from(35),
            "chainlink" | "link" => Decimal::from(15),
            _ => {
                return Err(AppError::ExternalServiceError(format!(
                    "No fallback price available for {}",
                    currency
                )))
            }
        };

        log::warn!(
            "Using fallback price for {}: ${}",
            currency,
            fallback_prices
        );
        Ok(fallback_prices)
    }

    /// Clear the price cache (useful for testing or manual refresh)
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        log::info!("Price cache cleared");
    }

    /// Check if a currency is a stablecoin that should be priced at exactly $1.00
    fn is_stablecoin(&self, currency: &str) -> bool {
        matches!(
            currency.to_lowercase().as_str(),
            "usdt"
                | "tether"
                | "usdc"
                | "usd-coin"
                | "dai"
                | "busd"
                | "binance-usd"
                | "frax"
                | "tusd"
                | "trueusd"
                | "paxg"
                | "pax-gold"
                | "ustc"
                | "terrausd"
        )
    }

    /// Get cache statistics
    pub async fn get_cache_stats(&self) -> HashMap<String, serde_json::Value> {
        let cache = self.cache.read().await;
        let mut stats = HashMap::new();

        stats.insert(
            "cached_currencies".to_string(),
            serde_json::Value::Number(cache.len().into()),
        );

        let currencies: Vec<String> = cache.keys().cloned().collect();
        stats.insert(
            "currencies".to_string(),
            serde_json::Value::Array(
                currencies
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );

        // Find oldest and newest cache entries
        if !cache.is_empty() {
            let now = Utc::now();
            let mut oldest_age = Duration::ZERO;
            let mut newest_age = Duration::MAX;

            for cached_price in cache.values() {
                let age = now - cached_price.last_updated;
                if let Ok(age_duration) = age.to_std() {
                    if age_duration > oldest_age {
                        oldest_age = age_duration;
                    }
                    if age_duration < newest_age {
                        newest_age = age_duration;
                    }
                }
            }

            stats.insert(
                "oldest_entry_age_seconds".to_string(),
                serde_json::Value::Number(oldest_age.as_secs().into()),
            );
            stats.insert(
                "newest_entry_age_seconds".to_string(),
                serde_json::Value::Number(newest_age.as_secs().into()),
            );
        }

        stats
    }
}

impl Default for PriceOracle {
    fn default() -> Self {
        Self::new()
    }
}

// Global price oracle instance
lazy_static::lazy_static! {
    pub static ref PRICE_ORACLE: PriceOracle = PriceOracle::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fallback_prices() {
        let oracle = PriceOracle::new();

        // Test known currencies
        let btc_price = oracle.get_fallback_price("bitcoin").unwrap();
        assert!(btc_price > Decimal::from(1000));

        let eth_price = oracle.get_fallback_price("ethereum").unwrap();
        assert!(eth_price > Decimal::from(100));

        let usdt_price = oracle.get_fallback_price("usdt").unwrap();
        assert_eq!(usdt_price, Decimal::ONE);
    }

    #[tokio::test]
    async fn test_coingecko_mapping() {
        let oracle = PriceOracle::new();

        assert_eq!(oracle.map_currency_to_coingecko_id("btc"), "bitcoin");
        assert_eq!(oracle.map_currency_to_coingecko_id("ethereum"), "ethereum");
        assert_eq!(oracle.map_currency_to_coingecko_id("sol"), "solana");
        assert_eq!(oracle.map_currency_to_coingecko_id("bnb"), "binancecoin");
    }
}
