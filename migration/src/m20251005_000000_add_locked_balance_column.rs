use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add locked_balance column to wallet_balance table if it doesn't exist
        // This makes the database schema match the entity model
        manager
            .alter_table(
                Table::alter()
                    .table(WalletBalance::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(WalletBalance::LockedBalance)
                            .decimal()
                            .not_null()
                            .default("0.00"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the locked_balance column
        manager
            .alter_table(
                Table::alter()
                    .table(WalletBalance::Table)
                    .drop_column(WalletBalance::LockedBalance)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum WalletBalance {
    Table,
    LockedBalance,
}
