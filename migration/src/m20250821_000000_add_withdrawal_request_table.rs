use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create withdrawal_request table
        manager
            .create_table(
                Table::create()
                    .table(WithdrawalRequest::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(WithdrawalRequest::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::MerchantId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::WalletId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::ExternalId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::IdempotencyKey)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::Environment)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::Network)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::Currency)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::ToAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::Amount)
                            .decimal()
                            .not_null(),
                    )
                    .col(ColumnDef::new(WithdrawalRequest::Fee).decimal().null())
                    .col(
                        ColumnDef::new(WithdrawalRequest::NetAmount)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(ColumnDef::new(WithdrawalRequest::TxHash).string().null())
                    .col(
                        ColumnDef::new(WithdrawalRequest::BlockchainConfirmations)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::RequiredConfirmations)
                            .integer()
                            .not_null()
                            .default(6),
                    )
                    .col(ColumnDef::new(WithdrawalRequest::Metadata).string().null())
                    .col(
                        ColumnDef::new(WithdrawalRequest::ErrorMessage)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::ProcessedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::ConfirmedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::BroadcastAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::FailedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WithdrawalRequest::FailureReason)
                            .string()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-withdrawal-request-merchant")
                            .from(WithdrawalRequest::Table, WithdrawalRequest::MerchantId)
                            .to(Merchant::Table, Merchant::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-withdrawal-request-wallet")
                            .from(WithdrawalRequest::Table, WithdrawalRequest::WalletId)
                            .to(Wallet::Table, Wallet::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique constraint on merchant_id + idempotency_key for duplicate prevention
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-idempotency")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::MerchantId)
                    .col(WithdrawalRequest::IdempotencyKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create index for status queries
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-status")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::Status)
                    .to_owned(),
            )
            .await?;

        // Create index for environment + network queries
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-env-network")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::Environment)
                    .col(WithdrawalRequest::Network)
                    .to_owned(),
            )
            .await?;

        // Create index for created_at ordering
        manager
            .create_index(
                Index::create()
                    .name("idx-withdrawal-request-created-at")
                    .table(WithdrawalRequest::Table)
                    .col(WithdrawalRequest::CreatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the table
        manager
            .drop_table(Table::drop().table(WithdrawalRequest::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum WithdrawalRequest {
    Table,
    Id,
    MerchantId,
    WalletId,
    ExternalId,
    IdempotencyKey,
    Environment,
    Network,
    Currency,
    ToAddress,
    Amount,
    Fee,
    NetAmount,
    Status,
    TxHash,
    BlockchainConfirmations,
    RequiredConfirmations,
    Metadata,
    ErrorMessage,
    CreatedAt,
    ProcessedAt,
    ConfirmedAt,
    UpdatedAt,
    BroadcastAt,
    FailedAt,
    FailureReason,
}

#[derive(DeriveIden)]
enum Merchant {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Wallet {
    Table,
    Id,
}
