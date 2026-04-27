use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add safe withdrawal fields to withdrawal_request table
        manager
            .alter_table(
                Table::alter()
                    .table(WithdrawalRequest::Table)
                    .add_column(
                        ColumnDef::new(WithdrawalRequest::FinalityRequired)
                            .integer()
                            .not_null()
                            .default(6),
                    )
                    .add_column(
                        ColumnDef::new(WithdrawalRequest::FinalityReachedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(WithdrawalRequest::ReplacedByTx)
                            .string()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(WithdrawalRequest::ReorgDepth)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .add_column(
                        ColumnDef::new(WithdrawalRequest::LifecycleState)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .add_column(
                        ColumnDef::new(WithdrawalRequest::WithdrawalFundsState)
                            .string()
                            .not_null()
                            .default("available"),
                    )
                    .to_owned(),
            )
            .await?;

        // Create index for finality tracking
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-finality")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::FinalityRequired)
                    .col(WithdrawalRequest::FinalityReachedAt)
                    .to_owned(),
            )
            .await?;

        // Create index for lifecycle state
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-lifecycle")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::LifecycleState)
                    .to_owned(),
            )
            .await?;

        // Create index for withdrawal funds state
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-funds-state")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::WithdrawalFundsState)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes first
        manager
            .drop_index(
                Index::drop()
                    .name("idx-withdrawal-request-finality")
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx-withdrawal-request-lifecycle")
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx-withdrawal-request-funds-state")
                    .to_owned(),
            )
            .await?;

        // Remove the columns
        manager
            .alter_table(
                Table::alter()
                    .table(WithdrawalRequest::Table)
                    .drop_column(WithdrawalRequest::FinalityRequired)
                    .drop_column(WithdrawalRequest::FinalityReachedAt)
                    .drop_column(WithdrawalRequest::ReplacedByTx)
                    .drop_column(WithdrawalRequest::ReorgDepth)
                    .drop_column(WithdrawalRequest::LifecycleState)
                    .drop_column(WithdrawalRequest::WithdrawalFundsState)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum WithdrawalRequest {
    Table,
    FinalityRequired,
    FinalityReachedAt,
    ReplacedByTx,
    ReorgDepth,
    LifecycleState,
    WithdrawalFundsState,
}
