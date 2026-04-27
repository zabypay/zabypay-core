use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create admin_roles table
        manager
            .create_table(
                Table::create()
                    .table(AdminRole::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminRole::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRole::Name)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(AdminRole::Description).text())
                    .col(ColumnDef::new(AdminRole::Permissions).json().not_null())
                    .col(
                        ColumnDef::new(AdminRole::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(AdminRole::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AdminRole::UpdatedAt).timestamp_with_time_zone())
                    .to_owned(),
            )
            .await?;

        // Create admin_users table
        manager
            .create_table(
                Table::create()
                    .table(AdminUser::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminUser::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminUser::Username)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(AdminUser::Email)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(AdminUser::PasswordHash).string().not_null())
                    .col(ColumnDef::new(AdminUser::FirstName).string().not_null())
                    .col(ColumnDef::new(AdminUser::LastName).string().not_null())
                    .col(ColumnDef::new(AdminUser::RoleId).string().not_null())
                    .col(
                        ColumnDef::new(AdminUser::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(ColumnDef::new(AdminUser::LastLoginAt).timestamp_with_time_zone())
                    .col(ColumnDef::new(AdminUser::PasswordChangedAt).timestamp_with_time_zone())
                    .col(
                        ColumnDef::new(AdminUser::FailedLoginAttempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(AdminUser::LockedUntil).timestamp_with_time_zone())
                    .col(
                        ColumnDef::new(AdminUser::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AdminUser::UpdatedAt).timestamp_with_time_zone())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_admin_user_role")
                            .from(AdminUser::Table, AdminUser::RoleId)
                            .to(AdminRole::Table, AdminRole::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Create admin_sessions table
        manager
            .create_table(
                Table::create()
                    .table(AdminSession::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminSession::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminSession::AdminUserId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminSession::TokenHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(AdminSession::IpAddress).string())
                    .col(ColumnDef::new(AdminSession::UserAgent).text())
                    .col(
                        ColumnDef::new(AdminSession::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminSession::IsActive)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(AdminSession::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AdminSession::LastUsedAt).timestamp_with_time_zone())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_admin_session_user")
                            .from(AdminSession::Table, AdminSession::AdminUserId)
                            .to(AdminUser::Table, AdminUser::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create audit_logs table
        manager
            .create_table(
                Table::create()
                    .table(AuditLog::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AuditLog::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AuditLog::AdminUserId).string())
                    .col(ColumnDef::new(AuditLog::Action).string().not_null())
                    .col(ColumnDef::new(AuditLog::Resource).string().not_null())
                    .col(ColumnDef::new(AuditLog::ResourceId).string())
                    .col(ColumnDef::new(AuditLog::OldValues).json())
                    .col(ColumnDef::new(AuditLog::NewValues).json())
                    .col(ColumnDef::new(AuditLog::IpAddress).string())
                    .col(ColumnDef::new(AuditLog::UserAgent).text())
                    .col(ColumnDef::new(AuditLog::Metadata).json())
                    .col(
                        ColumnDef::new(AuditLog::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_audit_log_admin_user")
                            .from(AuditLog::Table, AuditLog::AdminUserId)
                            .to(AdminUser::Table, AdminUser::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Create system_configs table
        manager
            .create_table(
                Table::create()
                    .table(SystemConfig::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SystemConfig::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SystemConfig::Key)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(SystemConfig::Value).json().not_null())
                    .col(ColumnDef::new(SystemConfig::Description).text())
                    .col(ColumnDef::new(SystemConfig::Category).string().not_null())
                    .col(
                        ColumnDef::new(SystemConfig::IsEncrypted)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(SystemConfig::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SystemConfig::UpdatedAt).timestamp_with_time_zone())
                    .col(ColumnDef::new(SystemConfig::UpdatedBy).string())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_system_config_admin_user")
                            .from(SystemConfig::Table, SystemConfig::UpdatedBy)
                            .to(AdminUser::Table, AdminUser::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // Create indexes for better performance
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_admin_users_role_id")
                    .table(AdminUser::Table)
                    .col(AdminUser::RoleId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_admin_sessions_token_hash")
                    .table(AdminSession::Table)
                    .col(AdminSession::TokenHash)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_audit_logs_admin_user_id")
                    .table(AuditLog::Table)
                    .col(AuditLog::AdminUserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_audit_logs_resource")
                    .table(AuditLog::Table)
                    .col(AuditLog::Resource)
                    .col(AuditLog::ResourceId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_system_configs_category")
                    .table(SystemConfig::Table)
                    .col(SystemConfig::Category)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SystemConfig::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AuditLog::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AdminSession::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AdminUser::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AdminRole::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(Iden)]
enum AdminRole {
    Table,
    Id,
    Name,
    Description,
    Permissions,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum AdminUser {
    Table,
    Id,
    Username,
    Email,
    PasswordHash,
    FirstName,
    LastName,
    RoleId,
    IsActive,
    LastLoginAt,
    PasswordChangedAt,
    FailedLoginAttempts,
    LockedUntil,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum AdminSession {
    Table,
    Id,
    AdminUserId,
    TokenHash,
    IpAddress,
    UserAgent,
    ExpiresAt,
    IsActive,
    CreatedAt,
    LastUsedAt,
}

#[derive(Iden)]
enum AuditLog {
    Table,
    Id,
    AdminUserId,
    Action,
    Resource,
    ResourceId,
    OldValues,
    NewValues,
    IpAddress,
    UserAgent,
    Metadata,
    CreatedAt,
}

#[derive(Iden)]
enum SystemConfig {
    Table,
    Id,
    Key,
    Value,
    Description,
    Category,
    IsEncrypted,
    CreatedAt,
    UpdatedAt,
    UpdatedBy,
}
