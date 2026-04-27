use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use crate::shared::utils::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceCheck {
    pub transaction_hash: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: f64,
    pub currency: String,
    pub risk_score: f64,
    pub risk_factors: Vec<RiskFactor>,
    pub compliance_status: ComplianceStatus,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComplianceStatus {
    Approved,
    Pending,
    Rejected,
    RequiresReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub factor_type: RiskFactorType,
    pub severity: RiskSeverity,
    pub description: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskFactorType {
    SanctionedAddress,
    DarknetMarket,
    MixerService,
    HighRiskExchange,
    SuspiciousPattern,
    LargeAmount,
    NewAddress,
    MultipleTransactions,
    GeographicRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainAnalysisResult {
    pub address: String,
    pub risk_score: f64,
    pub category: String,
    pub tags: Vec<String>,
    pub volume_24h: Option<f64>,
    pub transaction_count: Option<u64>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

pub struct ComplianceService {
    chain_analysis_client: ChainAnalysisClient,
    risk_scorer: RiskScorer,
    sanctioned_addresses: HashMap<String, bool>,
    risk_patterns: Vec<RiskPattern>,
}

impl ComplianceService {
    pub fn new(api_key: String) -> Self {
        Self {
            chain_analysis_client: ChainAnalysisClient::new(api_key),
            risk_scorer: RiskScorer::new(),
            sanctioned_addresses: HashMap::new(),
            risk_patterns: Self::load_risk_patterns(),
        }
    }

    pub async fn check_transaction(&self, transaction: &Transaction) -> Result<ComplianceCheck, AppError> {
        let mut risk_factors = Vec::new();
        let mut risk_score = 0.0;

        // Check for sanctioned addresses
        if self.is_sanctioned_address(&transaction.from_address).await? {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::SanctionedAddress,
                severity: RiskSeverity::Critical,
                description: "Source address is on sanctions list".to_string(),
                details: None,
            });
            risk_score += 100.0;
        }

        // Check for darknet market associations
        if self.is_darknet_market_address(&transaction.from_address).await? {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::DarknetMarket,
                severity: RiskSeverity::High,
                description: "Source address associated with darknet markets".to_string(),
                details: None,
            });
            risk_score += 80.0;
        }

        // Check for mixer service usage
        if self.is_mixer_service(&transaction.from_address).await? {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::MixerService,
                severity: RiskSeverity::High,
                description: "Source address associated with mixing services".to_string(),
                details: None,
            });
            risk_score += 70.0;
        }

        // Check for large amounts
        if self.is_large_amount(transaction.amount, &transaction.currency) {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::LargeAmount,
                severity: RiskSeverity::Medium,
                description: format!("Large transaction amount: {} {}", transaction.amount, transaction.currency),
                details: None,
            });
            risk_score += 30.0;
        }

        // Check for suspicious patterns
        let pattern_risks = self.check_suspicious_patterns(transaction).await?;
        risk_factors.extend(pattern_risks.clone());
        risk_score += pattern_risks.iter().map(|rf| match rf.severity {
            RiskSeverity::Low => 10.0,
            RiskSeverity::Medium => 25.0,
            RiskSeverity::High => 50.0,
            RiskSeverity::Critical => 100.0,
        }).sum::<f64>();

        // Get chain analysis data
        let chain_analysis = self.chain_analysis_client.analyze_address(&transaction.from_address).await?;
        if chain_analysis.risk_score > 50.0 {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::SuspiciousPattern,
                severity: RiskSeverity::Medium,
                description: format!("High risk address (score: {})", chain_analysis.risk_score),
                details: Some(serde_json::json!({
                    "chain_analysis_score": chain_analysis.risk_score,
                    "category": chain_analysis.category,
                    "tags": chain_analysis.tags,
                })),
            });
            risk_score += chain_analysis.risk_score * 0.5;
        }

        // Determine compliance status
        let compliance_status = if risk_score >= 100.0 {
            ComplianceStatus::Rejected
        } else if risk_score >= 50.0 {
            ComplianceStatus::RequiresReview
        } else if risk_score >= 20.0 {
            ComplianceStatus::Pending
        } else {
            ComplianceStatus::Approved
        };

        Ok(ComplianceCheck {
            transaction_hash: transaction.hash.clone(),
            from_address: transaction.from_address.clone(),
            to_address: transaction.to_address.clone(),
            amount: transaction.amount,
            currency: transaction.currency.clone(),
            risk_score,
            risk_factors,
            compliance_status,
            checked_at: Utc::now(),
        })
    }

    async fn is_sanctioned_address(&self, address: &str) -> Result<bool, AppError> {
        // Check local cache first
        if let Some(&is_sanctioned) = self.sanctioned_addresses.get(address) {
            return Ok(is_sanctioned);
        }

        // Check with external API
        let is_sanctioned = self.chain_analysis_client.is_sanctioned(address).await?;
        
        // Cache the result (in production, use Redis or similar)
        // self.sanctioned_addresses.insert(address.to_string(), is_sanctioned);
        
        Ok(is_sanctioned)
    }

    async fn is_darknet_market_address(&self, _address: &str) -> Result<bool, AppError> {
        // This would integrate with external APIs to check for darknet market associations
        // For now, return false as placeholder
        Ok(false)
    }

    async fn is_mixer_service(&self, _address: &str) -> Result<bool, AppError> {
        // This would integrate with external APIs to check for mixer service usage
        // For now, return false as placeholder
        Ok(false)
    }

    fn is_large_amount(&self, amount: f64, currency: &str) -> bool {
        match currency {
            "BTC" => amount > 10.0,
            "ETH" => amount > 100.0,
            "USDT" => amount > 10000.0,
            _ => amount > 1000.0,
        }
    }

    async fn check_suspicious_patterns(&self, transaction: &Transaction) -> Result<Vec<RiskFactor>, AppError> {
        let mut risk_factors = Vec::new();

        // Check for new addresses (first transaction)
        if self.is_new_address(&transaction.from_address).await? {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::NewAddress,
                severity: RiskSeverity::Low,
                description: "New address with limited transaction history".to_string(),
                details: None,
            });
        }

        // Check for multiple rapid transactions
        if self.has_multiple_recent_transactions(&transaction.from_address).await? {
            risk_factors.push(RiskFactor {
                factor_type: RiskFactorType::MultipleTransactions,
                severity: RiskSeverity::Medium,
                description: "Multiple transactions in short time period".to_string(),
                details: None,
            });
        }

        Ok(risk_factors)
    }

    async fn is_new_address(&self, _address: &str) -> Result<bool, AppError> {
        // This would check transaction history for the address
        // For now, return false as placeholder
        Ok(false)
    }

    async fn has_multiple_recent_transactions(&self, _address: &str) -> Result<bool, AppError> {
        // This would check for multiple transactions in the last hour
        // For now, return false as placeholder
        Ok(false)
    }

    fn load_risk_patterns() -> Vec<RiskPattern> {
        vec![
            RiskPattern {
                name: "Large amount transfer".to_string(),
                conditions: vec![
                    "amount > 10000".to_string(),
                    "currency == 'USDT'".to_string(),
                ],
                risk_score: 30.0,
            },
            RiskPattern {
                name: "New address large transfer".to_string(),
                conditions: vec![
                    "is_new_address == true".to_string(),
                    "amount > 1000".to_string(),
                ],
                risk_score: 50.0,
            },
        ]
    }
}

