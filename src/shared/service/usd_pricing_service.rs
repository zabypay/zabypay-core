use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sea_orm::{DatabaseConnection, IntoActiveModel};
use std::collections::HashMap;

use crate::shared::{
    entities::{payment_request, wallet_balance},
    models::usd_pricing::*,
    service::price_oracle::PRICE_ORACLE,
    utils::errors::AppError,
};

/// Service for computing and managing USD valuations across the system
pub struct UsdPricingService {
    config: UsdPricingConfig,
}

impl UsdPricingService {
    /// Create new USD pricing service with default configuration
    pub fn new() -> Self {
        Self {
            config: UsdPricingConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: UsdPricingConfig) -> Self {
        Self { config }
    }

    /// Compute USD pricing for a single crypto amount
    pub async fn compute_usd_pricing(
        &self,
        crypto_amount: Decimal,
        currency: &str,
        environment: &str,
    ) -> UsdPricing {
        if !self.config.enabled {
            return UsdPricing::unavailable();
        }

        if !self.is_supported_currency(currency) {
            log::debug!("Currency {} not supported for USD pricing", currency);
            return UsdPricing::unavailable();
        }

        let is_testnet = environment.to_lowercase() == "testnet";

        // Configure price oracle cache TTL based on environment
        let cache_duration = if is_testnet {
            self.config.testnet_cache_ttl
        } else {
            self.config.mainnet_cache_ttl
        };

        // Get USD price from oracle using normalized currency
        let normalized_currency = self.normalize_currency(currency);
        match PRICE_ORACLE.get_usd_price(&normalized_currency).await {
            Ok(usd_price) => {
                log::debug!(
                    "Got USD price for {} (normalized from {}): ${} (environment: {})",
                    normalized_currency,
                    currency,
                    usd_price,
                    environment
                );
                UsdPricing::from_live_price(crypto_amount, usd_price, is_testnet)
            }
            Err(e) => {
                log::warn!(
                    "Failed to get USD price for {} (normalized from {}): {}",
                    normalized_currency,
                    currency,
                    e
                );
                UsdPricing::unavailable()
            }
        }
    }

    /// Compute USD pricing for multiple currencies efficiently
    pub async fn compute_multiple_usd_pricing(
        &self,
        amounts_and_currencies: Vec<(Decimal, String)>,
        environment: &str,
    ) -> HashMap<String, UsdPricing> {
        if !self.config.enabled {
            return HashMap::new();
        }

        let is_testnet = environment.to_lowercase() == "testnet";

        // Extract unique normalized currencies
        let normalized_currencies: Vec<String> = amounts_and_currencies
            .iter()
            .map(|(_, currency)| self.normalize_currency(currency))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .filter(|c| {
                self.config
                    .supported_currencies
                    .iter()
                    .any(|sc| sc.eq_ignore_ascii_case(c))
            })
            .collect();

        let currency_refs: Vec<&str> = normalized_currencies.iter().map(|c| c.as_str()).collect();

        if currency_refs.is_empty() {
            return HashMap::new();
        }

        // Get prices for all currencies
        match PRICE_ORACLE.get_multiple_usd_prices(&currency_refs).await {
            Ok(prices) => {
                let mut results = HashMap::new();

                for (amount, currency) in amounts_and_currencies {
                    let normalized_currency = self.normalize_currency(&currency);
                    if let Some(usd_price) = prices.get(&normalized_currency.to_lowercase()) {
                        let pricing = UsdPricing::from_live_price(amount, *usd_price, is_testnet);
                        results.insert(currency, pricing);
                    } else {
                        results.insert(currency, UsdPricing::unavailable());
                    }
                }

                results
            }
            Err(e) => {
                log::warn!("Failed to get bulk USD prices: {}", e);
                // Return unavailable pricing for all currencies
                amounts_and_currencies
                    .into_iter()
                    .map(|(_, currency)| (currency, UsdPricing::unavailable()))
                    .collect()
            }
        }
    }

    /// Update payment metadata with USD pricing
    pub async fn enrich_payment_with_usd_pricing(
        &self,
        payment: &mut payment_request::ActiveModel,
        force_refresh: bool,
    ) -> Result<(), AppError> {
        use sea_orm::{ActiveValue, IntoActiveValue};

        if !self.config.enabled {
            return Ok(());
        }

        // Extract current values
        let amount = match &payment.amount {
            ActiveValue::Set(val) => *val,
            ActiveValue::Unchanged(val) => *val,
            _ => {
                return Err(AppError::ValidationError(
                    "Payment amount not available".to_string(),
                ))
            }
        };

        let currency = match &payment.currency {
            ActiveValue::Set(val) => val.clone(),
            ActiveValue::Unchanged(val) => val.clone(),
            _ => {
                return Err(AppError::ValidationError(
                    "Payment currency not available".to_string(),
                ))
            }
        };

        let environment = match &payment.environment {
            ActiveValue::Set(val) => val.clone(),
            ActiveValue::Unchanged(val) => val.clone(),
            _ => {
                return Err(AppError::ValidationError(
                    "Payment environment not available".to_string(),
                ))
            }
        };

        let existing_metadata = match &payment.metadata {
            ActiveValue::Set(val) => val.clone(),
            ActiveValue::Unchanged(val) => val.clone(),
            _ => None,
        };

        // Check if we need to refresh pricing
        if !force_refresh && existing_metadata.is_some() {
            let existing_pricing = PaymentMetadata::extract_usd_pricing(existing_metadata.clone());
            let cache_ttl = if environment.to_lowercase() == "testnet" {
                self.config.testnet_cache_ttl
            } else {
                self.config.mainnet_cache_ttl
            };

            if !existing_pricing.is_stale(cache_ttl) {
                // Existing pricing is still fresh
                return Ok(());
            }
        }

        // Compute fresh USD pricing
        let usd_pricing = self
            .compute_usd_pricing(amount, &currency, &environment)
            .await;

        // Update metadata
        let new_metadata = PaymentMetadata::from_json_with_usd_pricing(
            existing_metadata,
            usd_pricing,
        )
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to serialize payment metadata: {}", e))
        })?;

