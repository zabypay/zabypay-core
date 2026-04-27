use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MerchantEnvironmentType {
    #[serde(rename = "mainnet")]
    Mainnet,
    #[serde(rename = "testnet")]
    Testnet,
}

impl Default for MerchantEnvironmentType {
    fn default() -> Self {
        MerchantEnvironmentType::Mainnet
    }
}

impl From<MerchantEnvironmentType> for String {
    fn from(env_type: MerchantEnvironmentType) -> Self {
        match env_type {
            MerchantEnvironmentType::Mainnet => "mainnet".to_string(),
            MerchantEnvironmentType::Testnet => "testnet".to_string(),
        }
    }
}

impl From<String> for MerchantEnvironmentType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "testnet" => MerchantEnvironmentType::Testnet,
            _ => MerchantEnvironmentType::Mainnet,
        }
    }
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateMerchantRequest {
    #[validate(length(
        min = 1,
        max = 100,
        message = "Merchant name must be between 1 and 100 characters"
    ))]
    pub name: String,

    #[validate(length(max = 500, message = "Description must be less than 500 characters"))]
    pub description: Option<String>,

    #[validate(url(message = "Invalid website URL"))]
    pub website_url: Option<String>,

    #[validate(url(message = "Invalid webhook URL"))]
    pub webhook_url: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateMerchantRequest {
    #[validate(length(
        min = 1,
        max = 100,
        message = "Merchant name must be between 1 and 100 characters"
    ))]
    pub name: Option<String>,

    #[validate(length(max = 500, message = "Description must be less than 500 characters"))]
    pub description: Option<String>,

    #[validate(url(message = "Invalid website URL"))]
    pub website_url: Option<String>,

    #[validate(url(message = "Invalid webhook URL"))]
    pub webhook_url: Option<String>,

    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct MerchantResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub website_url: Option<String>,
    pub webhook_url: Option<String>,
    pub environment_type: MerchantEnvironmentType,
    pub preferred_networks: Vec<String>,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateApiKeyRequest {
    #[validate(length(
        min = 1,
        max = 50,
        message = "API key name must be between 1 and 50 characters"
    ))]
    pub name: String,
    pub environment_type: Option<MerchantEnvironmentType>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub api_key: Option<String>,
    pub secret_key: Option<String>,
    pub environment_type: MerchantEnvironmentType,
    pub is_active: bool,
    pub created_at: String,
    pub last_used: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyCreatedResponse {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub secret_key: String,
    pub environment_type: MerchantEnvironmentType,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct PayoutSettingsRequest {
    pub crypto_addresses: HashMap<String, String>,
    pub auto_convert_rules: Vec<ConversionRule>,
    pub settlement_frequency: SettlementFrequency,
    pub bank_account: Option<BankAccount>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConversionRule {
    pub from_currency: String,
    pub to_currency: String,
    pub min_amount: Option<f64>,
    pub max_amount: Option<f64>,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum SettlementFrequency {
    Immediate,
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BankAccount {
    pub account_number: String,
    pub routing_number: String,
    pub account_type: BankAccountType,
    pub bank_name: String,
    pub account_holder_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum BankAccountType {
    Checking,
    Savings,
}

#[derive(Debug, Serialize)]
pub struct PayoutSettingsResponse {
    pub merchant_id: String,
    pub crypto_addresses: HashMap<String, String>,
    pub auto_convert_rules: Vec<ConversionRule>,
    pub settlement_frequency: SettlementFrequency,
    pub bank_account: Option<BankAccount>,
    pub updated_at: String,
}
