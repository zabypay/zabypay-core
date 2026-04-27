use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create wallet_balance table for tracking balances
        manager
            .create_table(
                Table::create()
                    .table(WalletBalance::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(WalletBalance::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(WalletBalance::WalletId).string().not_null())
                    .col(ColumnDef::new(WalletBalance::Currency).string().not_null())
                    .col(
                        ColumnDef::new(WalletBalance::AvailableBalance)
                            .decimal()
                            .not_null()
                            .default("0.00"),
                    )
                    .col(
                        ColumnDef::new(WalletBalance::PendingBalance)
                            .decimal()
                            .not_null()
                            .default("0.00"),
                    )
                    .col(
                        ColumnDef::new(WalletBalance::TotalBalance)
                            .decimal()
                            .not_null()
                            .default("0.00"),
                    )
                    .col(
                        ColumnDef::new(WalletBalance::LastUpdated)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(WalletBalance::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-wallet-balance-wallet")
                            .from(WalletBalance::Table, WalletBalance::WalletId)
                            .to(Wallet::Table, Wallet::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create deposit_tracking table for monitoring incoming transactions
        manager
            .create_table(
                Table::create()
                    .table(DepositTracking::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DepositTracking::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::WalletId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(DepositTracking::Address).string().not_null())
                    .col(
                        ColumnDef::new(DepositTracking::Currency)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(DepositTracking::Network).string().not_null())
                    .col(ColumnDef::new(DepositTracking::TxHash).string().null())
                    .col(ColumnDef::new(DepositTracking::Amount).decimal().null())
                    .col(
                        ColumnDef::new(DepositTracking::Confirmations)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::RequiredConfirmations)
                            .integer()
                            .not_null()
                            .default(6),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::Status)
                            .string()
                            .not_null()
                            .default("monitoring"),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::BlockNumber)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::DetectedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::ConfirmedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::ProcessedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(DepositTracking::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-deposit-tracking-wallet")
                            .from(DepositTracking::Table, DepositTracking::WalletId)
                            .to(Wallet::Table, Wallet::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create wallet_transaction table for detailed transaction history
        manager
            .create_table(
                Table::create()
                    .table(WalletTransaction::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(WalletTransaction::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::WalletId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::TransactionType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::Currency)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::Amount)
                            .decimal()
                            .not_null(),
                    )
                    .col(ColumnDef::new(WalletTransaction::Fee).decimal().null())
                    .col(
                        ColumnDef::new(WalletTransaction::NetAmount)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::BalanceBefore)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::BalanceAfter)
                            .decimal()
                            .not_null(),
                    )
                    .col(ColumnDef::new(WalletTransaction::TxHash).string().null())
                    .col(
                        ColumnDef::new(WalletTransaction::FromAddress)
                            .string()
                            .null(),
                    )
                    .col(ColumnDef::new(WalletTransaction::ToAddress).string().null())
                    .col(
                        ColumnDef::new(WalletTransaction::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::Description)
                            .string()
                            .null(),
                    )
                    .col(ColumnDef::new(WalletTransaction::Metadata).string().null())
                    .col(
                        ColumnDef::new(WalletTransaction::RelatedEntityType)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::RelatedEntityId)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(WalletTransaction::ProcessedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-wallet-transaction-wallet")
                            .from(WalletTransaction::Table, WalletTransaction::WalletId)
                            .to(Wallet::Table, Wallet::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop tables in reverse order
        manager
            .drop_table(Table::drop().table(WalletTransaction::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(DepositTracking::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(WalletBalance::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum WalletBalance {
    Table,
    Id,
    WalletId,
    Currency,
    AvailableBalance,
    PendingBalance,
    TotalBalance,
    LastUpdated,
    CreatedAt,
}

#[derive(DeriveIden)]
enum DepositTracking {
    Table,
    Id,
    WalletId,
    Address,
    Currency,
    Network,
    TxHash,
    Amount,
    Confirmations,
    RequiredConfirmations,
    Status,
    BlockNumber,
    DetectedAt,
    ConfirmedAt,
    ProcessedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum WalletTransaction {
    Table,
    Id,
    WalletId,
    TransactionType,
    Currency,
    Amount,
    Fee,
    NetAmount,
    BalanceBefore,
    BalanceAfter,
    TxHash,
    FromAddress,
    ToAddress,
    Status,
    Description,
    Metadata,
    RelatedEntityType,
    RelatedEntityId,
    CreatedAt,
    ProcessedAt,
}

#[derive(DeriveIden)]
enum Wallet {
    Table,
    Id,
}
