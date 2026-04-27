use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use crate::shared::utils::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapRequest {
    pub from_currency: String,
    pub to_currency: String,
    pub amount: f64,
    pub from_network: String,
    pub to_network: String,
    pub recipient_address: String,
    pub slippage_tolerance: f64, // Percentage
    pub deadline: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuote {
    pub request_id: String,
    pub from_currency: String,
    pub to_currency: String,
    pub from_amount: f64,
    pub to_amount: f64,
    pub exchange_rate: f64,
    pub fee_amount: f64,
    pub fee_currency: String,
    pub estimated_time: u64, // seconds
    pub provider: String,
    pub quote_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapExecution {
    pub swap_id: String,
    pub request_id: String,
    pub status: SwapStatus,
    pub from_tx_hash: Option<String>,
    pub to_tx_hash: Option<String>,
    pub bridge_tx_hash: Option<String>,
    pub executed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwapStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeTransaction {
    pub bridge_id: String,
    pub from_network: String,
    pub to_network: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: f64,
    pub currency: String,
    pub status: BridgeStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

#[async_trait]
pub trait SwapProvider: Send + Sync {
    async fn get_quote(&self, request: &SwapRequest) -> Result<SwapQuote, AppError>;
    async fn execute_swap(&self, quote: &SwapQuote) -> Result<SwapExecution, AppError>;
    async fn get_swap_status(&self, swap_id: &str) -> Result<SwapStatus, AppError>;
    fn get_provider_name(&self) -> &str;
    fn get_supported_pairs(&self) -> Vec<(String, String)>;
}

#[async_trait]
pub trait BridgeProvider: Send + Sync {
    async fn create_bridge_transaction(&self, request: &SwapRequest) -> Result<BridgeTransaction, AppError>;
    async fn get_bridge_status(&self, bridge_id: &str) -> Result<BridgeStatus, AppError>;
    fn get_supported_bridges(&self) -> Vec<(String, String)>;
}

pub struct CrossChainSwapService {
    swap_providers: HashMap<String, Box<dyn SwapProvider>>,
    bridge_providers: HashMap<String, Box<dyn BridgeProvider>>,
    active_swaps: Arc<Mutex<HashMap<String, SwapExecution>>>,
    active_bridges: Arc<Mutex<HashMap<String, BridgeTransaction>>>,
}

impl CrossChainSwapService {
    pub fn new() -> Self {
        Self {
            swap_providers: HashMap::new(),
            bridge_providers: HashMap::new(),
            active_swaps: Arc::new(Mutex::new(HashMap::new())),
            active_bridges: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn add_swap_provider(&mut self, provider: Box<dyn SwapProvider>) {
        let provider_name = provider.get_provider_name().to_string();
        self.swap_providers.insert(provider_name, provider);
    }

    pub fn add_bridge_provider(&mut self, provider: Box<dyn BridgeProvider>) {
        let provider_name = "bridge".to_string(); // Simplified for now
        self.bridge_providers.insert(provider_name, provider);
    }

    pub async fn get_best_quote(&self, request: &SwapRequest) -> Result<SwapQuote, AppError> {
        let mut quotes = Vec::new();

        // Get quotes from all providers
        for provider in self.swap_providers.values() {
            if self.is_supported_pair(provider, request).await? {
                match provider.get_quote(request).await {
                    Ok(quote) => quotes.push(quote),
                    Err(e) => {
                        log::warn!("Failed to get quote from {}: {}", provider.get_provider_name(), e);
                    }
                }
            }
        }

        if quotes.is_empty() {
            return Err(AppError::NotFound("No swap providers available for this pair".to_string()));
        }

        // Find the best quote (highest output amount)
        let best_quote = quotes.into_iter()
            .max_by(|a, b| a.to_amount.partial_cmp(&b.to_amount).unwrap_or(std::cmp::Ordering::Equal))
            .ok_or(AppError::InternalServerError("No valid quotes found".to_string()))?;

        Ok(best_quote)
    }

    pub async fn execute_swap(&self, quote: &SwapQuote) -> Result<SwapExecution, AppError> {
        let provider_name = &quote.provider;
        let provider = self.swap_providers.get(provider_name)
            .ok_or(AppError::NotFound(format!("Provider {} not found", provider_name)))?;

        let execution = provider.execute_swap(quote).await?;
        
        // Store active swap
        {
            let mut swaps = self.active_swaps.lock().unwrap();
            swaps.insert(execution.swap_id.clone(), execution.clone());
        }
        
        Ok(execution)
    }

    pub async fn get_swap_status(&self, swap_id: &str) -> Result<SwapStatus, AppError> {
        {
            let swaps = self.active_swaps.lock().unwrap();
            if let Some(execution) = swaps.get(swap_id) {
                return Ok(execution.status.clone());
            }
        }

        // Check with all providers
        for provider in self.swap_providers.values() {
            match provider.get_swap_status(swap_id).await {
                Ok(status) => return Ok(status),
                Err(_) => continue,
            }
        }

        Err(AppError::NotFound("Swap not found".to_string()))
    }

    pub async fn create_bridge_transaction(&self, request: &SwapRequest) -> Result<BridgeTransaction, AppError> {
        let bridge_provider = self.bridge_providers.values().next()
            .ok_or(AppError::NotFound("No bridge providers available".to_string()))?;

        let bridge_tx = bridge_provider.create_bridge_transaction(request).await?;
        
        // Store active bridge
        {
            let mut bridges = self.active_bridges.lock().unwrap();
            bridges.insert(bridge_tx.bridge_id.clone(), bridge_tx.clone());
        }
        
        Ok(bridge_tx)
    }

    pub async fn get_bridge_status(&self, bridge_id: &str) -> Result<BridgeStatus, AppError> {
        {
            let bridges = self.active_bridges.lock().unwrap();
            if let Some(bridge) = bridges.get(bridge_id) {
                return Ok(bridge.status.clone());
            }
        }

        // Check with all bridge providers
        for provider in self.bridge_providers.values() {
            match provider.get_bridge_status(bridge_id).await {
                Ok(status) => return Ok(status),
                Err(_) => continue,
            }
        }

        Err(AppError::NotFound("Bridge transaction not found".to_string()))
    }

    async fn is_supported_pair(&self, provider: &Box<dyn SwapProvider>, request: &SwapRequest) -> Result<bool, AppError> {
        let supported_pairs = provider.get_supported_pairs();
        let pair = (request.from_currency.clone(), request.to_currency.clone());
        Ok(supported_pairs.contains(&pair))
    }

    pub async fn cleanup_expired_swaps(&self) -> Result<u64, AppError> {
        let _now = Utc::now();
        let mut expired_count = 0;

        {
            let swaps = self.active_swaps.lock().unwrap();
            for (_swap_id, execution) in swaps.iter() {
                if let SwapStatus::Expired = execution.status {
                    expired_count += 1;
                }
            }
        }

        Ok(expired_count)
    }
}

// 1inch Swap Provider Implementation
pub struct OneInchProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OneInchProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.1inch.dev".to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SwapProvider for OneInchProvider {
    async fn get_quote(&self, request: &SwapRequest) -> Result<SwapQuote, AppError> {
        let url = format!("{}/swap/v5.2/{}/quote", self.base_url, request.from_network);
        
        let params = serde_json::json!({
            "src": request.from_currency,
            "dst": request.to_currency,
            "amount": (request.amount * 1e18) as u64, // Convert to wei
            "from": "0x0000000000000000000000000000000000000000", // Placeholder
            "slippage": request.slippage_tolerance,
        });

        let response = self.client.get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&params)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("1inch API error: {}", e)))?;

        if response.status().is_success() {
            let data: serde_json::Value = response.json().await
                .map_err(|e| AppError::InternalServerError(format!("Failed to parse 1inch response: {}", e)))?;

            let to_amount = data.get("toAmount")
                .and_then(|ta| ta.as_str())
                .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0) as f64 / 1e18;

            let gas_estimate = data.get("tx")
                .and_then(|tx| tx.get("gas"))
                .and_then(|g| g.as_u64())
                .unwrap_or(0);

            return Ok(SwapQuote {
                request_id: uuid::Uuid::new_v4().to_string(),
                from_currency: request.from_currency.clone(),
                to_currency: request.to_currency.clone(),
                from_amount: request.amount,
                to_amount,
                exchange_rate: to_amount / request.amount,
                fee_amount: gas_estimate as f64 * 20.0 / 1e9, // Estimate gas cost
                fee_currency: "ETH".to_string(),
                estimated_time: 300, // 5 minutes
                provider: "1inch".to_string(),
                quote_expires_at: Utc::now() + chrono::Duration::minutes(5),
            });
        }

        Err(AppError::InternalServerError("Failed to get quote from 1inch".to_string()))
    }

    async fn execute_swap(&self, quote: &SwapQuote) -> Result<SwapExecution, AppError> {
        // In a real implementation, this would execute the swap
        // For now, return a mock execution
        Ok(SwapExecution {
            swap_id: uuid::Uuid::new_v4().to_string(),
            request_id: quote.request_id.clone(),
            status: SwapStatus::Pending,
            from_tx_hash: None,
            to_tx_hash: None,
            bridge_tx_hash: None,
            executed_at: None,
            error_message: None,
        })
    }

    async fn get_swap_status(&self, _swap_id: &str) -> Result<SwapStatus, AppError> {
        // In a real implementation, this would check the actual swap status
        // For now, return pending
        Ok(SwapStatus::Pending)
    }

    fn get_provider_name(&self) -> &str {
        "1inch"
    }

    fn get_supported_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("ETH".to_string(), "USDT".to_string()),
            ("USDT".to_string(), "ETH".to_string()),
            ("ETH".to_string(), "USDC".to_string()),
            ("USDC".to_string(), "ETH".to_string()),
        ]
    }
}