#[derive(Debug, Clone)]
pub struct RiskPattern {
    pub name: String,
    pub conditions: Vec<String>,
    pub risk_score: f64,
}

pub struct ChainAnalysisClient {
    api_key: String,
    client: reqwest::Client,
    base_url: String,
}

impl ChainAnalysisClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
            base_url: "https://api.chainalysis.com".to_string(),
        }
    }

    pub async fn analyze_address(&self, address: &str) -> Result<ChainAnalysisResult, AppError> {
        let url = format!("{}/api/v1/addresses/{}", self.base_url, address);
        
        let response = self.client.get(&url)
            .header("Token", &self.api_key)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Chainalysis API error: {}", e)))?;

        if response.status().is_success() {
            let data: serde_json::Value = response.json().await
                .map_err(|e| AppError::InternalServerError(format!("Failed to parse Chainalysis response: {}", e)))?;

            let risk_score = data.get("risk")
                .and_then(|r| r.as_f64())
                .unwrap_or(0.0);

            let category = data.get("category")
                .and_then(|c| c.as_str())
                .unwrap_or("unknown")
                .to_string();

            let tags = data.get("tags")
                .and_then(|t| t.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
                .unwrap_or_default();

            return Ok(ChainAnalysisResult {
                address: address.to_string(),
                risk_score,
                category,
                tags,
                volume_24h: None,
                transaction_count: None,
                first_seen: None,
                last_seen: None,
            });
        }

        // If API call fails, return default result
        Ok(ChainAnalysisResult {
            address: address.to_string(),
            risk_score: 0.0,
            category: "unknown".to_string(),
            tags: Vec::new(),
            volume_24h: None,
            transaction_count: None,
            first_seen: None,
            last_seen: None,
        })
    }

    pub async fn is_sanctioned(&self, address: &str) -> Result<bool, AppError> {
        let result = self.analyze_address(address).await?;
        Ok(result.category == "sanctioned" || result.risk_score > 80.0)
    }
}

pub struct RiskScorer {
    weights: HashMap<String, f64>,
}

impl RiskScorer {
    pub fn new() -> Self {
        let mut weights = HashMap::new();
        weights.insert("sanctioned_address".to_string(), 100.0);
        weights.insert("darknet_market".to_string(), 80.0);
        weights.insert("mixer_service".to_string(), 70.0);
        weights.insert("large_amount".to_string(), 30.0);
        weights.insert("new_address".to_string(), 20.0);
        weights.insert("multiple_transactions".to_string(), 25.0);

        Self { weights }
    }

    pub fn calculate_risk_score(&self, factors: &[RiskFactor]) -> f64 {
        factors.iter()
            .map(|factor| {
                let base_score = match factor.severity {
                    RiskSeverity::Low => 10.0,
                    RiskSeverity::Medium => 25.0,
                    RiskSeverity::High => 50.0,
                    RiskSeverity::Critical => 100.0,
                };
                base_score
            })
            .sum()
    }
}

// Transaction struct for compliance checks
#[derive(Debug, Clone)]
pub struct Transaction {
    pub hash: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: f64,
    pub currency: String,
    pub timestamp: DateTime<Utc>,
} 