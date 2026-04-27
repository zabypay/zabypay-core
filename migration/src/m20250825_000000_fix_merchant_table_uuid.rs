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

        // Try to alter the merchant ID column to UUID with default
        // This will work for fresh databases or ones with UUID-compatible strings
        let result = manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE merchant 
                 ALTER COLUMN id TYPE UUID USING CASE 
                   WHEN id ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' 
                   THEN id::UUID 
                   ELSE uuid_generate_v4() 
                 END,
                 ALTER COLUMN id SET DEFAULT uuid_generate_v4()",
            )
            .await;

        // If the ALTER fails, it might be because the column is already UUID
        if let Err(e) = result {
            println!(
                "Warning: Could not alter merchant.id column: {}. Might already be UUID.",
                e
            );

            // Try to just set the default
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE merchant ALTER COLUMN id SET DEFAULT uuid_generate_v4()",
                )
                .await
                .ok();
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Convert back to string if needed (destructive)
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE merchant 
                 ALTER COLUMN id TYPE VARCHAR,
                 ALTER COLUMN id DROP DEFAULT",
            )
            .await?;

        // Drop unique constraint
        manager
            .drop_index(Index::drop().name("unique_merchant_name").to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Merchant {
    Table,
    Id,
    Name,
}
