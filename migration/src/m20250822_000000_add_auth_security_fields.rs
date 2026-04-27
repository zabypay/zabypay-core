use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add token_version to user table for JWT invalidation
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(User::TokenVersion)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .add_column_if_not_exists(
                        ColumnDef::new(User::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .add_column_if_not_exists(
                        ColumnDef::new(User::LastPasswordChange)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Add token_version and session tracking to merchant table
        manager
            .alter_table(
                Table::alter()
                    .table(Merchant::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(Merchant::TokenVersion)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        // Create sessions table for refresh tokens and session management
        manager
            .create_table(
                Table::create()
                    .table(UserSession::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserSession::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserSession::UserId).string().not_null())
                    .col(
                        ColumnDef::new(UserSession::RefreshToken)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(UserSession::AccessTokenJti).string().null())
                    .col(ColumnDef::new(UserSession::IpAddress).string().null())
                    .col(ColumnDef::new(UserSession::UserAgent).string().null())
                    .col(
                        ColumnDef::new(UserSession::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(UserSession::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserSession::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserSession::LastUsedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-user_session-user_id")
                            .from(UserSession::Table, UserSession::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create API key usage tracking table for nonce prevention
        manager
            .create_table(
                Table::create()
                    .table(ApiKeyUsage::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ApiKeyUsage::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ApiKeyUsage::ApiKeyId).string().not_null())
                    .col(ColumnDef::new(ApiKeyUsage::Nonce).string().not_null())
                    .col(
                        ColumnDef::new(ApiKeyUsage::Timestamp)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ApiKeyUsage::Method).string().not_null())
                    .col(ColumnDef::new(ApiKeyUsage::Path).string().not_null())
                    .col(ColumnDef::new(ApiKeyUsage::IpAddress).string().null())
                    .col(
                        ColumnDef::new(ApiKeyUsage::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-api_key_usage-api_key_id")
                            .from(ApiKeyUsage::Table, ApiKeyUsage::ApiKeyId)
                            .to(ApiKey::Table, ApiKey::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create unique index for nonce to prevent replay attacks
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_api_key_usage_nonce_unique")
                    .table(ApiKeyUsage::Table)
                    .col(ApiKeyUsage::Nonce)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create composite index for API key usage queries
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_api_key_usage_api_key_timestamp")
                    .table(ApiKeyUsage::Table)
                    .col(ApiKeyUsage::ApiKeyId)
                    .col(ApiKeyUsage::Timestamp)
                    .to_owned(),
            )
            .await?;

        // Add rate limiting fields to API key table
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(ApiKey::RateLimitPerMinute)
                            .integer()
                            .default(60),
                    )
                    .add_column_if_not_exists(
                        ColumnDef::new(ApiKey::RateLimitPerHour)
                            .integer()
                            .default(1000),
                    )
                    .add_column_if_not_exists(
                        ColumnDef::new(ApiKey::LastRevokedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
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
                    .name("idx_api_key_usage_nonce_unique")
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_api_key_usage_api_key_timestamp")
                    .to_owned(),
            )
            .await?;

        // Drop tables
        manager
            .drop_table(Table::drop().table(ApiKeyUsage::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(UserSession::Table).to_owned())
            .await?;

        // Remove columns from user table
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .drop_column(User::TokenVersion)
                    .drop_column(User::IsActive)
                    .drop_column(User::LastPasswordChange)
                    .to_owned(),
            )
            .await?;

        // Remove columns from merchant table
        manager
            .alter_table(
                Table::alter()
                    .table(Merchant::Table)
                    .drop_column(Merchant::TokenVersion)
                    .to_owned(),
            )
            .await?;

        // Remove columns from api_key table
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .drop_column(ApiKey::RateLimitPerMinute)
                    .drop_column(ApiKey::RateLimitPerHour)
                    .drop_column(ApiKey::LastRevokedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum User {
    Table,
    Id,
    TokenVersion,
    IsActive,
    LastPasswordChange,
}

#[derive(DeriveIden)]
enum Merchant {
    Table,
    Id,
    TokenVersion,
}

#[derive(DeriveIden)]
enum ApiKey {
    Table,
    Id,
    RateLimitPerMinute,
    RateLimitPerHour,
    LastRevokedAt,
}

#[derive(DeriveIden)]
enum UserSession {
    Table,
    Id,
    UserId,
    RefreshToken,
    AccessTokenJti,
    IpAddress,
    UserAgent,
    IsActive,
    ExpiresAt,
    CreatedAt,
    LastUsedAt,
}

#[derive(DeriveIden)]
enum ApiKeyUsage {
    Table,
    Id,
    ApiKeyId,
    Nonce,
    Timestamp,
    Method,
    Path,
    IpAddress,
    CreatedAt,
}
