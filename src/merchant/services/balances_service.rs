use chrono::{DateTime, Utc};
use rust_decimal::{prelude::FromStr, Decimal};
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, FromQueryResult,
    QueryFilter, QuerySelect, Statement,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    merchant::models::withdrawal::{BalancesResponse, NetworkBalance, WalletBalance},
    shared::{
        entities::{merchant, wallet},
        service::PRICE_ORACLE,
        AppState,
    },
};

/// Error types for balances service
#[derive(Debug, Clone, Serialize)]
pub struct BalancesError {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
    pub retry_after: Option<u32>,
}

impl BalancesError {
    pub fn db_unavailable(details: Option<serde_json::Value>) -> Self {
        Self {
            code: "DB_UNAVAILABLE".to_string(),
            message: "Database is temporarily unavailable".to_string(),
            details,
            retry_after: Some(30),
        }
    }

    pub fn schema_out_of_date(details: Option<serde_json::Value>) -> Self {
        Self {
            code: "DB_SCHEMA_OUT_OF_DATE".to_string(),
            message: "Database schema is out of date".to_string(),
            details,
            retry_after: None,
        }
    }

    pub fn validation_error(message: String) -> Self {
        Self {
            code: "VALIDATION_ERROR".to_string(),
            message,
            details: None,
            retry_after: None,
        }
    }
}

/// Cache entry for balances
#[derive(Debug, Clone)]
struct BalanceCacheEntry {
    data: BalancesResponse,
    cached_at: DateTime<Utc>,
}

/// Column availability cache for wallet_balance table
#[derive(Debug, Clone)]
struct WalletBalanceColumns {
    has_locked_balance: bool,
    has_pending_balance: bool,
    has_reserved_balance: bool,
    checked_at: DateTime<Utc>,
}

/// Custom result for wallet balance queries with column compatibility
#[derive(Debug, Clone, FromQueryResult)]
#[allow(dead_code)]
struct WalletBalanceCompat {
    pub id: String,
    pub wallet_id: String,
    pub currency: String,
    pub available_balance: Decimal,
    pub pending_balance: Decimal, // This will be populated via COALESCE
    pub total_balance: Decimal,
    pub last_updated: DateTimeWithTimeZone,
    pub created_at: DateTimeWithTimeZone,
}

/// Balances service with resilient DB access and proper error handling
pub struct BalancesService {
    cache_ttl_seconds: u64,
    cache: tokio::sync::RwLock<HashMap<String, BalanceCacheEntry>>,
    column_cache: Arc<Mutex<Option<WalletBalanceColumns>>>,
}

