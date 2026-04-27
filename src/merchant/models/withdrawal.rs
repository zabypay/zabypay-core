use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use validator::Validate;

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreateWithdrawalRequest {
    #[validate(custom(function = "validate_environment"))]
    pub environment: String,

    #[validate(custom(function = "validate_network"))]
    pub network: String,

    #[validate(length(
        min = 10,
        max = 100,
        message = "Address must be between 10 and 100 characters"
    ))]
    pub to_address: String,

    #[validate(custom(function = "validate_amount"))]
    pub amount: String,

    pub amount_type: Option<String>,

    #[validate(length(
        min = 1,
        max = 255,
        message = "Idempotency key must be between 1 and 255 characters"
    ))]
    pub idempotency_key: String,

    pub external_id: Option<String>,

    pub wallet_selection_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WithdrawalResponse {
    pub id: String,
    pub merchant_id: String,
    pub external_id: Option<String>,
    pub idempotency_key: String,
    pub environment: String,
    pub network: String,
    pub currency: String,
    pub to_address: String,
    pub amount: String,
    pub requested_amount: String,
    pub transferred_amount: String,
    pub amount_type: String,
    pub usd_equivalent: Option<String>,
    pub fee: Option<String>,
    pub total_fee: Option<String>,
    pub net_amount: String,
    pub status: String,
    pub wallets_used: Vec<WalletUsage>,
    pub tx_hash: Option<String>,
    pub tx_hashes: Vec<String>,
    pub confirmations: i32,
    pub blockchain_confirmations: Option<i32>,
    pub required_confirmations: i32,
    pub explorer_url: Option<String>,
    pub explorer_urls: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub broadcast_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WalletUsage {
    pub wallet_id: String,
    pub wallet_address: String,
    pub amount_used: String,
    pub fee_paid: String,
    pub tx_hash: Option<String>,
    pub status: String,
    pub balance_before: String,
    pub balance_after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkBalance {
    pub currency: String,
    pub network: String,
    pub symbol: String,
    pub available: String,
    pub pending: String,
    pub total: String,
    pub decimals: u8,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BalancesResponse {
    pub environment: String,
    pub balances: Vec<NetworkBalance>,
    pub wallet_breakdown: Vec<WalletBalance>,
    pub total_usd_value: Option<String>,
    pub total_wallets: u64,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WalletBalance {
    pub wallet_id: String,
    pub wallet_address: String,
    pub currency: String,
    pub network: String,
    pub available_balance: String,
    pub usd_value: Option<String>,
    pub payment_source_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_transaction_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AggregatedBalance {
    pub currency: String,
    pub network: String,
    pub total_crypto_amount: String,
    pub total_usd_value: String,
    pub wallet_count: u64,
    pub wallets: Vec<WalletBalance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WithdrawalPlan {
    pub withdrawal_id: String,
    pub requested_amount: String,
    pub amount_type: String,
    pub total_available: String,
    pub selected_wallets: Vec<SelectedWallet>,
    pub total_fees_estimated: String,
    pub net_transfer_amount: String,
    pub can_fulfill: bool,
    pub shortfall_amount: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectedWallet {
    pub wallet_id: String,
    pub wallet_address: String,
    pub available_balance: String,
    pub amount_to_transfer: String,
    pub estimated_fee: String,
    pub net_amount: String,
    pub order: u32,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct FeeEstimateRequest {
    #[validate(custom(function = "validate_environment"))]
    pub environment: String,

    #[validate(custom(function = "validate_network"))]
    pub network: String,

    #[validate(custom(function = "validate_amount"))]
    pub amount: String,

    #[validate(length(min = 10, max = 100))]
    pub to_address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeeEstimateResponse {
    pub network: String,
    pub currency: String,
    pub balance: String,
    pub estimated_fee: String,
    pub transferable: String,
    pub net_amount: String,
    pub decimals: u8,
    pub gas_price: Option<String>,
    pub gas_limit: Option<String>,
    pub fee_rate: Option<String>,
    pub priority: String,
    pub estimated_confirmation_time: Option<String>,
    pub safety_buffer: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct MaxWithdrawableRequest {
    #[validate(custom(function = "validate_environment"))]
    pub environment: String,

    #[validate(custom(function = "validate_network"))]
    pub network: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaxWithdrawableResponse {
    pub network: String,
    pub currency: String,
    pub max_withdrawable: String,
    pub total_balance: String,
    pub estimated_fee: String,
    pub gas_price: Option<String>,
    pub gas_limit: Option<String>,
    pub wallets_available: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WithdrawalListQuery {
    pub page: Option<u64>,

    pub limit: Option<u64>,

    pub status: Option<String>,

    pub network: Option<String>,
    pub environment: Option<String>,

    pub from_date: Option<DateTime<Utc>>,

    pub to_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BalanceQuery {
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WithdrawalListResponse {
    pub withdrawals: Vec<WithdrawalResponse>,
    pub pagination: PaginationInfo,
    pub summary: WithdrawalSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaginationInfo {
    pub page: u64,
    pub limit: u64,
    pub total: u64,
    pub total_pages: u64,
    pub has_next: bool,
    pub has_prev: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WithdrawalSummary {
    pub total_count: u64,
    pub pending_count: u64,
    pub processing_count: u64,
    pub confirmed_count: u64,
    pub failed_count: u64,
    pub total_amount_usd: Option<String>,
    pub total_fees_usd: Option<String>,
}

fn validate_environment(environment: &str) -> Result<(), validator::ValidationError> {
    match environment {
        "mainnet" | "testnet" => Ok(()),
        _ => Err(validator::ValidationError::new("invalid_environment")),
    }
}

fn validate_network(network: &str) -> Result<(), validator::ValidationError> {
    match network {
        "eth" | "bnb" | "sol" | "btc" | "usdt_erc20" | "usdt_bep20" | "base_eth" => Ok(()),
        _ => Err(validator::ValidationError::new("invalid_network")),
    }
}

fn validate_amount(amount: &str) -> Result<(), validator::ValidationError> {
    if amount == "max" {
        return Ok(());
    }

    match amount.parse::<Decimal>() {
        Ok(decimal) if decimal > Decimal::ZERO => Ok(()),
        _ => Err(validator::ValidationError::new("invalid_amount")),
    }
}

impl CreateWithdrawalRequest {
    pub fn get_amount_type(&self) -> &str {
        self.amount_type.as_deref().unwrap_or("crypto")
    }

    pub fn is_usd_withdrawal(&self) -> bool {
        self.get_amount_type() == "usd"
    }

    pub fn get_currency(&self) -> &str {
        match self.network.as_str() {
            "eth" => "ethereum",
            "bnb" => "bnb",
            "sol" => "solana",
            "btc" => "bitcoin",
            "usdt_erc20" => "usdt",
            "usdt_bep20" => "usdt_bnb",
            "base_eth" => "ethereum",
            _ => "unknown",
        }
    }

    pub fn is_max_withdrawal(&self) -> bool {
        self.amount == "max"
    }

    pub fn validate_amount_for_network(&self) -> Result<Decimal, String> {
        if self.is_max_withdrawal() {
            return Ok(Decimal::ZERO);
        }

        let amount = self
            .amount
            .parse::<Decimal>()
            .map_err(|_| "Invalid amount format".to_string())?;

        if amount <= Decimal::ZERO {
            return Err("Amount must be greater than zero".to_string());
        }

        let min_amount = match self.network.as_str() {
            "btc" => Decimal::from_str_exact("0.00000001").unwrap(), // 1000 sats
            "eth" => Decimal::from_str_exact("0.00000001").unwrap(), // 0.001 ETH
            "bnb" => Decimal::from_str_exact("0.00000001").unwrap(), // 0.01 BNB
            "sol" => Decimal::from_str_exact("0.0000001").unwrap(),  // 0.001 SOL
            "usdt_erc20" | "usdt_bep20" => Decimal::from_str_exact("0.10").unwrap(), // 1 USDT
            _ => Decimal::ZERO,
        };

        if amount < min_amount {
            return Err(format!(
                "Minimum withdrawal amount is {} {}",
                min_amount,
                self.network.to_uppercase()
            ));
        }

        let max_amount = match self.network.as_str() {
            "btc" => Decimal::from_str_exact("10.0").unwrap(),
            "eth" => Decimal::from_str_exact("100.0").unwrap(),
            "bnb" => Decimal::from_str_exact("1000.0").unwrap(),
            "sol" => Decimal::from_str_exact("10000.0").unwrap(),
            "usdt_erc20" | "usdt_bep20" => Decimal::from_str_exact("100000.0").unwrap(),
            _ => Decimal::from_str_exact("1000000.0").unwrap(),
        };

        if amount > max_amount {
            return Err(format!(
                "Maximum withdrawal amount is {} {}",
                max_amount,
                self.network.to_uppercase()
            ));
        }

        Ok(amount)
    }
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct MultiWalletWithdrawalRequest {
    #[validate(custom(function = "validate_environment"))]
    pub environment: String,

    #[validate(length(
        min = 10,
        max = 100,
        message = "Address must be between 10 and 100 characters"
    ))]
    pub to_address: String,

    #[validate(custom(function = "validate_amount"))]
    pub amount: String,

    pub amount_type: String,

    pub currency: Option<String>,

    #[validate(length(
        min = 1,
        max = 255,
        message = "Idempotency key must be between 1 and 255 characters"
    ))]
    pub idempotency_key: String,

    pub external_id: Option<String>,

    pub wallet_selection_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiWalletWithdrawalResponse {
    pub id: String,
    pub merchant_id: String,
    pub external_id: Option<String>,
    pub idempotency_key: String,
    pub environment: String,
    pub to_address: String,
    pub requested_amount: String,
    pub requested_amount_type: String,
    pub transferred_amount_usd: String,
    pub total_fees_usd: String,
    pub net_amount_usd: String,
    pub status: String,
    pub wallets_used: u64,
    pub wallets_failed: u64,
    pub tx_hashes: Vec<String>,
    pub transactions: Vec<WithdrawalTransaction>,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WithdrawalTransaction {
    pub wallet_id: String,
    pub wallet_address: String,
    pub currency: String,
    pub amount: String,
    pub fee: String,
    pub net_amount: String,
    pub usd_equivalent: String,
    pub tx_hash: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WalletSelection {
    pub wallet_id: String,
    pub address: String,
    pub currency: String,
    pub symbol: String,
    pub amount_crypto: String,
    pub amount_usd: String,
    pub estimated_fee: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WithdrawalPreview {
    pub requested_amount: String,
    pub requested_amount_type: String,
    pub requested_usd_equivalent: String,
    pub total_available_usd: String,
    pub selected_wallets: u64,
    pub total_selected_usd: String,
    pub estimated_total_fees_usd: String,
    pub net_amount_usd: String,
    pub wallet_selections: Vec<WalletSelection>,
    pub can_fulfill: bool,
    pub environment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiWalletBalanceResponse {
    pub environment: String,
    pub total_wallets: u64,
    pub total_usd_value: String,
    pub wallet_balances: Vec<MultiWalletBalance>,
    pub currency_totals: HashMap<String, String>,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiWalletBalance {
    pub wallet_id: String,
    pub address: String,
    pub currency: String,
    pub network: String,
    pub symbol: String,
    pub available: String,
    pub pending: String,
    pub total: String,
    pub usd_value: String,
    pub decimals: u8,
    pub last_updated: DateTime<Utc>,
}
