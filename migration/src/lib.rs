pub use sea_orm_migration::prelude::*;

mod m20250115_000000_create_admin_tables;
mod m20250115_000001_create_pending_user_table;
mod m20250608_141511_create_user_table;
mod m20250609_000000_add_merchant_tables;
mod m20250722_105535_create_wallet_table;
mod m20250723_000000_add_settlement_tables;
mod m20250730_000000_add_wallet_balance_tables;
mod m20250803_000000_add_encrypted_api_keys;
mod m20250805_000000_update_webhook_log;
mod m20250805_000001_add_merchant_network_preferences;
mod m20250805_000002_add_api_key_environment;
mod m20250805_000003_add_plain_api_keys;
mod m20250805_212315_add_payment_request_id_to_webhook_log;
mod m20250806_122254_add_environment_to_payment_request;
mod m20250821_000000_add_withdrawal_request_table;
mod m20250822_000000_add_auth_security_fields;
mod m20250825_000000_fix_merchant_table_uuid;
mod m20250825_000001_fix_merchant_uuid_properly;
mod m20250922_000000_add_safe_withdrawal_fields;
mod m20251005_000000_add_locked_balance_column;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250608_141511_create_user_table::Migration),
            Box::new(m20250609_000000_add_merchant_tables::Migration),
            Box::new(m20250722_105535_create_wallet_table::Migration),
            Box::new(m20250723_000000_add_settlement_tables::Migration),
            Box::new(m20250730_000000_add_wallet_balance_tables::Migration),
            Box::new(m20250115_000000_create_admin_tables::Migration),
            Box::new(m20250115_000001_create_pending_user_table::Migration),
            Box::new(m20250803_000000_add_encrypted_api_keys::Migration),
            Box::new(m20250805_000000_update_webhook_log::Migration),
            Box::new(m20250805_000001_add_merchant_network_preferences::Migration),
            Box::new(m20250805_000002_add_api_key_environment::Migration),
            Box::new(m20250805_000003_add_plain_api_keys::Migration),
            Box::new(m20250805_212315_add_payment_request_id_to_webhook_log::Migration),
            Box::new(m20250806_122254_add_environment_to_payment_request::Migration),
            Box::new(m20250821_000000_add_withdrawal_request_table::Migration),
            Box::new(m20250822_000000_add_auth_security_fields::Migration),
            Box::new(m20250922_000000_add_safe_withdrawal_fields::Migration),
            Box::new(m20251005_000000_add_locked_balance_column::Migration),
            // Box::new(m20250825_000000_fix_merchant_table_uuid::Migration),
            // Box::new(m20250825_000001_fix_merchant_uuid_properly::Migration),
        ]
    }
}
