use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .add_column(
                        ColumnDef::new(ApiKey::EnvironmentType)
                            .string()
                            .not_null()
                            .default("mainnet"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKey::Table)
                    .drop_column(ApiKey::EnvironmentType)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ApiKey {
    Table,
    EnvironmentType,
}
