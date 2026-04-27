use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create settlement table
        manager
            .create_table(
                Table::create()
                    .table(Settlement::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Settlement::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Settlement::MerchantId).string().not_null())
                    .col(
                        ColumnDef::new(Settlement::PaymentRequestId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Settlement::OriginalAmount)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Settlement::OriginalCurrency)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Settlement::SettledAmount)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Settlement::SettledCurrency)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Settlement::ConversionRate).decimal().null())
                    .col(
                        ColumnDef::new(Settlement::FeeAmount)
                            .decimal()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Settlement::FeeCurrency).string().not_null())
                    .col(
                        ColumnDef::new(Settlement::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(Settlement::PayoutAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Settlement::PayoutNetwork)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Settlement::PayoutTxHash).string().null())
                    .col(
                        ColumnDef::new(Settlement::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Settlement::SettledAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-settlement-merchant")
                            .from(Settlement::Table, Settlement::MerchantId)
                            .to(Merchant::Table, Merchant::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-settlement-payment")
                            .from(Settlement::Table, Settlement::PaymentRequestId)
                            .to(PaymentRequest::Table, PaymentRequest::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create payout_transaction table
        manager
            .create_table(
                Table::create()
                    .table(PayoutTransaction::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PayoutTransaction::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::MerchantId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::Amount)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::Currency)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::ToAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::Network)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(ColumnDef::new(PayoutTransaction::TxHash).string().null())
                    .col(
                        ColumnDef::new(PayoutTransaction::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::CompletedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(PayoutTransaction::ErrorMessage)
                            .text()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-payout-merchant")
                            .from(PayoutTransaction::Table, PayoutTransaction::MerchantId)
                            .to(Merchant::Table, Merchant::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create compliance_check table
        manager
            .create_table(
                Table::create()
                    .table(ComplianceCheck::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ComplianceCheck::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::TransactionHash)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::FromAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::ToAddress)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ComplianceCheck::Amount).decimal().not_null())
                    .col(
                        ColumnDef::new(ComplianceCheck::Currency)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::RiskScore)
                            .decimal()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::RiskFactors)
                            .text()
                            .not_null()
                            .default("[]"),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::ComplianceStatus)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(ComplianceCheck::CheckedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        // Create settlement_rule table
        manager
            .create_table(
                Table::create()
                    .table(SettlementRule::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SettlementRule::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::MerchantId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::AutoConvert)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::TargetCurrency)
                            .string()
                            .not_null()
                            .default("USDT"),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::MinSettlementAmount)
                            .decimal()
                            .not_null()
                            .default(10),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::SettlementFrequency)
                            .string()
                            .not_null()
                            .default("immediate"),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::PayoutAddresses)
                            .text()
                            .not_null()
                            .default("{}"),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(SettlementRule::UpdatedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-settlement-rule-merchant")
                            .from(SettlementRule::Table, SettlementRule::MerchantId)
                            .to(Merchant::Table, Merchant::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SettlementRule::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(ComplianceCheck::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(PayoutTransaction::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Settlement::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Merchant {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum PaymentRequest {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Settlement {
    Table,
    Id,
    MerchantId,
    PaymentRequestId,
    OriginalAmount,
    OriginalCurrency,
    SettledAmount,
    SettledCurrency,
    ConversionRate,
    FeeAmount,
    FeeCurrency,
    Status,
    PayoutAddress,
    PayoutNetwork,
    PayoutTxHash,
    CreatedAt,
    SettledAt,
}

#[derive(DeriveIden)]
enum PayoutTransaction {
    Table,
    Id,
    MerchantId,
    Amount,
    Currency,
    ToAddress,
    Network,
    Status,
    TxHash,
    CreatedAt,
    CompletedAt,
    ErrorMessage,
}

#[derive(DeriveIden)]
enum ComplianceCheck {
    Table,
    Id,
    TransactionHash,
    FromAddress,
    ToAddress,
    Amount,
    Currency,
    RiskScore,
    RiskFactors,
    ComplianceStatus,
    CheckedAt,
}

#[derive(DeriveIden)]
enum SettlementRule {
    Table,
    Id,
    MerchantId,
    AutoConvert,
    TargetCurrency,
    MinSettlementAmount,
    SettlementFrequency,
    PayoutAddresses,
    Enabled,
    CreatedAt,
    UpdatedAt,
}
