use crate::shared::utils::errors::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: f64,
    pub currency: String,
    pub block_number: Option<u64>,
    pub block_hash: Option<String>,
    pub confirmations: u32,
    pub status: TransactionStatus,
    pub gas_used: Option<u64>,
    pub gas_price: Option<String>,
    pub network: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransactionStatus {
    Pending,
    Confirmed,
    Failed,
    Dropped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    pub network: String,
    pub required_confirmations: u32,
    pub polling_interval_seconds: u64,
    pub rpc_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub transaction_hash: String,
    pub block_number: u64,
    pub block_hash: String,
    pub gas_used: u64,
    pub status: bool,
    pub logs: Vec<Log>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Log {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
}

// Define an enum to hold different network implementations
#[derive(Debug)]
pub enum NetworkProvider {
    Ethereum(EthereumNetwork),
    Bitcoin(BitcoinNetwork),
    Solana(SolanaNetwork),
}

impl NetworkProvider {
    pub async fn get_transaction(&self, tx_hash: &str) -> Result<Transaction, AppError> {
        match self {
            NetworkProvider::Ethereum(network) => network.get_transaction(tx_hash).await,
            NetworkProvider::Bitcoin(network) => network.get_transaction(tx_hash).await,
            NetworkProvider::Solana(network) => network.get_transaction(tx_hash).await,
        }
    }

    pub async fn get_latest_block(&self) -> Result<u64, AppError> {
        match self {
            NetworkProvider::Ethereum(network) => network.get_latest_block().await,
            NetworkProvider::Bitcoin(network) => network.get_latest_block().await,
            NetworkProvider::Solana(network) => network.get_latest_block().await,
        }
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<TransactionReceipt, AppError> {
        match self {
            NetworkProvider::Ethereum(network) => network.get_transaction_receipt(tx_hash).await,
            NetworkProvider::Bitcoin(network) => network.get_transaction_receipt(tx_hash).await,
            NetworkProvider::Solana(network) => network.get_transaction_receipt(tx_hash).await,
        }
    }

    pub async fn get_confirmations(&self, tx_hash: &str) -> Result<u32, AppError> {
        match self {
            NetworkProvider::Ethereum(network) => network.get_confirmations(tx_hash).await,
            NetworkProvider::Bitcoin(network) => network.get_confirmations(tx_hash).await,
            NetworkProvider::Solana(network) => network.get_confirmations(tx_hash).await,
        }
    }

    pub fn get_network_name(&self) -> &str {
        match self {
            NetworkProvider::Ethereum(network) => network.get_network_name(),
            NetworkProvider::Bitcoin(network) => network.get_network_name(),
            NetworkProvider::Solana(network) => network.get_network_name(),
        }
    }
}

pub struct BlockchainMonitor {
    networks: HashMap<String, NetworkProvider>,
    configs: HashMap<String, MonitoringConfig>,
    monitored_transactions: Arc<RwLock<HashMap<String, MonitoredTransaction>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredTransaction {
    pub payment_request_id: String,
    pub tx_hash: String,
    pub network: String,
    pub required_confirmations: u32,
    pub created_at: DateTime<Utc>,
    pub last_checked: DateTime<Utc>,
    pub status: TransactionStatus,
}

impl BlockchainMonitor {
    pub fn new() -> Self {
        Self {
            networks: HashMap::new(),
            configs: HashMap::new(),
            monitored_transactions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_network(&mut self, network: NetworkProvider, config: MonitoringConfig) {
        let network_name = network.get_network_name().to_string();
        self.networks.insert(network_name.clone(), network);
        self.configs.insert(network_name, config);
    }

    pub async fn start_monitoring(
        &self,
        payment_request_id: &str,
        tx_hash: &str,
        network: &str,
    ) -> Result<(), AppError> {
        let config = self.configs.get(network).ok_or(AppError::NotFound(format!(
            "Network {} not configured",
            network
        )))?;

        let monitored_tx = MonitoredTransaction {
            payment_request_id: payment_request_id.to_string(),
            tx_hash: tx_hash.to_string(),
            network: network.to_string(),
            required_confirmations: config.required_confirmations,
            created_at: Utc::now(),
            last_checked: Utc::now(),
            status: TransactionStatus::Pending,
        };

        {
            let mut monitored = self.monitored_transactions.write().await;
            monitored.insert(tx_hash.to_string(), monitored_tx);
        }

        log::info!(
            "Started monitoring transaction {} on network {}",
            tx_hash,
            network
        );
        Ok(())
    }

    pub async fn check_transaction_status(
        &self,
        tx_hash: &str,
    ) -> Result<TransactionStatus, AppError> {
        let monitored = self.monitored_transactions.read().await;
        let monitored_tx = monitored.get(tx_hash).ok_or(AppError::NotFound(
            "Transaction not being monitored".to_string(),
        ))?;

        let network = self
            .networks
            .get(&monitored_tx.network)
            .ok_or(AppError::NotFound("Network not found".to_string()))?;

        let confirmations = network.get_confirmations(tx_hash).await?;

        let status = if confirmations >= monitored_tx.required_confirmations {
            TransactionStatus::Confirmed
        } else {
            TransactionStatus::Pending
        };

        // Update monitored transaction
        {
            let mut monitored = self.monitored_transactions.write().await;
            if let Some(tx) = monitored.get_mut(tx_hash) {
                tx.status = status.clone();
                tx.last_checked = Utc::now();
            }
        }

        Ok(status)
    }

    pub async fn get_monitored_transactions(&self) -> Vec<MonitoredTransaction> {
        let monitored = self.monitored_transactions.read().await;
        monitored.values().cloned().collect()
    }

    pub async fn stop_monitoring(&self, tx_hash: &str) -> Result<(), AppError> {
        let mut monitored = self.monitored_transactions.write().await;
        monitored.remove(tx_hash).ok_or(AppError::NotFound(
            "Transaction not being monitored".to_string(),
        ))?;

        log::info!("Stopped monitoring transaction {}", tx_hash);
        Ok(())
    }

    pub async fn cleanup_old_transactions(&self, max_age_hours: u64) -> Result<u64, AppError> {
        let cutoff = Utc::now() - chrono::Duration::hours(max_age_hours as i64);
        let mut monitored = self.monitored_transactions.write().await;

        let initial_count = monitored.len();
        monitored.retain(|_, tx| tx.created_at > cutoff);

        Ok((initial_count - monitored.len()) as u64)
    }
}

// Ethereum Network Implementation
#[derive(Debug)]
pub struct EthereumNetwork {
    rpc_url: String,
    client: reqwest::Client,
}

impl EthereumNetwork {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_transaction(&self, tx_hash: &str) -> Result<Transaction, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionByHash",
            "params": [tx_hash],
            "id": 1
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse response: {}", e))
        })?;

        // Parse Ethereum transaction response
        if let Some(result) = json.get("result") {
            if result.is_null() {
                return Err(AppError::NotFound("Transaction not found".to_string()));
            }

            Ok(Transaction {
                hash: tx_hash.to_string(),
                from_address: result
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                to_address: result
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                amount: 0.0, // Would need to parse value and convert from wei
                currency: "ETH".to_string(),
                block_number: result
                    .get("blockNumber")
                    .and_then(|v| v.as_str())
                    .and_then(|s| u64::from_str_radix(&s[2..], 16).ok()),
                block_hash: result
                    .get("blockHash")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                confirmations: 0, // Will be calculated separately
                status: TransactionStatus::Pending,
                gas_used: result
                    .get("gas")
                    .and_then(|v| v.as_str())
                    .and_then(|s| u64::from_str_radix(&s[2..], 16).ok()),
                gas_price: result
                    .get("gasPrice")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                network: "ethereum".to_string(),
                timestamp: Utc::now(),
            })
        } else {
            Err(AppError::InternalServerError(
                "Invalid RPC response".to_string(),
            ))
        }
    }

    pub async fn get_latest_block(&self) -> Result<u64, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse response: {}", e))
        })?;

        if let Some(result) = json.get("result").and_then(|v| v.as_str()) {
            u64::from_str_radix(&result[2..], 16).map_err(|e| {
                AppError::InternalServerError(format!("Failed to parse block number: {}", e))
            })
        } else {
            Err(AppError::InternalServerError(
                "Invalid RPC response".to_string(),
            ))
        }
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<TransactionReceipt, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionReceipt",
            "params": [tx_hash],
            "id": 1
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("RPC request failed: {}", e)))?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse response: {}", e))
        })?;

        if let Some(result) = json.get("result") {
            if result.is_null() {
                return Err(AppError::NotFound(
                    "Transaction receipt not found".to_string(),
                ));
            }

            Ok(TransactionReceipt {
                transaction_hash: tx_hash.to_string(),
                block_number: result
                    .get("blockNumber")
                    .and_then(|v| v.as_str())
                    .and_then(|s| u64::from_str_radix(&s[2..], 16).ok())
                    .unwrap_or(0),
                block_hash: result
                    .get("blockHash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                gas_used: result
                    .get("gasUsed")
                    .and_then(|v| v.as_str())
                    .and_then(|s| u64::from_str_radix(&s[2..], 16).ok())
                    .unwrap_or(0),
                status: result
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "0x1")
                    .unwrap_or(false),
                logs: vec![], // Would need to parse logs array
            })
        } else {
            Err(AppError::InternalServerError(
                "Invalid RPC response".to_string(),
            ))
        }
    }

    pub async fn get_confirmations(&self, tx_hash: &str) -> Result<u32, AppError> {
        let receipt = self.get_transaction_receipt(tx_hash).await?;
        let latest_block = self.get_latest_block().await?;

        if receipt.block_number == 0 {
            Ok(0) // Transaction not yet mined
        } else {
            Ok((latest_block - receipt.block_number) as u32 + 1)
        }
    }

    pub fn get_network_name(&self) -> &str {
        "ethereum"
    }
}

