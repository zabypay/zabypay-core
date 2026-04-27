use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add payment_request_id column to webhook_log table
        manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(WebhookLog::PaymentRequestId).string().null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove payment_request_id column
        manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .drop_column(WebhookLog::PaymentRequestId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
enum WebhookLog {
    Table,
    PaymentRequestId,
}