impl BalancesService {
    pub fn new() -> Self {
        let cache_ttl = std::env::var("PRICE_CACHE_TTL_SEC")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .unwrap_or(5);

        Self {
            cache_ttl_seconds: cache_ttl,
            cache: tokio::sync::RwLock::new(HashMap::new()),
            column_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Get merchant balances with resilient DB access and proper error handling
    pub async fn get_merchant_balances(
        &self,
        app_state: &AppState,
        merchant_id: &str,
        environment: &str,
    ) -> Result<BalancesResponse, BalancesError> {
        // Validate environment
        if !matches!(environment, "mainnet" | "testnet") {
            return Err(BalancesError::validation_error(
                "Environment must be 'mainnet' or 'testnet'".to_string(),
            ));
        }

        // Check cache first
        let cache_key = format!("{}:{}", merchant_id, environment);
        if let Some(cached) = self.get_from_cache(&cache_key).await {
            return Ok(cached);
        }

        // Perform database operations with READ COMMITTED isolation
        let result = self
            .fetch_balances_from_db(app_state, merchant_id, environment)
            .await;

        match result {
            Ok(balances) => {
                // Cache successful result
                self.cache_result(&cache_key, balances.clone()).await;
                Ok(balances)
            }
            Err(e) => {
                // Log structured error
                self.log_error("get_merchant_balances", merchant_id, environment, &e)
                    .await;
                Err(e)
            }
        }
    }

    /// Fetch balances from database with proper error handling
    async fn fetch_balances_from_db(
        &self,
        app_state: &AppState,
        merchant_id: &str,
        environment: &str,
    ) -> Result<BalancesResponse, BalancesError> {
        // Use READ COMMITTED isolation level for consistent reads

        // Check if required tables exist
        if let Err(e) = self.verify_schema(&app_state.db).await {
            return Err(e);
        }

        // Get merchant
        let merchant = self.get_merchant(&app_state.db, merchant_id).await?;

        // Get balances data
        let (balances, wallet_breakdown, total_wallets) = self
            .get_balance_data(&app_state.db, &merchant.user_id, merchant_id, environment)
            .await?;

        // Calculate total USD value
        let total_usd_value = self.calculate_total_usd_value(&balances).await;

        let response = BalancesResponse {
            environment: environment.to_string(),
            balances,
            wallet_breakdown,
            total_usd_value: Some(total_usd_value.to_string()),
            total_wallets,
            last_updated: Utc::now(),
        };

        Ok(response)
    }

    /// Verify database schema
    async fn verify_schema(&self, db: &DatabaseConnection) -> Result<(), BalancesError> {
        // Try to query essential tables to ensure schema is up to date
        match merchant::Entity::find().limit(1).one(db).await {
            Ok(_) => {}
            Err(DbErr::Conn(ref conn_err))
                if conn_err.to_string().contains("relation")
                    || conn_err.to_string().contains("table") =>
            {
                return Err(BalancesError::schema_out_of_date(Some(serde_json::json!({
                    "missing_table": "merchant"
                }))));
            }
            Err(e) => return Err(self.map_db_error(e)),
        }

        match wallet::Entity::find().limit(1).one(db).await {
            Ok(_) => {}
            Err(DbErr::Conn(ref conn_err))
                if conn_err.to_string().contains("relation")
                    || conn_err.to_string().contains("table") =>
            {
                return Err(BalancesError::schema_out_of_date(Some(serde_json::json!({
                    "missing_table": "wallet"
                }))));
            }
            Err(e) => return Err(self.map_db_error(e)),
        }

        // Check wallet_balance table exists using raw SQL to avoid entity column issues
        let check_table_query = "SELECT 1 FROM wallet_balance LIMIT 1";
        match db
            .query_one(Statement::from_string(
                db.get_database_backend(),
                check_table_query.to_string(),
            ))
            .await
        {
            Ok(_) => {}
            Err(DbErr::Query(ref query_err))
                if query_err.to_string().contains("does not exist")
                    || query_err.to_string().contains("relation") =>
            {
                return Err(BalancesError::schema_out_of_date(Some(serde_json::json!({
                    "missing_table": "wallet_balance"
                }))));
            }
            Err(DbErr::Query(ref query_err))
                if query_err.to_string().contains("column")
                    && query_err.to_string().contains("locked_balance") =>
            {
                // Table exists but has different columns - this is fine, we handle it
            }
            Err(e) => {
                // Check if it's a column error we can handle
                let err_str = e.to_string();
                if !err_str.contains("locked_balance") {
                    return Err(self.map_db_error(e));
                }
                // Column mismatch is fine, we handle it with our compat query
            }
        }

        Ok(())
    }

    /// Get merchant by ID
    async fn get_merchant(
        &self,
        db: &DatabaseConnection,
        merchant_id: &str,
    ) -> Result<merchant::Model, BalancesError> {
        merchant::Entity::find()
            .filter(merchant::Column::Id.eq(merchant_id))
            .one(db)
            .await
            .map_err(|e| self.map_db_error(e))?
            .ok_or_else(|| BalancesError::validation_error("Merchant not found".to_string()))
    }

    /// Check which columns exist in wallet_balance table
    async fn detect_wallet_balance_columns(
        &self,
        db: &DatabaseConnection,
    ) -> Result<WalletBalanceColumns, BalancesError> {
        // Check cache first
        let mut cache = self.column_cache.lock().await;
        if let Some(ref columns) = *cache {
            // Cache for 1 hour
            if Utc::now()
                .signed_duration_since(columns.checked_at)
                .num_seconds()
                < 3600
            {
                return Ok(columns.clone());
            }
        }

        // Query to check column existence
        let query = r#"
            SELECT column_name 
            FROM information_schema.columns 
            WHERE table_name = 'wallet_balance' 
            AND column_name IN ('locked_balance', 'pending_balance', 'reserved_balance')
        "#;

        let result: Vec<String> = db
            .query_all(Statement::from_string(
                db.get_database_backend(),
                query.to_string(),
            ))
            .await
            .map_err(|e| self.map_db_error(e))?
            .into_iter()
            .filter_map(|row| row.try_get::<String>("", "column_name").ok())
            .collect();

        let columns = WalletBalanceColumns {
            has_locked_balance: result.contains(&"locked_balance".to_string()),
            has_pending_balance: result.contains(&"pending_balance".to_string()),
            has_reserved_balance: result.contains(&"reserved_balance".to_string()),
            checked_at: Utc::now(),
        };

        *cache = Some(columns.clone());
        Ok(columns)
    }

    /// Get balance data for merchant - calculate directly from payments
    async fn get_balance_data(
        &self,
        db: &DatabaseConnection,
        user_id: &str,
        merchant_id: &str,
        environment: &str,
    ) -> Result<(Vec<NetworkBalance>, Vec<WalletBalance>, u64), BalancesError> {
        use crate::shared::entities::{payment_request, withdrawal_request};

        log::info!(
            " Getting balance data for merchant_id={}, user_id={}, environment={}",
            merchant_id,
            user_id,
            environment
        );

        // Get all paid/confirmed payments for this merchant
        // First try with environment filter, then without if empty
        let mut payments = payment_request::Entity::find()
            .filter(payment_request::Column::MerchantId.eq(merchant_id))
            .filter(payment_request::Column::Status.is_in(vec!["paid", "confirmed"]))
            .filter(payment_request::Column::Environment.eq(environment))
            .all(db)
            .await
            .map_err(|e| self.map_db_error(e))?;

        // If no payments found with environment filter, try without it (for backward compatibility)
        if payments.is_empty() {
            log::info!(
                "No payments found with environment={}, trying without environment filter",
                environment
            );
            payments = payment_request::Entity::find()
                .filter(payment_request::Column::MerchantId.eq(merchant_id))
                .filter(payment_request::Column::Status.is_in(vec!["paid", "confirmed"]))
                .all(db)
                .await
                .map_err(|e| self.map_db_error(e))?;

            // Log if we found payments without environment filter
            if !payments.is_empty() {
                log::warn!("Found {} payments without environment filter - these payments may have missing environment field", payments.len());
            }
        }

        log::info!("💰 Found {} payments matching criteria (merchant_id={}, status=paid/confirmed, environment={})",
                  payments.len(), merchant_id, environment);

        // Get all successful/processing withdrawals to deduct from balance
        // Include processing, broadcasted, and confirmed statuses (exclude failed/cancelled)
        let withdrawals = withdrawal_request::Entity::find()
            .filter(withdrawal_request::Column::MerchantId.eq(merchant_id))
            .filter(withdrawal_request::Column::Status.is_in(vec![
                "pending",
                "processing",
                "broadcasted",
                "confirmed",
            ]))
            .filter(withdrawal_request::Column::Environment.eq(environment))
            .all(db)
            .await
            .map_err(|e| self.map_db_error(e))?;

        log::info!("💸 Found {} withdrawals to deduct (merchant_id={}, environment={}, statuses=pending/processing/broadcasted/confirmed)",
                  withdrawals.len(), merchant_id, environment);

        if payments.is_empty() {
            log::warn!(
                " No paid/confirmed payments found for merchant {} in {} environment",
                merchant_id,
                environment
            );
            return Ok((vec![], vec![], 0));
        }

        // Group by currency and wallet address
        let mut currency_aggregates: HashMap<String, NetworkBalance> = HashMap::new();
        let mut wallet_breakdown_map: HashMap<String, WalletBalance> = HashMap::new();
        let mut wallet_addresses: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Build withdrawal deduction maps - both per-network and per-wallet
        let mut network_withdrawal_deductions: HashMap<String, Decimal> = HashMap::new();
        let mut wallet_withdrawal_deductions: HashMap<String, Decimal> = HashMap::new();
        let mut network_fees: HashMap<String, Decimal> = HashMap::new();

        for withdrawal in &withdrawals {
            let currency = withdrawal.currency.to_uppercase();
            let (base_currency, network) =
                self.extract_currency_and_network_from_payment(&currency, environment);
            let network_key = format!("{}_{}", base_currency, network);

            // Track network-level deductions
            let current = network_withdrawal_deductions
                .entry(network_key.clone())
                .or_insert(Decimal::ZERO);
            *current += withdrawal.amount;

            // Track fees separately at network level
            if let Some(fee) = withdrawal.fee {
                let fee_current = network_fees
                    .entry(network_key.clone())
                    .or_insert(Decimal::ZERO);
                *fee_current += fee;
            }

            // Track per-wallet deductions if wallet_id is available
            // Note: wallet_id in withdrawal might reference the source wallet
            if !withdrawal.wallet_id.is_empty() {
                let wallet_key = withdrawal.wallet_id.clone();
                let wallet_current = wallet_withdrawal_deductions
                    .entry(wallet_key)
                    .or_insert(Decimal::ZERO);
                *wallet_current += withdrawal.amount;
            }

            log::debug!(
                "Withdrawal deduction: id={}, network={}, amount={}, fee={:?}, wallet_id={}",
                withdrawal.id,
                network_key,
                withdrawal.amount,
                withdrawal.fee,
                withdrawal.wallet_id
            );
        }

        for payment in &payments {
            let currency = payment.currency.to_uppercase(); // Use uppercase to match "ETH"
            let wallet_addr = payment.wallet_address.clone();
            wallet_addresses.insert(wallet_addr.clone());

            // Extract currency and network info
            let (base_currency, network) =
                self.extract_currency_and_network_from_payment(&currency, environment);
            let network_info = self.get_network_info(&base_currency, &network, environment);

            // Update currency aggregate
            let key = format!("{}_{}", base_currency, network);
            let aggregate =
                currency_aggregates
                    .entry(key.clone())
                    .or_insert_with(|| NetworkBalance {
                        currency: base_currency.clone(),
                        network: network.clone(),
                        symbol: network_info.symbol.clone(),
                        available: "0".to_string(),
                        pending: "0".to_string(),
                        total: "0".to_string(),
                        decimals: network_info.decimals,
                        updated_at: Utc::now(),
                    });

            let total = Decimal::from_str(&aggregate.total).unwrap_or(Decimal::ZERO);
            aggregate.total = (total + payment.amount).to_string();
            if let Some(updated_at) = payment.updated_at {
                aggregate.updated_at = updated_at.with_timezone(&Utc);
            }

            // Update wallet breakdown
            let wallet_key = format!("{}_{}", wallet_addr, currency);
            let wallet_entry = wallet_breakdown_map
                .entry(wallet_key.clone())
                .or_insert_with(|| {
                    WalletBalance {
                        wallet_id: payment.id.clone(), // Use payment ID as placeholder
                        wallet_address: wallet_addr.clone(),
                        currency: base_currency.clone(),
                        network: network.clone(),
                        available_balance: "0".to_string(),
                        usd_value: None,
                        payment_source_id: Some(payment.id.clone()),
                        created_at: payment.created_at.with_timezone(&Utc),
                        last_transaction_at: payment.updated_at.map(|t| t.with_timezone(&Utc)),
                    }
                });

            let wallet_total =
                Decimal::from_str(&wallet_entry.available_balance).unwrap_or(Decimal::ZERO);
            wallet_entry.available_balance = (wallet_total + payment.amount).to_string();
            if let Some(updated_at) = payment.updated_at {
                wallet_entry.last_transaction_at = Some(updated_at.with_timezone(&Utc));
            }

            // Apply wallet-level deductions if available
            let deducted_amount = wallet_withdrawal_deductions
                .get(&payment.id)
                .copied()
                .unwrap_or(Decimal::ZERO);
            let final_balance =
                (wallet_total + payment.amount).max(Decimal::ZERO) - deducted_amount;
            wallet_entry.available_balance = final_balance.max(Decimal::ZERO).to_string();

            // Calculate USD value based on final balance
            let usd_value = self
                .calculate_usd_value(&base_currency, final_balance)
                .await;
            wallet_entry.usd_value = usd_value.map(|v| v.to_string());
        }

        // Apply withdrawal deductions to network-level aggregates
        for (key, aggregate) in currency_aggregates.iter_mut() {
            let total_deposits = Decimal::from_str(&aggregate.total).unwrap_or(Decimal::ZERO);
            let withdrawal_deduction = network_withdrawal_deductions
                .get(key)
                .copied()
                .unwrap_or(Decimal::ZERO);
            let fee_deduction = network_fees.get(key).copied().unwrap_or(Decimal::ZERO);
            let total_deduction = withdrawal_deduction + fee_deduction;

            // Calculate available balance (total deposits - withdrawals - fees)
            let available = (total_deposits - total_deduction).max(Decimal::ZERO);

            // Handle edge case where deductions exceed deposits
            if total_deduction > total_deposits && total_deposits > Decimal::ZERO {
                log::warn!(" Anomaly detected for {}: deductions ({}) exceed deposits ({}). Clamping to 0.",
                          key, total_deduction, total_deposits);
            }

            aggregate.available = available.to_string();
            aggregate.pending = "0".to_string(); // TODO: Track actual pending amounts from processing withdrawals
            aggregate.total = total_deposits.to_string();

            log::info!(
                "💎 Balance for {}: deposits={}, withdrawals={}, fees={}, available={}",
                key,
                total_deposits,
                withdrawal_deduction,
                fee_deduction,
                available
            );
        }

        // Filter out zero balances from network aggregates unless they have pending amounts
        let network_balances: Vec<NetworkBalance> = currency_aggregates
            .into_values()
            .filter(|balance| {
                let available = Decimal::from_str(&balance.available).unwrap_or(Decimal::ZERO);
                let pending = Decimal::from_str(&balance.pending).unwrap_or(Decimal::ZERO);
                available > Decimal::ZERO || pending > Decimal::ZERO
            })
            .collect();

        // Filter out zero balances from wallet breakdown
        let wallet_breakdown: Vec<WalletBalance> = wallet_breakdown_map
            .into_values()
            .filter(|wallet| {
                let available =
                    Decimal::from_str(&wallet.available_balance).unwrap_or(Decimal::ZERO);
                available > Decimal::ZERO
            })
            .collect();
        let total_wallets = wallet_addresses.len() as u64;

        log::info!(
            "Calculated balances from {} payments: {} currency aggregates, {} unique wallets",
            payments.len(),
            network_balances.len(),
            total_wallets
        );

        Ok((network_balances, wallet_breakdown, total_wallets))
    }

    /// Extract currency and network from payment currency field
    fn extract_currency_and_network_from_payment(
        &self,
        payment_currency: &str,
        _environment: &str,
    ) -> (String, String) {
        match payment_currency.to_lowercase().as_str() {
            "eth" => ("eth".to_string(), "eth".to_string()),
            "btc" => ("btc".to_string(), "btc".to_string()),
            "sol" => ("sol".to_string(), "sol".to_string()),
            "bnb" => ("bnb".to_string(), "bnb".to_string()),
            "usdt" => ("usdt".to_string(), "usdt_erc20".to_string()),
            "usdt_bnb" => ("usdt_bnb".to_string(), "usdt_bep20".to_string()),
            _ => {
                let base = payment_currency
                    .split('_')
                    .next()
                    .unwrap_or(payment_currency);
                (base.to_string(), base.to_string())
            }
        }
    }

    /// Extract currency and network from wallet currency
    fn extract_currency_and_network(
        &self,
        wallet_currency: &str,
        environment: &str,
    ) -> (String, String) {
        // Remove environment suffix
        let currency_without_env = wallet_currency
            .strip_suffix(&format!("_{}", environment))
            .unwrap_or(wallet_currency);

        // Handle specific currency mappings
        match currency_without_env.to_lowercase().as_str() {
            "usdt_bnb" => ("usdt_bnb".to_string(), "usdt_bep20".to_string()),
            "usdt" => ("usdt".to_string(), "usdt_erc20".to_string()),
            "eth" => ("eth".to_string(), "eth".to_string()),
            "btc" => ("btc".to_string(), "btc".to_string()),
            "sol" => ("sol".to_string(), "sol".to_string()),
            "bnb" => ("bnb".to_string(), "bnb".to_string()),
            _ => {
                let base = currency_without_env
                    .split('_')
                    .next()
                    .unwrap_or(currency_without_env);
                (base.to_string(), base.to_string())
            }
        }
    }

    /// Get network info for currency
    fn get_network_info(&self, currency: &str, _network: &str, _environment: &str) -> NetworkInfo {
        match currency.to_lowercase().as_str() {
            "eth" => NetworkInfo {
                symbol: "ETH".to_string(),
                decimals: 18,
            },
            "btc" => NetworkInfo {
                symbol: "BTC".to_string(),
                decimals: 8,
            },
            "sol" => NetworkInfo {
                symbol: "SOL".to_string(),
                decimals: 9,
            },
            "bnb" => NetworkInfo {
                symbol: "BNB".to_string(),
                decimals: 18,
            },
            "usdt_bnb" => NetworkInfo {
                symbol: "USDT".to_string(),
                decimals: 18,
            },
            "usdt" => NetworkInfo {
                symbol: "USDT".to_string(),
                decimals: 6,
            },
            _ => NetworkInfo {
                symbol: currency.to_uppercase(),
                decimals: 18,
            },
        }
    }

    /// Calculate USD value for amount
    async fn calculate_usd_value(&self, currency: &str, amount: Decimal) -> Option<Decimal> {
        // Use cached prices from DB/Oracle, no blocking external calls
        match PRICE_ORACLE.get_usd_price(currency).await {
            Ok(price) => {
                let price_decimal = Decimal::from_str(&price.to_string()).ok()?;
                Some(amount * price_decimal)
            }
            Err(_) => None, // Return null if price missing
        }
    }

    /// Calculate total USD value based on available balances (after deductions)
    async fn calculate_total_usd_value(&self, balances: &[NetworkBalance]) -> Decimal {
        let mut total = Decimal::ZERO;

        for balance in balances {
            // Use available balance (post-deduction) for USD calculation, not total
            if let Ok(amount) = Decimal::from_str(&balance.available) {
                if amount > Decimal::ZERO {
                    if let Some(usd_value) =
                        self.calculate_usd_value(&balance.currency, amount).await
                    {
                        total += usd_value;
                    }
                }
            }
        }

        total
    }

    /// Map database errors to balances errors
    fn map_db_error(&self, err: DbErr) -> BalancesError {
        match &err {
            DbErr::Conn(_) => BalancesError::db_unavailable(Some(serde_json::json!({
                "error": err.to_string()
            }))),
            DbErr::ConnectionAcquire(_) => BalancesError::db_unavailable(Some(serde_json::json!({
                "error": "Failed to acquire database connection"
            }))),
            _ => BalancesError::db_unavailable(Some(serde_json::json!({
                "error": err.to_string()
            }))),
        }
    }

    /// Cache management
    async fn get_from_cache(&self, key: &str) -> Option<BalancesResponse> {
        let cache = self.cache.read().await;
        if let Some(entry) = cache.get(key) {
            let age = Utc::now().signed_duration_since(entry.cached_at);
            if age.num_seconds() < self.cache_ttl_seconds as i64 {
                return Some(entry.data.clone());
            }
        }
        None
    }

    async fn cache_result(&self, key: &str, data: BalancesResponse) {
        let mut cache = self.cache.write().await;
        cache.insert(
            key.to_string(),
            BalanceCacheEntry {
                data,
                cached_at: Utc::now(),
            },
        );
    }

    /// Structured logging
    async fn log_error(
        &self,
        route: &str,
        merchant_id: &str,
        environment: &str,
        error: &BalancesError,
    ) {
        let schema_version =
            std::env::var("DB_SCHEMA_VERSION").unwrap_or_else(|_| "unknown".to_string());

        log::error!(
            "Balances service error: route={} merchant_id={} environment={} err_type={} err={} schema_version={}",
            route,
            merchant_id,
            environment,
            error.code,
            error.message,
            schema_version
        );
    }
}

#[derive(Debug)]
struct NetworkInfo {
    symbol: String,
    decimals: u8,
}

impl Default for BalancesService {
    fn default() -> Self {
        Self::new()
    }
}
