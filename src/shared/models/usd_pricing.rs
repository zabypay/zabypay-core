use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// USD pricing information for a crypto amount
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdPricing {
    /// USD price per unit of cryptocurrency at time of calculation
    pub usd_price: Option<Decimal>,
    /// Total USD value (crypto_amount * usd_price)
    pub usd_value: Option<Decimal>,
    /// Timestamp when price was calculated
    pub price_snapshot_ts: Option<DateTime<Utc>>,
    /// Environment-specific price indicator (mainnet uses live prices, testnet simulated)
    pub is_simulated: Option<bool>,
}

impl UsdPricing {
    /// Create new USD pricing from live price oracle
    pub fn from_live_price(crypto_amount: Decimal, usd_price: Decimal, is_testnet: bool) -> Self {
        let usd_value = crypto_amount * usd_price;

        Self {
            usd_price: Some(usd_price),
            usd_value: Some(usd_value),
            price_snapshot_ts: Some(Utc::now()),
            is_simulated: Some(is_testnet),
        }
    }

    /// Create USD pricing with no price available
    pub fn unavailable() -> Self {
        Self {
            usd_price: None,
            usd_value: None,
            price_snapshot_ts: None,
            is_simulated: None,
        }
    }

    /// Check if pricing is stale (older than given seconds)
    pub fn is_stale(&self, max_age_seconds: u64) -> bool {
        match self.price_snapshot_ts {
            Some(timestamp) => {
                let age = Utc::now() - timestamp;
                age.num_seconds() as u64 > max_age_seconds
            }
            None => true, // No timestamp = stale
        }
    }

    /// Update the USD value based on new crypto amount (keep same price/timestamp)
    pub fn recalculate_value(&mut self, new_crypto_amount: Decimal) {
        if let Some(price) = self.usd_price {
            self.usd_value = Some(new_crypto_amount * price);
        }
    }
}

impl Default for UsdPricing {
    fn default() -> Self {
        Self::unavailable()
    }
}

/// Extended payment metadata that includes USD pricing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMetadata {
    /// USD pricing information
    #[serde(flatten)]
    pub usd_pricing: UsdPricing,

    /// Additional custom metadata fields
    #[serde(flatten)]
    pub custom_fields: HashMap<String, serde_json::Value>,
}

impl PaymentMetadata {
    /// Create from existing JSON string, preserving custom fields
    pub fn from_json_with_usd_pricing(
        existing_json: Option<String>,
        usd_pricing: UsdPricing,
    ) -> Result<String, serde_json::Error> {
        let mut metadata = if let Some(json_str) = existing_json {
            // Parse existing metadata, preserving custom fields
            let existing: HashMap<String, serde_json::Value> =
                serde_json::from_str(&json_str).unwrap_or_default();

            PaymentMetadata {
                usd_pricing,
                custom_fields: existing,
            }
        } else {
            // New metadata with just USD pricing
            PaymentMetadata {
                usd_pricing,
                custom_fields: HashMap::new(),
            }
        };

        serde_json::to_string(&metadata)
    }

    /// Extract USD pricing from JSON metadata string
    pub fn extract_usd_pricing(metadata_json: Option<String>) -> UsdPricing {
        if let Some(json_str) = metadata_json {
            if let Ok(metadata) = serde_json::from_str::<PaymentMetadata>(&json_str) {
                return metadata.usd_pricing;
            }
        }
        UsdPricing::unavailable()
    }
}

/// Balance with USD valuation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdValuedBalance {
    pub currency: String,
    pub available_balance: Decimal,
    pub pending_balance: Decimal,
    pub total_balance: Decimal,

    /// USD pricing for available balance
    pub available_usd_price: Option<Decimal>,
    pub available_usd_value: Option<Decimal>,

    /// USD pricing for pending balance  
    pub pending_usd_price: Option<Decimal>,
    pub pending_usd_value: Option<Decimal>,

    /// USD pricing for total balance
    pub total_usd_price: Option<Decimal>,
    pub total_usd_value: Option<Decimal>,

    /// When prices were calculated
    pub price_snapshot_ts: Option<DateTime<Utc>>,

    /// Whether this is simulated pricing (testnet)
    pub is_simulated: bool,
}

impl UsdValuedBalance {
    /// Create from raw balance and USD price
    pub fn from_balance_and_price(
        currency: String,
        available_balance: Decimal,
        pending_balance: Decimal,
        total_balance: Decimal,
        usd_price: Option<Decimal>,
        is_testnet: bool,
    ) -> Self {
        let (available_usd_value, pending_usd_value, total_usd_value) =
            if let Some(price) = usd_price {
                (
                    Some(available_balance * price),
                    Some(pending_balance * price),
                    Some(total_balance * price),
                )
            } else {
                (None, None, None)
            };

        Self {
            currency,
            available_balance,
            pending_balance,
            total_balance,
            available_usd_price: usd_price,
            available_usd_value,
            pending_usd_price: usd_price,
            pending_usd_value,
            total_usd_price: usd_price,
            total_usd_value,
            price_snapshot_ts: if usd_price.is_some() {
                Some(Utc::now())
            } else {
                None
            },
            is_simulated: is_testnet,
        }
    }
}

