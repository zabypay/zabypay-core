use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PendingUser::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PendingUser::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(PendingUser::Name).string().not_null())
                    .col(
                        ColumnDef::new(PendingUser::Email)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(PendingUser::Password).string().not_null())
                    .col(
                        ColumnDef::new(PendingUser::VerificationCode)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingUser::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PendingUser::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PendingUser::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum PendingUser {
    Table,
    Id,
    Name,
    Email,
    Password,
    VerificationCode,
    ExpiresAt,
    CreatedAt,
}
