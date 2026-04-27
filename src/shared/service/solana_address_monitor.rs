use crate::shared::utils::errors::AppError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;

/// Monitors Solana addresses for incoming transactions
pub struct SolanaAddressMonitor {
    rpc_url: String,
    client: reqwest::Client,
}

impl SolanaAddressMonitor {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: reqwest::Client::new(),
        }
    }

    /// Get recent transactions for a Solana address
    pub async fn get_recent_transactions(
        &self,
        address: &str,
        limit: usize,
    ) -> Result<Vec<SolanaTransaction>, AppError> {
        // Get signatures for the address
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignaturesForAddress",
            "params": [
                address,
                {
                    "limit": limit
                }
            ]
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Solana RPC failed: {}", e)))?;

        let json: Value = response
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Parse error: {}", e)))?;

        let signatures = json["result"]
            .as_array()
            .ok_or_else(|| AppError::InternalServerError("Invalid response".to_string()))?;

        let mut transactions = Vec::new();

        // Get details for each transaction
        for sig_info in signatures {
            if let Some(signature) = sig_info["signature"].as_str() {
                if let Ok(tx) = self.get_transaction_details(signature).await {
                    transactions.push(tx);
                }
            }
        }

        Ok(transactions)
    }

    /// Get transaction details including amount
    async fn get_transaction_details(
        &self,
        signature: &str,
    ) -> Result<SolanaTransaction, AppError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [
                signature,
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
            .await?;

        let json: Value = response.json().await?;
        let result = &json["result"];

        if result.is_null() {
            return Err(AppError::NotFound("Transaction not found".to_string()));
        }

        // Parse transaction to extract transfer amount
        let meta = &result["meta"];
        let empty_vec = vec![];
        let pre_balances = meta["preBalances"].as_array().unwrap_or(&empty_vec);
        let post_balances = meta["postBalances"].as_array().unwrap_or(&empty_vec);

        // Calculate amount from balance changes
        let mut amount_lamports = 0i64;
        if pre_balances.len() > 1 && post_balances.len() > 1 {
            let pre = pre_balances[1].as_i64().unwrap_or(0);
            let post = post_balances[1].as_i64().unwrap_or(0);
            amount_lamports = post - pre;
        }

        // Convert lamports to SOL
        let amount_sol = amount_lamports.abs() as f64 / 1_000_000_000.0;

        // Get block time
        let block_time = result["blockTime"]
            .as_i64()
            .map(|t| DateTime::from_timestamp(t, 0))
            .flatten()
            .unwrap_or_else(Utc::now);

        Ok(SolanaTransaction {
            signature: signature.to_string(),
            amount_sol,
            amount_lamports: amount_lamports.abs() as u64,
            block_time,
            status: if meta["err"].is_null() {
                "confirmed"
            } else {
                "failed"
            }
            .to_string(),
            fee: meta["fee"].as_u64().unwrap_or(0),
        })
    }

    /// Check if address received expected amount
    pub async fn check_payment_received(
        &self,
        address: &str,
        expected_amount_sol: f64,
        since_time: DateTime<Utc>,
    ) -> Result<Option<SolanaTransaction>, AppError> {
        let transactions = self.get_recent_transactions(address, 20).await?;

        for tx in transactions {
            // Check if transaction is after payment creation time
            if tx.block_time < since_time {
                continue;
            }

            // Check if amount matches (with 0.1% tolerance for fees)
            let tolerance = expected_amount_sol * 0.001;
            if (tx.amount_sol - expected_amount_sol).abs() <= tolerance {
                if tx.status == "confirmed" {
                    return Ok(Some(tx));
                }
            }
        }

        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct SolanaTransaction {
    pub signature: String,
    pub amount_sol: f64,
    pub amount_lamports: u64,
    pub block_time: DateTime<Utc>,
    pub status: String,
    pub fee: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lamports_to_sol() {
        let lamports = 1_000_000_000u64;
        let sol = lamports as f64 / 1_000_000_000.0;
        assert_eq!(sol, 1.0);
    }
}
