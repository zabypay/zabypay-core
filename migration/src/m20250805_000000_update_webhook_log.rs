use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add new columns to webhook_log table
        manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(WebhookLog::MerchantId)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .add_column_if_not_exists(
                        ColumnDef::new(WebhookLog::EventType)
                            .string()
                            .not_null()
                            .default("payment.confirmed"),
                    )
                    .add_column_if_not_exists(
                        ColumnDef::new(WebhookLog::LastAttemptAt).timestamp_with_time_zone(),
                    )
                    .to_owned(),
            )
            .await?;

        // Rename DeliveryStatus to Status and AttemptCount to Attempts
        // We need to do this in separate steps to handle potential errors
        let _ = manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .rename_column(WebhookLog::DeliveryStatus, WebhookLog::Status)
                    .to_owned(),
            )
            .await;

        let _ = manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .rename_column(WebhookLog::AttemptCount, WebhookLog::Attempts)
                    .to_owned(),
            )
            .await;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Revert changes
        manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .drop_column(WebhookLog::MerchantId)
                    .drop_column(WebhookLog::EventType)
                    .drop_column(WebhookLog::LastAttemptAt)
                    .to_owned(),
            )
            .await?;

        // Rename back
        let _ = manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .rename_column(WebhookLog::Status, WebhookLog::DeliveryStatus)
                    .to_owned(),
            )
            .await;

        let _ = manager
            .alter_table(
                Table::alter()
                    .table(WebhookLog::Table)
                    .rename_column(WebhookLog::Attempts, WebhookLog::AttemptCount)
                    .to_owned(),
            )
            .await;

        Ok(())
    }
}

#[derive(Iden)]
enum WebhookLog {
    Table,
    MerchantId,
    EventType,
    Status,
    Attempts,
    LastAttemptAt,
    DeliveryStatus,
    AttemptCount,
}