// Wormhole Bridge Provider Implementation
pub struct WormholeBridgeProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl WormholeBridgeProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.wormhole.com".to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl BridgeProvider for WormholeBridgeProvider {
    async fn create_bridge_transaction(&self, request: &SwapRequest) -> Result<BridgeTransaction, AppError> {
        // In a real implementation, this would create a bridge transaction
        // For now, return a mock bridge transaction
        Ok(BridgeTransaction {
            bridge_id: uuid::Uuid::new_v4().to_string(),
            from_network: request.from_network.clone(),
            to_network: request.to_network.clone(),
            from_address: "0x0000000000000000000000000000000000000000".to_string(),
            to_address: request.recipient_address.clone(),
            amount: request.amount,
            currency: request.from_currency.clone(),
            status: BridgeStatus::Pending,
            created_at: Utc::now(),
            completed_at: None,
        })
    }

    async fn get_bridge_status(&self, _bridge_id: &str) -> Result<BridgeStatus, AppError> {
        // In a real implementation, this would check the actual bridge status
        // For now, return pending
        Ok(BridgeStatus::Pending)
    }

    fn get_supported_bridges(&self) -> Vec<(String, String)> {
        vec![
            ("ethereum".to_string(), "solana".to_string()),
            ("solana".to_string(), "ethereum".to_string()),
            ("ethereum".to_string(), "polygon".to_string()),
            ("polygon".to_string(), "ethereum".to_string()),
        ]
    }
}

