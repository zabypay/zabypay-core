use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use crate::shared::utils::errors::AppError;
use crate::merchant::models::merchant::{SettlementFrequency};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementRequest {
    pub merchant_id: String,
    pub payment_request_id: String,
    pub amount: f64,
    pub currency: String,
    pub transaction_hash: String,
    pub from_address: String,
    pub to_address: String,
    pub network: String,
    pub block_number: Option<u64>,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settlement {
    pub settlement_id: String,
    pub merchant_id: String,
    pub payment_request_id: String,
    pub original_amount: f64,
    pub original_currency: String,
    pub settled_amount: f64,
    pub settled_currency: String,
    pub conversion_rate: Option<f64>,
    pub fee_amount: f64,
    pub fee_currency: String,
    pub status: SettlementStatus,
    pub payout_address: String,
    pub payout_network: String,
    pub payout_tx_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SettlementStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutTransaction {
    pub payout_id: String,
    pub merchant_id: String,
    pub amount: f64,
    pub currency: String,
    pub to_address: String,
    pub network: String,
    pub status: PayoutStatus,
    pub tx_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PayoutStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementRule {
    pub merchant_id: String,
    pub auto_convert: bool,
    pub target_currency: String,
    pub min_settlement_amount: f64,
    pub settlement_frequency: SettlementFrequency,
    pub payout_addresses: HashMap<String, String>,
    pub enabled: bool,
}

pub struct SettlementEngine {
    settlement_rules: HashMap<String, SettlementRule>,
    pending_settlements: HashMap<String, Settlement>,
    pending_payouts: HashMap<String, PayoutTransaction>,
    conversion_service: Box<dyn ConversionService>,
    payout_service: Box<dyn PayoutService>,
    notification_service: Box<dyn NotificationService>,
}

#[async_trait::async_trait]
pub trait ConversionService: Send + Sync {
    async fn convert_currency(&self, from_currency: &str, to_currency: &str, amount: f64) -> Result<ConversionResult, AppError>;
    async fn get_conversion_rate(&self, from_currency: &str, to_currency: &str) -> Result<f64, AppError>;
}

#[async_trait::async_trait]
pub trait PayoutService: Send + Sync {
    async fn create_payout(&self, payout: &PayoutTransaction) -> Result<String, AppError>;
    async fn get_payout_status(&self, payout_id: &str) -> Result<PayoutStatus, AppError>;
    async fn get_payout_tx_hash(&self, payout_id: &str) -> Result<Option<String>, AppError>;
}

#[async_trait::async_trait]
pub trait NotificationService: Send + Sync {
    async fn send_settlement_notification(&self, settlement: &Settlement) -> Result<(), AppError>;
    async fn send_payout_notification(&self, payout: &PayoutTransaction) -> Result<(), AppError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionResult {
    pub from_currency: String,
    pub to_currency: String,
    pub from_amount: f64,
    pub to_amount: f64,
    pub rate: f64,
    pub fee_amount: f64,
    pub fee_currency: String,
}

impl SettlementEngine {
    pub fn new(
        conversion_service: Box<dyn ConversionService>,
        payout_service: Box<dyn PayoutService>,
        notification_service: Box<dyn NotificationService>,
    ) -> Self {
        Self {
            settlement_rules: HashMap::new(),
            pending_settlements: HashMap::new(),
            pending_payouts: HashMap::new(),
            conversion_service,
            payout_service,
            notification_service,
        }
    }

    pub async fn process_settlement(&mut self, request: &SettlementRequest) -> Result<Settlement, AppError> {
        // Get merchant settlement rules
        let settlement_rule = self.get_settlement_rule(&request.merchant_id).await?;
        
        // Create settlement record
        let settlement_id = uuid::Uuid::new_v4().to_string();
        let mut settlement = Settlement {
            settlement_id: settlement_id.clone(),
            merchant_id: request.merchant_id.clone(),
            payment_request_id: request.payment_request_id.clone(),
            original_amount: request.amount,
            original_currency: request.currency.clone(),
            settled_amount: request.amount,
            settled_currency: request.currency.clone(),
            conversion_rate: None,
            fee_amount: 0.0,
            fee_currency: request.currency.clone(),
            status: SettlementStatus::Pending,
            payout_address: "".to_string(),
            payout_network: request.network.clone(),
            payout_tx_hash: None,
            created_at: Utc::now(),
            settled_at: None,
        };

        // Apply conversion if needed
        if settlement_rule.auto_convert && request.currency != settlement_rule.target_currency {
            let conversion_result = self.conversion_service.convert_currency(
                &request.currency,
                &settlement_rule.target_currency,
                request.amount,
            ).await?;

            settlement.settled_amount = conversion_result.to_amount;
            settlement.settled_currency = settlement_rule.target_currency.clone();
            settlement.conversion_rate = Some(conversion_result.rate);
            settlement.fee_amount = conversion_result.fee_amount;
            settlement.fee_currency = conversion_result.fee_currency;
        }

        // Get payout address for the settled currency
        if let Some(payout_address) = settlement_rule.payout_addresses.get(&settlement.settled_currency) {
            settlement.payout_address = payout_address.clone();
        } else {
            return Err(AppError::ValidationError(
                format!("No payout address configured for currency: {}", settlement.settled_currency)
            ));
        }

        // Store settlement
        self.pending_settlements.insert(settlement_id.clone(), settlement.clone());

        // Process settlement based on frequency
        match settlement_rule.settlement_frequency {
            SettlementFrequency::Immediate => {
                self.process_immediate_settlement(&settlement).await?;
            }
            SettlementFrequency::Daily => {
                // Add to daily batch
                self.add_to_daily_batch(&settlement).await?;
            }
            SettlementFrequency::Weekly => {
                // Add to weekly batch
                self.add_to_weekly_batch(&settlement).await?;
            }
            SettlementFrequency::Monthly => {
                // Add to monthly batch
                self.add_to_monthly_batch(&settlement).await?;
            }
        }

        Ok(settlement)
    }

    async fn process_immediate_settlement(&mut self, settlement: &Settlement) -> Result<(), AppError> {
        // Create payout transaction
        let payout = PayoutTransaction {
            payout_id: uuid::Uuid::new_v4().to_string(),
            merchant_id: settlement.merchant_id.clone(),
            amount: settlement.settled_amount,
            currency: settlement.settled_currency.clone(),
            to_address: settlement.payout_address.clone(),
            network: settlement.payout_network.clone(),
            status: PayoutStatus::Pending,
            tx_hash: None,
            created_at: Utc::now(),
            completed_at: None,
            error_message: None,
        };

        // Execute payout
        let payout_tx_hash = self.payout_service.create_payout(&payout).await?;
        
        // Update settlement
        let mut updated_settlement = settlement.clone();
        updated_settlement.status = SettlementStatus::Processing;
        updated_settlement.payout_tx_hash = Some(payout_tx_hash);
        
        // Store payout
        self.pending_payouts.insert(payout.payout_id.clone(), payout.clone());

        // Send notifications
        self.notification_service.send_settlement_notification(&updated_settlement).await?;
        self.notification_service.send_payout_notification(&payout).await?;

        Ok(())
    }

    async fn add_to_daily_batch(&self, settlement: &Settlement) -> Result<(), AppError> {
        // In a real implementation, this would add to a daily batch queue
        // For now, just log it
        log::info!("Added settlement {} to daily batch for merchant {}", 
            settlement.settlement_id, settlement.merchant_id);
        Ok(())
    }

    async fn add_to_weekly_batch(&self, settlement: &Settlement) -> Result<(), AppError> {
        // In a real implementation, this would add to a weekly batch queue
        log::info!("Added settlement {} to weekly batch for merchant {}", 
            settlement.settlement_id, settlement.merchant_id);
        Ok(())
    }

    async fn add_to_monthly_batch(&self, settlement: &Settlement) -> Result<(), AppError> {
        // In a real implementation, this would add to a monthly batch queue
        log::info!("Added settlement {} to monthly batch for merchant {}", 
            settlement.settlement_id, settlement.merchant_id);
        Ok(())
    }

    async fn get_settlement_rule(&self, merchant_id: &str) -> Result<SettlementRule, AppError> {
        // In a real implementation, this would fetch from database
        // For now, return a default rule
        Ok(SettlementRule {
            merchant_id: merchant_id.to_string(),
            auto_convert: true,
            target_currency: "USDT".to_string(),
            min_settlement_amount: 10.0,
            settlement_frequency: SettlementFrequency::Immediate,
            payout_addresses: {
                let mut addresses = HashMap::new();
                addresses.insert("USDT".to_string(), "0x1234567890123456789012345678901234567890".to_string());
                addresses.insert("ETH".to_string(), "0x1234567890123456789012345678901234567890".to_string());
                addresses.insert("BTC".to_string(), "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_string());
                addresses
            },
            enabled: true,
        })
    }

    pub async fn process_daily_settlements(&self) -> Result<u64, AppError> {
        // Process all daily settlements
        let processed_count = 0;
        
        // In a real implementation, this would process all settlements in the daily batch
        // For now, just return 0
        Ok(processed_count)
    }

    pub async fn process_weekly_settlements(&self) -> Result<u64, AppError> {
        // Process all weekly settlements
        let processed_count = 0;
        
        // In a real implementation, this would process all settlements in the weekly batch
        Ok(processed_count)
    }

    pub async fn process_monthly_settlements(&self) -> Result<u64, AppError> {
        // Process all monthly settlements
        let processed_count = 0;
        
        // In a real implementation, this would process all settlements in the monthly batch
        Ok(processed_count)
    }

    pub async fn get_settlement_status(&self, settlement_id: &str) -> Result<SettlementStatus, AppError> {
        if let Some(settlement) = self.pending_settlements.get(settlement_id) {
            return Ok(settlement.status.clone());
        }
        
        Err(AppError::NotFound("Settlement not found".to_string()))
    }

    pub async fn get_payout_status(&self, payout_id: &str) -> Result<PayoutStatus, AppError> {
        if let Some(payout) = self.pending_payouts.get(payout_id) {
            return Ok(payout.status.clone());
        }
        
        Err(AppError::NotFound("Payout not found".to_string()))
    }
}

// Default Conversion Service Implementation
pub struct DefaultConversionService {
    exchange_rate_service: crate::shared::service::exchange_rate::ExchangeRateService,
}

impl DefaultConversionService {
    pub fn new() -> Self {
        let mut exchange_service = crate::shared::service::exchange_rate::ExchangeRateService::new();
        exchange_service.add_provider(Box::new(crate::shared::service::exchange_rate::KrakenProvider::new()));
        exchange_service.add_provider(Box::new(crate::shared::service::exchange_rate::BinanceProvider::new()));
        
        Self {
            exchange_rate_service: exchange_service,
        }
    }
}

#[async_trait::async_trait]
impl ConversionService for DefaultConversionService {
    async fn convert_currency(&self, from_currency: &str, to_currency: &str, amount: f64) -> Result<ConversionResult, AppError> {
        let rate = self.get_conversion_rate(from_currency, to_currency).await?;
        let converted_amount = amount * rate;
        let fee_amount = converted_amount * 0.001; // 0.1% fee
        let final_amount = converted_amount - fee_amount;

        Ok(ConversionResult {
            from_currency: from_currency.to_string(),
            to_currency: to_currency.to_string(),
            from_amount: amount,
            to_amount: final_amount,
            rate,
            fee_amount,
            fee_currency: to_currency.to_string(),
        })
    }

    async fn get_conversion_rate(&self, from_currency: &str, to_currency: &str) -> Result<f64, AppError> {
        let rate_info = self.exchange_rate_service.get_best_rate(from_currency, to_currency).await?;
        Ok(rate_info.rate)
    }
}

// Default Payout Service Implementation
pub struct DefaultPayoutService {
    client: reqwest::Client,
}

impl DefaultPayoutService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl PayoutService for DefaultPayoutService {
    async fn create_payout(&self, payout: &PayoutTransaction) -> Result<String, AppError> {
        // In a real implementation, this would create an actual blockchain transaction
        // For now, return a mock transaction hash
        let tx_hash = format!("0x{}", uuid::Uuid::new_v4().to_string().replace("-", ""));
        
        log::info!("Created payout transaction {} for {} {} to {}", 
            tx_hash, payout.amount, payout.currency, payout.to_address);
        
        Ok(tx_hash)
    }

    async fn get_payout_status(&self, _payout_id: &str) -> Result<PayoutStatus, AppError> {
        // In a real implementation, this would check the actual blockchain status
        // For now, return completed
        Ok(PayoutStatus::Completed)
    }

    async fn get_payout_tx_hash(&self, _payout_id: &str) -> Result<Option<String>, AppError> {
        // In a real implementation, this would return the actual transaction hash
        // For now, return a mock hash
        Ok(Some(format!("0x{}", uuid::Uuid::new_v4().to_string().replace("-", ""))))
    }
}

// Default Notification Service Implementation
pub struct DefaultNotificationService {
    email_service: crate::client::services::email_service::EmailService,
}

impl DefaultNotificationService {
    pub fn new() -> Self {
        Self {
            email_service: crate::client::services::email_service::EmailService,
        }
    }
}

#[async_trait::async_trait]
impl NotificationService for DefaultNotificationService {
    async fn send_settlement_notification(&self, settlement: &Settlement) -> Result<(), AppError> {
        log::info!("Settlement notification sent for settlement {}: {} {} -> {} {}", 
            settlement.settlement_id,
            settlement.original_amount,
            settlement.original_currency,
            settlement.settled_amount,
            settlement.settled_currency);
        
        // In a real implementation, this would send an email or webhook notification
        Ok(())
    }

    async fn send_payout_notification(&self, payout: &PayoutTransaction) -> Result<(), AppError> {
        log::info!("Payout notification sent for payout {}: {} {} to {}", 
            payout.payout_id,
            payout.amount,
            payout.currency,
            payout.to_address);
        
        // In a real implementation, this would send an email or webhook notification
        Ok(())
    }
} 