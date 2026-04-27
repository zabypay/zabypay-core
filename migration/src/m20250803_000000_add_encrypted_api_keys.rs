use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add encrypted API key and secret key columns to api_key table
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .add_column(ColumnDef::new(ApiKey::EncryptedApiKey).string().null())
                    .add_column(ColumnDef::new(ApiKey::EncryptedSecretKey).string().null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove encrypted API key columns
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .drop_column(ApiKey::EncryptedApiKey)
                    .drop_column(ApiKey::EncryptedSecretKey)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum ApiKey {
    Table,
    EncryptedApiKey,
    EncryptedSecretKey,
}
