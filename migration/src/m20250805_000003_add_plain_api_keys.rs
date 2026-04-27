use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add plain text API key and secret key columns
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .add_column(ColumnDef::new(ApiKey::ApiKey).string().null())
                    .add_column(ColumnDef::new(ApiKey::SecretKey).string().null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove plain text API key columns
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .drop_column(ApiKey::ApiKey)
                    .drop_column(ApiKey::SecretKey)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum ApiKey {
    Table,
    ApiKey,
    SecretKey,
}
