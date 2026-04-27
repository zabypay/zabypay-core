use sea_orm::DatabaseConnection;
use std::sync::Arc;
use tokio::time::{interval, Duration};

use crate::merchant::services::balance_usd_service::BalanceUsdService;
use crate::shared::{service::usd_pricing_service::USD_PRICING_SERVICE, utils::errors::AppError};

/// Background service for refreshing USD pricing periodically
pub struct UsdPriceRefreshService {
    db: Arc<DatabaseConnection>,
    refresh_interval_seconds: u64,
    is_running: Arc<tokio::sync::RwLock<bool>>,
}

impl UsdPriceRefreshService {
    /// Create new price refresh service
    pub fn new(db: Arc<DatabaseConnection>, refresh_interval_seconds: u64) -> Self {
        Self {
            db,
            refresh_interval_seconds,
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
        }
    }

    /// Start the background price refresh service
    pub async fn start(&self) -> Result<(), AppError> {
        {
            let mut running = self.is_running.write().await;
            if *running {
                log::warn!("USD price refresh service is already running");
                return Ok(());
            }
            *running = true;
        }

        let db = Arc::clone(&self.db);
        let refresh_interval_seconds = self.refresh_interval_seconds;
        let is_running = Arc::clone(&self.is_running);

        tokio::spawn(async move {
            log::info!(
                " Starting USD price refresh service (interval: {}s)",
                refresh_interval_seconds
            );

            let mut timer = interval(Duration::from_secs(refresh_interval_seconds));

            loop {
                // Check if service should continue running
                {
                    let running = is_running.read().await;
                    if !*running {
                        log::info!("⏹️ Stopping USD price refresh service");
                        break;
                    }
                }

                timer.tick().await;

                log::debug!("🔄 Running periodic USD price refresh");

                let start_time = std::time::Instant::now();

                // Refresh pending payments pricing for both environments
                let mut total_payments_updated = 0;
                let mut total_balances_updated = 0;

                for environment in &["mainnet", "testnet"] {
                    match USD_PRICING_SERVICE
                        .refresh_pending_payment_pricing(&db, Some(environment))
                        .await
                    {
                        Ok(count) => {
                            total_payments_updated += count;
                            log::debug!(
                                "✅ Updated {} pending payments for {} environment",
                                count,
                                environment
                            );
                        }
                        Err(e) => {
                            log::error!(
                                " Failed to refresh payment pricing for {} environment: {}",
                                environment,
                                e
                            );
                        }
                    }

                    // Refresh balance pricing (less frequent)
                    match BalanceUsdService::refresh_balance_pricing(&db, Some(environment)).await {
                        Ok(count) => {
                            total_balances_updated += count;
                            log::debug!(
                                "✅ Updated balances for {} merchants in {} environment",
                                count,
                                environment
                            );
                        }
                        Err(e) => {
                            log::error!(
                                " Failed to refresh balance pricing for {} environment: {}",
                                environment,
                                e
                            );
                        }
                    }
                }

                let elapsed = start_time.elapsed();
                log::info!(
                    " USD price refresh completed in {:.2}s: {} payments, {} merchants updated",
                    elapsed.as_secs_f32(),
                    total_payments_updated,
                    total_balances_updated
                );

                // Clear price oracle cache periodically to ensure fresh data
                if total_payments_updated > 0 || total_balances_updated > 0 {
                    use crate::shared::service::price_oracle::PRICE_ORACLE;
                    // Only clear cache occasionally to avoid API rate limits
                    if rand::random::<f32>() < 0.1 {
                        // 10% chance
                        PRICE_ORACLE.clear_cache().await;
                        log::debug!("🧹 Cleared price oracle cache");
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the background service
    pub async fn stop(&self) {
        let mut running = self.is_running.write().await;
        *running = false;
        log::info!("📡 USD price refresh service stop requested");
    }

    /// Check if the service is currently running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    /// Get cache statistics from the price oracle
    pub async fn get_price_cache_stats(
        &self,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        use crate::shared::service::price_oracle::PRICE_ORACLE;
        PRICE_ORACLE.get_cache_stats().await
    }

    /// Force refresh all pricing immediately
    pub async fn force_refresh_all(&self) -> Result<(u32, u32), AppError> {
        log::info!("🔄 Force refreshing all USD pricing");

        let mut total_payments = 0;
        let mut total_balances = 0;

        for environment in &["mainnet", "testnet"] {
            // Clear cache first to ensure fresh prices
            {
                use crate::shared::service::price_oracle::PRICE_ORACLE;
                PRICE_ORACLE.clear_cache().await;
            }

            let payments_updated = USD_PRICING_SERVICE
                .refresh_pending_payment_pricing(&self.db, Some(environment))
                .await?;
            let balances_updated =
                BalanceUsdService::refresh_balance_pricing(&self.db, Some(environment)).await?;

            total_payments += payments_updated;
            total_balances += balances_updated;

            log::info!(
                "✅ Force refresh completed for {}: {} payments, {} balances",
                environment,
                payments_updated,
                balances_updated
            );
        }

        Ok((total_payments, total_balances))
    }
}

/// Configuration for the USD price refresh service
#[derive(Debug, Clone)]
pub struct UsdPriceRefreshConfig {
    /// How often to refresh prices (seconds)
    pub refresh_interval_seconds: u64,
    /// Whether to auto-start the service
    pub auto_start: bool,
    /// Whether the service is enabled
    pub enabled: bool,
}

impl Default for UsdPriceRefreshConfig {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 60, // 1 minute
            auto_start: true,
            enabled: true,
        }
    }
}

/// Global USD price refresh service factory
pub struct UsdPriceRefreshServiceFactory;

impl UsdPriceRefreshServiceFactory {
    /// Create and optionally start the USD price refresh service
    pub async fn create_and_start(
        db: Arc<DatabaseConnection>,
        config: UsdPriceRefreshConfig,
    ) -> Result<Arc<UsdPriceRefreshService>, AppError> {
        if !config.enabled {
            log::info!("💤 USD price refresh service is disabled");
            return Err(AppError::InternalServerError(
                "USD price refresh service is disabled".to_string(),
            ));
        }

        let service = Arc::new(UsdPriceRefreshService::new(
            db,
            config.refresh_interval_seconds,
        ));

        if config.auto_start {
            service.start().await?;
            log::info!("✅ USD price refresh service started automatically");
        }

        Ok(service)
    }
}