// Bitcoin Network Implementation (placeholder)
#[derive(Debug)]
pub struct BitcoinNetwork {
    rpc_url: String,
    client: reqwest::Client,
}

impl BitcoinNetwork {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_transaction(&self, tx_hash: &str) -> Result<Transaction, AppError> {
        // Use BlockCypher API for Bitcoin transactions
        let url = format!("{}/txs/{}", self.rpc_url, tx_hash);

        let response = self.client.get(&url).send().await.map_err(|e| {
            AppError::InternalServerError(format!("Bitcoin API request failed: {}", e))
        })?;

        if !response.status().is_success() {
            return Err(AppError::NotFound(
                "Bitcoin transaction not found".to_string(),
            ));
        }

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse Bitcoin response: {}", e))
        })?;

        // Parse Bitcoin transaction response
        Ok(Transaction {
            hash: tx_hash.to_string(),
            from_address: json
                .get("inputs")
                .and_then(|inputs| inputs.as_array())
                .and_then(|arr| arr.first())
                .and_then(|input| input.get("addresses"))
                .and_then(|addrs| addrs.as_array())
                .and_then(|arr| arr.first())
                .and_then(|addr| addr.as_str())
                .unwrap_or("")
                .to_string(),
            to_address: json
                .get("outputs")
                .and_then(|outputs| outputs.as_array())
                .and_then(|arr| arr.first())
                .and_then(|output| output.get("addresses"))
                .and_then(|addrs| addrs.as_array())
                .and_then(|arr| arr.first())
                .and_then(|addr| addr.as_str())
                .unwrap_or("")
                .to_string(),
            amount: json.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as f64 / 100_000_000.0, // Convert satoshis to BTC
            currency: "BTC".to_string(),
            block_number: json.get("block_height").and_then(|v| v.as_u64()),
            block_hash: json
                .get("block_hash")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            confirmations: json
                .get("confirmations")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            status: if json
                .get("confirmations")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                > 0
            {
                TransactionStatus::Confirmed
            } else {
                TransactionStatus::Pending
            },
            gas_used: None,
            gas_price: None,
            network: "bitcoin".to_string(),
            timestamp: Utc::now(), // Would need to parse from API
        })
    }

    pub async fn get_latest_block(&self) -> Result<u64, AppError> {
        let url = format!("{}", self.rpc_url);

        let response = self.client.get(&url).send().await.map_err(|e| {
            AppError::InternalServerError(format!("Bitcoin API request failed: {}", e))
        })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse Bitcoin response: {}", e))
        })?;

        json.get("height")
            .and_then(|v| v.as_u64())
            .ok_or(AppError::InternalServerError(
                "Failed to get latest Bitcoin block".to_string(),
            ))
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<TransactionReceipt, AppError> {
        // Bitcoin doesn't have receipts like Ethereum, but we can simulate it
        let tx = self.get_transaction(tx_hash).await?;

        Ok(TransactionReceipt {
            transaction_hash: tx_hash.to_string(),
            block_number: tx.block_number.unwrap_or(0),
            block_hash: tx.block_hash.unwrap_or_default(),
            gas_used: 0, // Bitcoin doesn't use gas
            status: matches!(tx.status, TransactionStatus::Confirmed),
            logs: vec![], // Bitcoin doesn't have logs
        })
    }

    pub async fn get_confirmations(&self, tx_hash: &str) -> Result<u32, AppError> {
        let tx = self.get_transaction(tx_hash).await?;
        Ok(tx.confirmations)
    }

    pub fn get_network_name(&self) -> &str {
        "bitcoin"
    }
}