/// Aggregated USD balance summary across all currencies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdBalanceSummary {
    /// Total USD value across all available balances
    pub total_available_usd: Decimal,

    /// Total USD value across all pending balances
    pub total_pending_usd: Decimal,

    /// Grand total USD value
    pub total_usd_value: Decimal,

    /// Per-currency breakdowns
    pub balances: Vec<UsdValuedBalance>,

    /// When this summary was calculated
    pub calculated_at: DateTime<Utc>,

    /// Environment (affects price display in UI)
    pub environment: String,
}

impl UsdBalanceSummary {
    /// Create summary from individual USD-valued balances
    pub fn from_balances(balances: Vec<UsdValuedBalance>, environment: String) -> Self {
        let total_available_usd = balances
            .iter()
            .filter_map(|b| b.available_usd_value)
            .fold(Decimal::ZERO, |acc, val| acc + val);

        let total_pending_usd = balances
            .iter()
            .filter_map(|b| b.pending_usd_value)
            .fold(Decimal::ZERO, |acc, val| acc + val);

        let total_usd_value = total_available_usd + total_pending_usd;

        Self {
            total_available_usd,
            total_pending_usd,
            total_usd_value,
            balances,
            calculated_at: Utc::now(),
            environment,
        }
    }
}

/// Configuration for USD pricing behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsdPricingConfig {
    /// Cache TTL for mainnet prices (seconds)
    pub mainnet_cache_ttl: u64,

    /// Cache TTL for testnet prices (seconds)  
    pub testnet_cache_ttl: u64,

    /// Whether to enable USD pricing globally
    pub enabled: bool,

    /// Fallback behavior when price unavailable
    pub use_fallback_prices: bool,

    /// Supported currencies for USD conversion
    pub supported_currencies: Vec<String>,
}

impl Default for UsdPricingConfig {
    fn default() -> Self {
        Self {
            mainnet_cache_ttl: 60,  // 1 minute for mainnet
            testnet_cache_ttl: 300, // 5 minutes for testnet
            enabled: true,
            use_fallback_prices: true,
            supported_currencies: vec![
                "BTC".to_string(),
                "ETH".to_string(),
                "BNB".to_string(),
                "SOL".to_string(),
                "USDT".to_string(),
                "USDC".to_string(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_usd_pricing_creation() {
        let pricing = UsdPricing::from_live_price(
            Decimal::from_str("0.5").unwrap(),
            Decimal::from_str("600.0").unwrap(),
            false,
        );

        assert_eq!(pricing.usd_price, Some(Decimal::from_str("600.0").unwrap()));
        assert_eq!(pricing.usd_value, Some(Decimal::from_str("300.0").unwrap())); // 0.5 * 600
        assert_eq!(pricing.is_simulated, Some(false));
        assert!(pricing.price_snapshot_ts.is_some());
    }

    #[test]
    fn test_usd_pricing_unavailable() {
        let pricing = UsdPricing::unavailable();

        assert_eq!(pricing.usd_price, None);
        assert_eq!(pricing.usd_value, None);
        assert_eq!(pricing.price_snapshot_ts, None);
    }

    #[test]
    fn test_payment_metadata_serialization() {
        let usd_pricing = UsdPricing::from_live_price(
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("600.0").unwrap(),
            false,
        );
        let metadata_json = PaymentMetadata::from_json_with_usd_pricing(None, usd_pricing).unwrap();

        assert!(metadata_json.contains("usd_price"));
        assert!(metadata_json.contains("usd_value"));
        assert!(metadata_json.contains("price_snapshot_ts"));
    }

    #[test]
    fn test_usd_valued_balance() {
        let balance = UsdValuedBalance::from_balance_and_price(
            "BNB".to_string(),
            Decimal::from_str("1.5").unwrap(),
            Decimal::from_str("0.5").unwrap(),
            Decimal::from_str("2.0").unwrap(),
            Some(Decimal::from_str("600.0").unwrap()),
            false,
        );

        assert_eq!(
            balance.available_usd_value,
            Some(Decimal::from_str("900.0").unwrap())
        ); // 1.5 * 600
        assert_eq!(
            balance.pending_usd_value,
            Some(Decimal::from_str("300.0").unwrap())
        ); // 0.5 * 600
        assert_eq!(
            balance.total_usd_value,
            Some(Decimal::from_str("1200.0").unwrap())
        ); // 2.0 * 600
        assert!(!balance.is_simulated);
    }
}
