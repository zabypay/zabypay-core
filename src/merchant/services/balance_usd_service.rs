use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::collections::HashMap;

use crate::shared::{
    entities::{merchant, wallet, wallet_balance},
    models::usd_pricing::{UsdBalanceSummary, UsdValuedBalance},
    service::usd_pricing_service::USD_PRICING_SERVICE,
    utils::errors::AppError,
};

/// Service for managing wallet balances with USD valuations
pub struct BalanceUsdService;

impl BalanceUsdService {
    /// Get USD-valued balances for a merchant across all currencies
    pub async fn get_merchant_balances_with_usd(
        db: &DatabaseConnection,
        merchant_id: &str,
        environment: Option<&str>,
    ) -> Result<UsdBalanceSummary, AppError> {
        // Get merchant to find user_id
        let merchant_record = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to find merchant: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        // Get all wallets for this merchant
        let mut wallet_query =
            wallet::Entity::find().filter(wallet::Column::UserId.eq(&merchant_record.user_id));

        // Filter by environment if specified
        if let Some(env) = environment {
            let env_pattern = format!("_{}", env);
            wallet_query = wallet_query.filter(wallet::Column::Currency.contains(&env_pattern));
        }

        let wallets = wallet_query
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query wallets: {}", e)))?;

        if wallets.is_empty() {
            let env_str = environment.unwrap_or("all environments");
            return Ok(UsdBalanceSummary::from_balances(
                vec![],
                env_str.to_string(),
            ));
        }

        // Get balance data for each wallet
        let wallet_ids: Vec<String> = wallets.iter().map(|w| w.id.clone()).collect();
        let balances = wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.is_in(wallet_ids))
            .all(db)
            .await
            .map_err(|e| {
                AppError::DatabaseError(format!("Failed to query wallet balances: {}", e))
            })?;

        // Group balances by currency
        let mut currency_balances: HashMap<String, (Decimal, Decimal, Decimal)> = HashMap::new();

        for balance in balances {
            // Find the corresponding wallet to get currency info
            if let Some(wallet_info) = wallets.iter().find(|w| w.id == balance.wallet_id) {
                let base_currency = Self::extract_base_currency(&wallet_info.currency);
                let entry = currency_balances.entry(base_currency).or_insert((
                    Decimal::ZERO,
                    Decimal::ZERO,
                    Decimal::ZERO,
                ));
                entry.0 += balance.available_balance;
                entry.1 += balance.pending_balance;
                entry.2 += balance.total_balance;
            }
        }

        // Convert to USD-valued balances
        let balance_tuples: Vec<(String, Decimal, Decimal, Decimal)> = currency_balances
            .into_iter()
            .map(|(currency, (available, pending, total))| (currency, available, pending, total))
            .collect();

        let env_str = environment.unwrap_or("mainnet");
        let summary = USD_PRICING_SERVICE
            .create_balance_summary(balance_tuples, env_str)
            .await;

        log::info!(" Generated USD balance summary for merchant {} ({} env): {} currencies, ${:.2} total", 
                  merchant_id, env_str, summary.balances.len(), summary.total_usd_value);

