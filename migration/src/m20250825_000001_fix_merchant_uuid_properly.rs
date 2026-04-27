use sea_orm::{Statement, Value};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Enable UUID extension if not already enabled
        manager
            .get_connection()
            .execute_unprepared("CREATE EXTENSION IF NOT EXISTS \"uuid-ossp\"")
            .await?;

        // Check if merchant.id is already UUID
        let check_result = manager
            .get_connection()
            .query_one(Statement::from_string(
                manager.get_database_backend(),
                "SELECT data_type FROM information_schema.columns WHERE table_name = 'merchant' AND column_name = 'id'".to_string()
            ))
            .await?;

        if let Some(row) = check_result {
            let data_type: String = row.try_get("", "data_type")?;
            if data_type.to_lowercase() != "uuid" {
                // Drop foreign key constraints temporarily
                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE api_key DROP CONSTRAINT IF EXISTS fk_api_key_merchant",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE api_key DROP CONSTRAINT IF EXISTS \"fk-apikey-merchant\"",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared("ALTER TABLE payment_request DROP CONSTRAINT IF EXISTS fk_payment_request_merchant")
                    .await.ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE settlement DROP CONSTRAINT IF EXISTS fk_settlement_merchant",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared("ALTER TABLE settlement_rule DROP CONSTRAINT IF EXISTS fk_settlement_rule_merchant")
                    .await.ok();

                manager
                    .get_connection()
                    .execute_unprepared("ALTER TABLE payout_transaction DROP CONSTRAINT IF EXISTS fk_payout_transaction_merchant")
                    .await.ok();

                manager
                    .get_connection()
                    .execute_unprepared("ALTER TABLE withdrawal_request DROP CONSTRAINT IF EXISTS fk_withdrawal_request_merchant")
                    .await.ok();

                // Convert merchant ID column to UUID
                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE merchant 
                         ALTER COLUMN id TYPE UUID USING CASE 
                           WHEN id ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' 
                           THEN id::UUID 
                           ELSE uuid_generate_v4() 
                         END"
                    )
                    .await?;

                // Recreate foreign key constraints
                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE api_key 
                         ADD CONSTRAINT fk_api_key_merchant 
                         FOREIGN KEY (merchant_id) REFERENCES merchant(id) ON DELETE CASCADE",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE payment_request 
                         ADD CONSTRAINT fk_payment_request_merchant 
                         FOREIGN KEY (merchant_id) REFERENCES merchant(id) ON DELETE CASCADE",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE settlement 
                         ADD CONSTRAINT fk_settlement_merchant 
                         FOREIGN KEY (merchant_id) REFERENCES merchant(id) ON DELETE CASCADE",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE settlement_rule 
                         ADD CONSTRAINT fk_settlement_rule_merchant 
                         FOREIGN KEY (merchant_id) REFERENCES merchant(id) ON DELETE CASCADE",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE payout_transaction 
                         ADD CONSTRAINT fk_payout_transaction_merchant 
                         FOREIGN KEY (merchant_id) REFERENCES merchant(id) ON DELETE CASCADE",
                    )
                    .await
                    .ok();

                manager
                    .get_connection()
                    .execute_unprepared(
                        "ALTER TABLE withdrawal_request 
                         ADD CONSTRAINT fk_withdrawal_request_merchant 
                         FOREIGN KEY (merchant_id) REFERENCES merchant(id) ON DELETE CASCADE",
                    )
                    .await
                    .ok();
            }
        }

        // Set UUID default for merchant ID
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE merchant ALTER COLUMN id SET DEFAULT uuid_generate_v4()",
            )
            .await?;

        // Add unique constraint on merchant name if it doesn't exist
        manager
            .create_index(
                Index::create()
                    .name("unique_merchant_name")
                    .table(Merchant::Table)
                    .col(Merchant::Name)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await
            .ok(); // Ignore if already exists

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove UUID default
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE merchant ALTER COLUMN id DROP DEFAULT")
            .await?;

        // Drop unique constraint
        manager
            .drop_index(Index::drop().name("unique_merchant_name").to_owned())
            .await
            .ok();

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Merchant {
    Table,
    Id,
    Name,
}