// Solana Network Implementation (placeholder)
#[derive(Debug)]
pub struct SolanaNetwork {
    rpc_url: String,
    client: reqwest::Client,
}

impl SolanaNetwork {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_transaction(&self, tx_hash: &str) -> Result<Transaction, AppError> {
        // Use Solana JSON RPC to get transaction
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [
                tx_hash,
                {
                    "encoding": "json",
                    "maxSupportedTransactionVersion": 0
                }
            ]
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Solana RPC request failed: {}", e))
            })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse Solana response: {}", e))
        })?;

        if let Some(result) = json.get("result") {
            if result.is_null() {
                return Err(AppError::NotFound(
                    "Solana transaction not found".to_string(),
                ));
            }

            // Parse Solana transaction
            let transaction = result.get("transaction").unwrap_or(result);
            let meta = result.get("meta");

            Ok(Transaction {
                hash: tx_hash.to_string(),
                from_address: transaction
                    .get("message")
                    .and_then(|msg| msg.get("accountKeys"))
                    .and_then(|keys| keys.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|addr| addr.as_str())
                    .unwrap_or("")
                    .to_string(),
                to_address: transaction
                    .get("message")
                    .and_then(|msg| msg.get("accountKeys"))
                    .and_then(|keys| keys.as_array())
                    .and_then(|arr| arr.get(1))
                    .and_then(|addr| addr.as_str())
                    .unwrap_or("")
                    .to_string(),
                amount: meta
                    .and_then(|m| m.get("preBalances"))
                    .and_then(|pre| pre.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|bal| bal.as_u64())
                    .unwrap_or(0) as f64
                    / 1_000_000_000.0, // Convert lamports to SOL
                currency: "SOL".to_string(),
                block_number: result.get("slot").and_then(|v| v.as_u64()),
                block_hash: result
                    .get("block_hash")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                confirmations: if meta.and_then(|m| m.get("err")).is_none() {
                    32
                } else {
                    0
                }, // Solana finality
                status: if meta.and_then(|m| m.get("err")).is_none() {
                    TransactionStatus::Confirmed
                } else {
                    TransactionStatus::Failed
                },
                gas_used: meta.and_then(|m| m.get("fee")).and_then(|v| v.as_u64()),
                gas_price: None,
                network: "solana".to_string(),
                timestamp: Utc::now(), // Would need to get from block data
            })
        } else {
            Err(AppError::InternalServerError(
                "Invalid Solana RPC response".to_string(),
            ))
        }
    }

    pub async fn get_latest_block(&self) -> Result<u64, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSlot",
            "params": []
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Solana RPC request failed: {}", e))
            })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to parse Solana response: {}", e))
        })?;

        json.get("result")
            .and_then(|v| v.as_u64())
            .ok_or(AppError::InternalServerError(
                "Failed to get latest Solana slot".to_string(),
            ))
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<TransactionReceipt, AppError> {
        // Solana doesn't have receipts like Ethereum, but we can simulate it
        let tx = self.get_transaction(tx_hash).await?;

        Ok(TransactionReceipt {
            transaction_hash: tx_hash.to_string(),
            block_number: tx.block_number.unwrap_or(0),
            block_hash: tx.block_hash.unwrap_or_default(),
            gas_used: tx.gas_used.unwrap_or(0),
            status: matches!(tx.status, TransactionStatus::Confirmed),
            logs: vec![], // Would need to parse from transaction meta
        })
    }

    pub async fn get_confirmations(&self, tx_hash: &str) -> Result<u32, AppError> {
        let tx = self.get_transaction(tx_hash).await?;
        Ok(tx.confirmations)
    }

    pub fn get_network_name(&self) -> &str {
        "solana"
    }
}