// HTLC (Hashed Time-Locked Contract) Implementation
pub struct HTLCService {
    contracts: Arc<Mutex<HashMap<String, HTLCContract>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HTLCContract {
    pub contract_id: String,
    pub sender_address: String,
    pub recipient_address: String,
    pub amount: f64,
    pub currency: String,
    pub hash_lock: String,
    pub time_lock: DateTime<Utc>,
    pub status: HTLCStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HTLCStatus {
    Pending,
    Funded,
    Completed,
    Expired,
    Refunded,
}

impl HTLCService {
    pub fn new() -> Self {
        Self {
            contracts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_htlc(&self, sender: &str, recipient: &str, amount: f64, currency: &str, time_lock_hours: u64) -> Result<HTLCContract, AppError> {
        let contract_id = uuid::Uuid::new_v4().to_string();
        let hash_lock = self.generate_hash_lock();
        let time_lock = Utc::now() + chrono::Duration::hours(time_lock_hours as i64);

        let contract = HTLCContract {
            contract_id: contract_id.clone(),
            sender_address: sender.to_string(),
            recipient_address: recipient.to_string(),
            amount,
            currency: currency.to_string(),
            hash_lock,
            time_lock,
            status: HTLCStatus::Pending,
            created_at: Utc::now(),
        };

        // In a real implementation, this would deploy the HTLC contract
        // For now, just store it in memory
        {
            let mut contracts = self.contracts.lock().unwrap();
            contracts.insert(contract_id, contract.clone());
        }
        
        Ok(contract)
    }

    pub async fn fund_htlc(&self, contract_id: &str) -> Result<(), AppError> {
        {
            let mut contracts = self.contracts.lock().unwrap();
            if let Some(contract) = contracts.get_mut(contract_id) {
                contract.status = HTLCStatus::Funded;
                Ok(())
            } else {
                Err(AppError::NotFound("HTLC contract not found".to_string()))
            }
        }
    }

    pub async fn complete_htlc(&self, contract_id: &str, secret: &str) -> Result<(), AppError> {
        {
            let mut contracts = self.contracts.lock().unwrap();
            if let Some(contract) = contracts.get_mut(contract_id) {
                if self.verify_secret(secret, &contract.hash_lock) {
                    contract.status = HTLCStatus::Completed;
                    Ok(())
                } else {
                    Err(AppError::ValidationError("Invalid secret".to_string()))
                }
            } else {
                Err(AppError::NotFound("HTLC contract not found".to_string()))
            }
        }
    }

    pub async fn refund_htlc(&self, contract_id: &str) -> Result<(), AppError> {
        {
            let mut contracts = self.contracts.lock().unwrap();
            if let Some(contract) = contracts.get_mut(contract_id) {
                if contract.time_lock < Utc::now() {
                    contract.status = HTLCStatus::Refunded;
                    Ok(())
                } else {
                    Err(AppError::ValidationError("Time lock not expired".to_string()))
                }
            } else {
                Err(AppError::NotFound("HTLC contract not found".to_string()))
            }
        }
    }

    fn generate_hash_lock(&self) -> String {
        use sha2::{Sha256, Digest};
        let random_bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        let mut hasher = Sha256::new();
        hasher.update(&random_bytes);
        format!("{:x}", hasher.finalize())
    }

    fn verify_secret(&self, secret: &str, hash_lock: &str) -> bool {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(secret.as_bytes());
        let computed_hash = format!("{:x}", hasher.finalize());
        computed_hash == hash_lock
    }
} 