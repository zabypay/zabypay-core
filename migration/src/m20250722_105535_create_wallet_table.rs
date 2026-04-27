use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create wallet table
        manager
            .create_table(
                Table::create()
                    .table(Wallet::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Wallet::Id).string().not_null().primary_key())
                    .col(ColumnDef::new(Wallet::UserId).string().not_null())
                    .col(ColumnDef::new(Wallet::Currency).string().not_null())
                    .col(ColumnDef::new(Wallet::Address).string().not_null())
                    .col(ColumnDef::new(Wallet::PublicKey).string().not_null())
                    .col(ColumnDef::new(Wallet::PrivateKey).string().not_null())
                    .col(ColumnDef::new(Wallet::Mnemonic).string().not_null())
                    .col(
                        ColumnDef::new(Wallet::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-wallet-user")
                            .from(Wallet::Table, Wallet::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Wallet::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Wallet {
    Table,
    Id,
    UserId,
    Currency,
    Address,
    PublicKey,
    PrivateKey,
    Mnemonic,
    CreatedAt,
}

#[derive(DeriveIden)]
enum User {
    Table,
    Id,
}