        payment.metadata = Some(new_metadata).into_active_value();
        payment.updated_at =
            Some(Utc::now().with_timezone(&chrono::FixedOffset::east_opt(0).unwrap()))
                .into_active_value();

        Ok(())
    }

    /// Create USD-valued balance from raw balance data
    pub async fn create_usd_valued_balance(
        &self,
        currency: String,
        available_balance: Decimal,
        pending_balance: Decimal,
        total_balance: Decimal,
        environment: &str,
    ) -> UsdValuedBalance {
        if !self.config.enabled || !self.is_supported_currency(&currency) {
            return UsdValuedBalance::from_balance_and_price(
                currency,
                available_balance,
                pending_balance,
                total_balance,
                None,
                environment.to_lowercase() == "testnet",
            );
        }

        let normalized_currency = self.normalize_currency(&currency);
        let usd_price = match PRICE_ORACLE.get_usd_price(&normalized_currency).await {
            Ok(price) => Some(price),
            Err(e) => {
                log::warn!(
                    "Failed to get USD price for balance {} (normalized from {}): {}",
                    normalized_currency,
                    currency,
                    e
                );
                None
            }
        };

        UsdValuedBalance::from_balance_and_price(
            currency,
            available_balance,
            pending_balance,
            total_balance,
            usd_price,
            environment.to_lowercase() == "testnet",
        )
    }

    /// Create complete USD balance summary for a merchant
    pub async fn create_balance_summary(
        &self,
        balances: Vec<(String, Decimal, Decimal, Decimal)>, // (currency, available, pending, total)
        environment: &str,
    ) -> UsdBalanceSummary {
        if !self.config.enabled {
            let usd_balances: Vec<UsdValuedBalance> = balances
                .into_iter()
                .map(|(currency, available, pending, total)| {
                    UsdValuedBalance::from_balance_and_price(
                        currency,
                        available,
                        pending,
                        total,
                        None,
                        environment.to_lowercase() == "testnet",
                    )
                })
                .collect();

            return UsdBalanceSummary::from_balances(usd_balances, environment.to_string());
        }

        // Extract unique normalized currencies for batch price fetching
        let normalized_currencies: Vec<String> = balances
            .iter()
            .map(|(currency, _, _, _)| self.normalize_currency(currency))
            .filter(|c| {
                self.config
                    .supported_currencies
                    .iter()
                    .any(|sc| sc.eq_ignore_ascii_case(c))
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let currency_refs: Vec<&str> = normalized_currencies.iter().map(|c| c.as_str()).collect();

        // Get all prices in one batch
        let prices = match PRICE_ORACLE.get_multiple_usd_prices(&currency_refs).await {
            Ok(prices) => prices,
            Err(e) => {
                log::warn!("Failed to get bulk prices for balance summary: {}", e);
                HashMap::new()
            }
        };

        // Create USD-valued balances
        let usd_balances: Vec<UsdValuedBalance> = balances
            .into_iter()
            .map(|(currency, available, pending, total)| {
                let normalized_currency = self.normalize_currency(&currency);
                let usd_price = prices.get(&normalized_currency.to_lowercase()).copied();
                UsdValuedBalance::from_balance_and_price(
                    currency,
                    available,
                    pending,
                    total,
                    usd_price,
                    environment.to_lowercase() == "testnet",
                )
            })
            .collect();

        UsdBalanceSummary::from_balances(usd_balances, environment.to_string())
    }

    /// Refresh USD pricing for pending payments (background job)
    pub async fn refresh_pending_payment_pricing(
        &self,
        db: &DatabaseConnection,
        environment: Option<&str>,
    ) -> Result<u32, AppError> {
        use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};

        if !self.config.enabled {
            return Ok(0);
        }

        log::info!("Starting USD pricing refresh for pending payments");

        // Build query for pending payments
        let mut query =
            payment_request::Entity::find().filter(payment_request::Column::Status.eq("pending"));

        if let Some(env) = environment {
            query = query.filter(payment_request::Column::Environment.eq(env));
        }

        let pending_payments = query.all(db).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to query pending payments: {}", e))
        })?;

        let mut updated_count = 0;

        for payment in pending_payments {
            // Check if pricing needs refresh
            let existing_pricing = PaymentMetadata::extract_usd_pricing(payment.metadata.clone());
            let cache_ttl = if payment.environment.to_lowercase() == "testnet" {
                self.config.testnet_cache_ttl
            } else {
                self.config.mainnet_cache_ttl
            };

            if !existing_pricing.is_stale(cache_ttl) {
                continue; // Skip if still fresh
            }

            // Update pricing
            let payment_id = payment.id.clone();
            let mut active_payment = payment.into_active_model();
            if self
                .enrich_payment_with_usd_pricing(&mut active_payment, true)
                .await
                .is_ok()
            {
                if let Err(e) = active_payment.save(db).await {
                    log::warn!(
                        "Failed to save updated pricing for payment {}: {}",
                        payment_id,
                        e
                    );
                    continue;
                }
                updated_count += 1;
            }
        }

        log::info!("Updated USD pricing for {} pending payments", updated_count);
        Ok(updated_count)
    }

    /// Check if currency is supported for USD pricing
    fn is_supported_currency(&self, currency: &str) -> bool {
        let normalized_currency = self.normalize_currency(currency);
        self.config
            .supported_currencies
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&normalized_currency))
    }

    /// Normalize currency names to handle token variants
    fn normalize_currency(&self, currency: &str) -> String {
        // Handle BEP-20 and ERC-20 token variants
        if currency.to_uppercase().starts_with("USDT") {
            "USDT".to_string()
        } else if currency.to_uppercase().starts_with("USDC") {
            "USDC".to_string()
        } else if currency.to_uppercase().starts_with("BNB") {
            "BNB".to_string()
        } else if currency.to_uppercase().starts_with("ETH") {
            "ETH".to_string()
        } else if currency.to_uppercase().starts_with("BTC") {
            "BTC".to_string()
        } else if currency.to_uppercase().starts_with("SOL") {
            "SOL".to_string()
        } else {
            currency.to_uppercase()
        }
    }

    /// Get current configuration
    pub fn get_config(&self) -> &UsdPricingConfig {
        &self.config
    }

    /// Update configuration
    pub fn update_config(&mut self, new_config: UsdPricingConfig) {
        self.config = new_config;
    }
}

impl Default for UsdPricingService {
    fn default() -> Self {
        Self::new()
    }
}

// Global USD pricing service instance
lazy_static::lazy_static! {
    pub static ref USD_PRICING_SERVICE: UsdPricingService = UsdPricingService::new();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_supported_currencies() {
        let service = UsdPricingService::new();

        assert!(service.is_supported_currency("BTC"));
        assert!(service.is_supported_currency("eth"));
        assert!(service.is_supported_currency("BnB"));
        assert!(!service.is_supported_currency("UNKNOWN"));
    }

    #[tokio::test]
    async fn test_usd_balance_creation() {
        let service = UsdPricingService::new();

        // Test with mock pricing - in real test you'd mock the PRICE_ORACLE
        let balance = service
            .create_usd_valued_balance(
                "BTC".to_string(),
                Decimal::from_str("1.0").unwrap(),
                Decimal::from_str("0.5").unwrap(),
                Decimal::from_str("1.5").unwrap(),
                "mainnet",
            )
            .await;

        assert_eq!(balance.currency, "BTC");
        assert_eq!(balance.total_balance, Decimal::from_str("1.5").unwrap());
        assert!(!balance.is_simulated); // mainnet
    }
}
