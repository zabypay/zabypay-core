use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize)]
#[sea_orm(table_name = "wallet_audit_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub wallet_id: String,
    pub user_id: String,
    pub action: String, // "created", "balance_updated", "deposit_detected", etc.
    pub old_value: Option<String>, // JSON of old state
    pub new_value: Option<String>, // JSON of new state
    pub description: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::wallet::Entity",
        from = "Column::WalletId",
        to = "super::wallet::Column::Id"
    )]
    Wallet,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl Related<super::wallet::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Wallet.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, Serialize, Deserialize)]
pub enum AuditAction {
    WalletCreated,
    BalanceUpdated,
    DepositDetected,
    DepositConfirmed,
    WithdrawalRequested,
    WithdrawalProcessed,
    TransactionCreated,
    StatusChanged,
    ConfigurationUpdated,
    SecurityEvent,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditAction::WalletCreated => "wallet_created",
            AuditAction::BalanceUpdated => "balance_updated",
            AuditAction::DepositDetected => "deposit_detected",
            AuditAction::DepositConfirmed => "deposit_confirmed",
            AuditAction::WithdrawalRequested => "withdrawal_requested",
            AuditAction::WithdrawalProcessed => "withdrawal_processed",
            AuditAction::TransactionCreated => "transaction_created",
            AuditAction::StatusChanged => "status_changed",
            AuditAction::ConfigurationUpdated => "configuration_updated",
            AuditAction::SecurityEvent => "security_event",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "wallet_created" => Some(AuditAction::WalletCreated),
            "balance_updated" => Some(AuditAction::BalanceUpdated),
            "deposit_detected" => Some(AuditAction::DepositDetected),
            "deposit_confirmed" => Some(AuditAction::DepositConfirmed),
            "withdrawal_requested" => Some(AuditAction::WithdrawalRequested),
            "withdrawal_processed" => Some(AuditAction::WithdrawalProcessed),
            "transaction_created" => Some(AuditAction::TransactionCreated),
            "status_changed" => Some(AuditAction::StatusChanged),
            "configuration_updated" => Some(AuditAction::ConfigurationUpdated),
            "security_event" => Some(AuditAction::SecurityEvent),
            _ => None,
        }
    }
}

impl Model {
    pub fn new(
        wallet_id: String,
        user_id: String,
        action: AuditAction,
        description: Option<String>,
        ip_address: Option<String>,
        user_agent: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            wallet_id,
            user_id,
            action: action.as_str().to_string(),
            old_value: None,
            new_value: None,
            description,
            ip_address,
            user_agent,
            created_at: chrono::Utc::now(),
        }
    }

    pub fn with_state_change<T: Serialize>(
        mut self,
        old_state: Option<&T>,
        new_state: Option<&T>,
    ) -> Self {
        if let Some(old) = old_state {
            self.old_value = serde_json::to_string(old).ok();
        }
        if let Some(new) = new_state {
            self.new_value = serde_json::to_string(new).ok();
        }
        self
    }

    pub fn get_old_state<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        self.old_value
            .as_ref()
            .and_then(|v| serde_json::from_str(v).ok())
    }

    pub fn get_new_state<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        self.new_value
            .as_ref()
            .and_then(|v| serde_json::from_str(v).ok())
    }
} 