//! Chain Configuration Entity

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "chain_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub network: String,
    pub environment: String,
    pub required_confirmations: i32,
    pub pending_ttl_seconds: i32,
    pub reorg_grace_confirmations: i32,
    pub poll_interval_ms: i32,
    pub rpc_url: Option<String>,
    pub explorer_url_template: Option<String>,
    pub enabled: bool,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Get explorer URL for a specific transaction
    pub fn get_explorer_url(&self, tx_hash: &str) -> Option<String> {
        self.explorer_url_template
            .as_ref()
            .map(|template| template.replace("{tx_hash}", tx_hash))
    }

    /// Check if this configuration supports the given network and environment
    pub fn matches(&self, network: &str, environment: &str) -> bool {
        self.network == network && self.environment == environment && self.enabled
    }

    /// Get the timeout duration for pending transactions
    pub fn pending_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.pending_ttl_seconds as u64)
    }

    /// Get the polling interval for monitoring
    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.poll_interval_ms as u64)
    }
}
