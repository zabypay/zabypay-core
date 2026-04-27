use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create merchant table
        manager
            .create_table(
                Table::create()
                    .table(Merchant::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Merchant::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Merchant::UserId).string().not_null())
                    .col(ColumnDef::new(Merchant::Name).string().not_null())
                    .col(ColumnDef::new(Merchant::Description).string().null())
                    .col(ColumnDef::new(Merchant::WebsiteUrl).string().null())
                    .col(ColumnDef::new(Merchant::WebhookUrl).string().null())
                    .col(ColumnDef::new(Merchant::WebhookSecret).string().null())
                    .col(
                        ColumnDef::new(Merchant::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Merchant::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Merchant::UpdatedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-merchant-user")
                            .from(Merchant::Table, Merchant::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create api_key table
        manager
            .create_table(
                Table::create()
                    .table(ApiKey::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ApiKey::Id).string().not_null().primary_key())
                    .col(ColumnDef::new(ApiKey::MerchantId).string().not_null())
                    .col(ColumnDef::new(ApiKey::Name).string().not_null())
                    .col(ColumnDef::new(ApiKey::KeyPrefix).string().not_null())
                    .col(ColumnDef::new(ApiKey::KeyHash).string().not_null())
                    .col(ColumnDef::new(ApiKey::SecretHash).string().not_null())
                    .col(
                        ColumnDef::new(ApiKey::Permissions)
                            .string()
                            .not_null()
                            .default("[]"),
                    )
                    .col(
                        ColumnDef::new(ApiKey::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(ApiKey::LastUsed)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ApiKey::ExpiresAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ApiKey::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-apikey-merchant")
                            .from(ApiKey::Table, ApiKey::MerchantId)
                            .to(Merchant::Table, Merchant::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create payment_request table
        manager
            .create_table(
                Table::create()
                    .table(PaymentRequest::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PaymentRequest::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PaymentRequest::MerchantId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PaymentRequest::ExternalId).string().null())
                    .col(ColumnDef::new(PaymentRequest::Amount).decimal().not_null())
                    .col(ColumnDef::new(PaymentRequest::Currency).string().not_null())
                    .col(
                        ColumnDef::new(PaymentRequest::WalletAddress)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PaymentRequest::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(ColumnDef::new(PaymentRequest::Metadata).string().null())
                    .col(ColumnDef::new(PaymentRequest::PaymentUrl).string().null())
                    .col(
                        ColumnDef::new(PaymentRequest::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PaymentRequest::PaidAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(PaymentRequest::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(PaymentRequest::UpdatedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-payment-merchant")
                            .from(PaymentRequest::Table, PaymentRequest::MerchantId)
                            .to(Merchant::Table, Merchant::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create transaction table
        manager
            .create_table(
                Table::create()
                    .table(Transaction::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Transaction::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Transaction::PaymentRequestId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Transaction::TxHash).string().not_null())
                    .col(
                        ColumnDef::new(Transaction::BlockNumber)
                            .big_integer()
                            .null(),
                    )
                    .col(ColumnDef::new(Transaction::BlockHash).string().null())
                    .col(ColumnDef::new(Transaction::FromAddress).string().not_null())
                    .col(ColumnDef::new(Transaction::ToAddress).string().not_null())
                    .col(ColumnDef::new(Transaction::Amount).decimal().not_null())
                    .col(ColumnDef::new(Transaction::Currency).string().not_null())
                    .col(ColumnDef::new(Transaction::GasUsed).big_integer().null())
                    .col(ColumnDef::new(Transaction::GasPrice).string().null())
                    .col(
                        ColumnDef::new(Transaction::Confirmations)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Transaction::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(Transaction::Network)
                            .string()
                            .not_null()
                            .default("mainnet"),
                    )
                    .col(
                        ColumnDef::new(Transaction::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Transaction::ConfirmedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-transaction-payment")
                            .from(Transaction::Table, Transaction::PaymentRequestId)
                            .to(PaymentRequest::Table, PaymentRequest::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create webhook_log table
        manager
            .create_table(
                Table::create()
                    .table(WebhookLog::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(WebhookLog::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(WebhookLog::PaymentRequestId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(WebhookLog::WebhookUrl).string().not_null())
                    .col(ColumnDef::new(WebhookLog::Payload).text().not_null())
                    .col(ColumnDef::new(WebhookLog::ResponseStatus).integer().null())
                    .col(ColumnDef::new(WebhookLog::ResponseBody).text().null())
                    .col(
                        ColumnDef::new(WebhookLog::DeliveryStatus)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(WebhookLog::AttemptCount)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(
                        ColumnDef::new(WebhookLog::NextRetryAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WebhookLog::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(WebhookLog::DeliveredAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-webhook-payment")
                            .from(WebhookLog::Table, WebhookLog::PaymentRequestId)
                            .to(PaymentRequest::Table, PaymentRequest::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(WebhookLog::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Transaction::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(PaymentRequest::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(ApiKey::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Merchant::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum User {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Merchant {
    Table,
    Id,
    UserId,
    Name,
    Description,
    WebsiteUrl,
    WebhookUrl,
    WebhookSecret,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ApiKey {
    Table,
    Id,
    MerchantId,
    Name,
    KeyPrefix,
    KeyHash,
    SecretHash,
    Permissions,
    IsActive,
    LastUsed,
    ExpiresAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum PaymentRequest {
    Table,
    Id,
    MerchantId,
    ExternalId,
    Amount,
    Currency,
    WalletAddress,
    Status,
    Metadata,
    PaymentUrl,
    ExpiresAt,
    PaidAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Transaction {
    Table,
    Id,
    PaymentRequestId,
    TxHash,
    BlockNumber,
    BlockHash,
    FromAddress,
    ToAddress,
    Amount,
    Currency,
    GasUsed,
    GasPrice,
    Confirmations,
    Status,
    Network,
    CreatedAt,
    ConfirmedAt,
}

#[derive(DeriveIden)]
enum WebhookLog {
    Table,
    Id,
    PaymentRequestId,
    WebhookUrl,
    Payload,
    ResponseStatus,
    ResponseBody,
    DeliveryStatus,
    AttemptCount,
    NextRetryAt,
    CreatedAt,
    DeliveredAt,
}
