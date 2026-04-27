use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add environment column to payment_request table
        manager
            .alter_table(
                Table::alter()
                    .table(PaymentRequest::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(PaymentRequest::Environment)
                            .string()
                            .default("mainnet")
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove environment column
        manager
            .alter_table(
                Table::alter()
                    .table(PaymentRequest::Table)
                    .drop_column(PaymentRequest::Environment)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum PaymentRequest {
    Table,
    Environment,
}
