use crate::shared::{
    service::{price_oracle::PRICE_ORACLE, token_config_service::TokenConfigService},
    utils::errors::AppError,
};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use std::str::FromStr;

pub struct UsdConversionService;

impl UsdConversionService {
    /// Convert USD amount to crypto amount using live price oracle
    pub async fn convert_usd_to_crypto(
        usd_amount: &str,
        currency: &str,
        _environment: &str,
    ) -> Result<String, AppError> {
        let usd_decimal = Decimal::from_str(usd_amount)
            .map_err(|_| AppError::ValidationError("Invalid USD amount format".to_string()))?;

        if usd_decimal <= Decimal::ZERO {
            return Err(AppError::ValidationError(
                "USD amount must be greater than zero".to_string(),
            ));
        }

        // Normalize currency for price oracle
        let price_currency = Self::normalize_currency_for_pricing(currency);

        // Get current USD price from oracle (returns Decimal)
        let usd_price_decimal = PRICE_ORACLE
            .get_usd_price(&price_currency)
            .await
            .map_err(|e| {
                AppError::ExternalServiceError(format!(
                    "Failed to get USD price for {}: {}",
                    price_currency, e
                ))
            })?;

        if usd_price_decimal <= Decimal::ZERO {
            return Err(AppError::ExternalServiceError(format!(
                "Invalid USD price for {}: {}",
                price_currency, usd_price_decimal
            )));
        }

        let crypto_amount = usd_decimal / usd_price_decimal;

        log::info!(
            "💱 USD conversion: ${} USD ÷ ${}/unit = {} {}",
            usd_amount,
            usd_price_decimal,
            crypto_amount,
            currency
        );

        Ok(crypto_amount.to_string())
    }

    /// Convert USD to token base units for USDT tokens
    pub async fn convert_usd_to_token_base_units(
        usd_amount: &str,
        currency: &str,
        environment: &str,
    ) -> Result<String, AppError> {
        // First convert USD to crypto amount
        let crypto_amount = Self::convert_usd_to_crypto(usd_amount, currency, environment).await?;

        // If this is a token, convert to base units using proper decimals
        if TokenConfigService::is_supported_token(currency) {
            let token_config = TokenConfigService::get_token_config(currency, environment)?;
            TokenConfigService::amount_to_base_units(
                &crypto_amount,
                token_config.decimals,
                "crypto",
            )
        } else {
            // For native currencies, return as-is (will be handled by existing logic)
            Ok(crypto_amount)
        }
    }

    /// Normalize currency names for price oracle compatibility
    fn normalize_currency_for_pricing(currency: &str) -> String {
        match currency.to_lowercase().as_str() {
            "usdt_erc20" | "usdt_bep20" | "usdt" => "usdt".to_string(),
            "usdc_erc20" | "usdc_bep20" | "usdc" => "usdc".to_string(),
            "ethereum" | "eth" => "ethereum".to_string(),
            "bitcoin" | "btc" => "bitcoin".to_string(),
            "solana" | "sol" => "solana".to_string(),
            "bnb" | "bsc" => "binancecoin".to_string(),
            _ => currency.to_lowercase(),
        }
    }

    /// Get supported currencies for USD conversion
    pub fn get_supported_currencies() -> Vec<String> {
        vec![
            "usdt_erc20".to_string(),
            "usdt_bep20".to_string(),
            "eth".to_string(),
            "bnb".to_string(),
            "sol".to_string(),
            "btc".to_string(),
        ]
    }

    /// Check if currency supports USD conversion
    pub fn supports_usd_conversion(currency: &str) -> bool {
        Self::get_supported_currencies().contains(&currency.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_currency_for_pricing() {
        assert_eq!(
            UsdConversionService::normalize_currency_for_pricing("usdt_erc20"),
            "usdt"
        );
        assert_eq!(
            UsdConversionService::normalize_currency_for_pricing("usdt_bep20"),
            "usdt"
        );
        assert_eq!(
            UsdConversionService::normalize_currency_for_pricing("eth"),
            "ethereum"
        );
        assert_eq!(
            UsdConversionService::normalize_currency_for_pricing("bnb"),
            "binancecoin"
        );
    }

    #[test]
    fn test_supports_usd_conversion() {
        assert!(UsdConversionService::supports_usd_conversion("usdt_erc20"));
        assert!(UsdConversionService::supports_usd_conversion("eth"));
        assert!(!UsdConversionService::supports_usd_conversion("unknown"));
    }
}