        Ok(summary)
    }

    /// Get USD-valued balance for a specific currency
    pub async fn get_currency_balance_with_usd(
        db: &DatabaseConnection,
        merchant_id: &str,
        currency: &str,
        environment: &str,
    ) -> Result<Option<UsdValuedBalance>, AppError> {
        // Get merchant to find user_id
        let merchant_record = merchant::Entity::find_by_id(merchant_id)
            .one(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to find merchant: {}", e)))?
            .ok_or_else(|| AppError::NotFound("Merchant not found".to_string()))?;

        // Find wallets for this currency and environment
        let currency_pattern = format!("{}_{}", currency.to_lowercase(), environment);
        let wallets = wallet::Entity::find()
            .filter(wallet::Column::UserId.eq(&merchant_record.user_id))
            .filter(wallet::Column::Currency.contains(&currency_pattern))
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query wallets: {}", e)))?;

        if wallets.is_empty() {
            return Ok(None);
        }

        // Get balance data for these wallets
        let wallet_ids: Vec<String> = wallets.iter().map(|w| w.id.clone()).collect();
        let balances = wallet_balance::Entity::find()
            .filter(wallet_balance::Column::WalletId.is_in(wallet_ids))
            .all(db)
            .await
            .map_err(|e| {
                AppError::DatabaseError(format!("Failed to query wallet balances: {}", e))
            })?;

        // Aggregate balances
        let mut total_available = Decimal::ZERO;
        let mut total_pending = Decimal::ZERO;
        let mut total_balance = Decimal::ZERO;

        for balance in balances {
            total_available += balance.available_balance;
            total_pending += balance.pending_balance;
            total_balance += balance.total_balance;
        }

        // Create USD-valued balance
        let usd_balance = USD_PRICING_SERVICE
            .create_usd_valued_balance(
                currency.to_uppercase(),
                total_available,
                total_pending,
                total_balance,
                environment,
            )
            .await;

        Ok(Some(usd_balance))
    }

    /// Refresh USD pricing for all balances (background job)
    pub async fn refresh_balance_pricing(
        db: &DatabaseConnection,
        environment: Option<&str>,
    ) -> Result<u32, AppError> {
        log::info!("🔄 Starting balance USD pricing refresh");

        // Get all merchants
        let merchants = merchant::Entity::find()
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query merchants: {}", e)))?;

        let mut refresh_count = 0;

        for merchant in merchants {
            // Refresh balance summary for this merchant
            match Self::get_merchant_balances_with_usd(db, &merchant.id, environment).await {
                Ok(summary) => {
                    log::debug!(
                        "✅ Refreshed balance pricing for merchant {}: {} currencies, ${:.2} total",
                        merchant.id,
                        summary.balances.len(),
                        summary.total_usd_value
                    );
                    refresh_count += 1;
                }
                Err(e) => {
                    log::warn!(
                        " Failed to refresh balance pricing for merchant {}: {}",
                        merchant.id,
                        e
                    );
                }
            }
        }

        log::info!(
            "✅ Completed balance USD pricing refresh: {} merchants processed",
            refresh_count
        );
        Ok(refresh_count)
    }

    /// Calculate total USD value across all merchant balances  
    pub async fn get_total_platform_value_usd(
        db: &DatabaseConnection,
        environment: Option<&str>,
    ) -> Result<Decimal, AppError> {
        let merchants = merchant::Entity::find()
            .all(db)
            .await
            .map_err(|e| AppError::DatabaseError(format!("Failed to query merchants: {}", e)))?;

        let mut total_platform_usd = Decimal::ZERO;

        for merchant in merchants {
            match Self::get_merchant_balances_with_usd(db, &merchant.id, environment).await {
                Ok(summary) => {
                    total_platform_usd += summary.total_usd_value;
                }
                Err(e) => {
                    log::warn!("Failed to get balance for merchant {}: {}", merchant.id, e);
                }
            }
        }

        Ok(total_platform_usd)
    }

    /// Get balance history with USD values (for charts/analytics)
    pub async fn get_balance_history_with_usd(
        db: &DatabaseConnection,
        merchant_id: &str,
        currency: Option<&str>,
        days: u32,
        environment: &str,
    ) -> Result<Vec<BalanceHistoryPoint>, AppError> {
        // This is a placeholder for balance history functionality
        // In a real implementation, you'd need a separate table to store historical balance snapshots

        log::info!(
            "📈 Balance history requested for merchant {} ({} days)",
            merchant_id,
            days
        );

        // For now, return current balance as a single point
        let current_summary =
            Self::get_merchant_balances_with_usd(db, merchant_id, Some(environment)).await?;

        let history_point = BalanceHistoryPoint {
            timestamp: chrono::Utc::now(),
            currency: currency.unwrap_or("ALL").to_string(),
            crypto_balance: if let Some(currency_filter) = currency {
                current_summary
                    .balances
                    .iter()
                    .find(|b| b.currency.eq_ignore_ascii_case(currency_filter))
                    .map(|b| b.total_balance)
                    .unwrap_or(Decimal::ZERO)
            } else {
                current_summary
                    .balances
                    .iter()
                    .map(|b| b.total_balance)
                    .fold(Decimal::ZERO, |acc, val| acc + val)
            },
            usd_value: if let Some(currency_filter) = currency {
                current_summary
                    .balances
                    .iter()
                    .find(|b| b.currency.eq_ignore_ascii_case(currency_filter))
                    .and_then(|b| b.total_usd_value)
                    .unwrap_or(Decimal::ZERO)
            } else {
                current_summary.total_usd_value
            },
            environment: environment.to_string(),
        };

        Ok(vec![history_point])
    }

    /// Extract base currency from full currency string (e.g., "btc_mainnet_uuid" -> "BTC")
    fn extract_base_currency(full_currency: &str) -> String {
        let parts: Vec<&str> = full_currency.split('_').collect();
        if !parts.is_empty() {
            parts[0].to_uppercase()
        } else {
            full_currency.to_uppercase()
        }
    }
}

/// Historical balance point with USD valuation
#[derive(Debug, Clone, serde::Serialize)]
pub struct BalanceHistoryPoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub currency: String,
    pub crypto_balance: Decimal,
    pub usd_value: Decimal,
    pub environment: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_currency() {
        assert_eq!(
            BalanceUsdService::extract_base_currency("btc_mainnet_12345"),
            "BTC"
        );
        assert_eq!(
            BalanceUsdService::extract_base_currency("eth_testnet_67890"),
            "ETH"
        );
        assert_eq!(
            BalanceUsdService::extract_base_currency("bnb_mainnet"),
            "BNB"
        );
        assert_eq!(BalanceUsdService::extract_base_currency("SOL"), "SOL");
    }
}
