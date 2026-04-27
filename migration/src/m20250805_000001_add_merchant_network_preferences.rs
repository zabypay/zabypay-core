use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add environment_type column to merchant table
        manager
            .alter_table(
                Table::alter()
                    .table(Merchant::Table)
                    .add_column(
                        ColumnDef::new(Merchant::EnvironmentType)
                            .string()
                            .not_null()
                            .default("mainnet"),
                    )
                    .to_owned(),
            )
            .await?;

        // Add preferred_networks column to merchant table
        manager
            .alter_table(
                Table::alter()
                    .table(Merchant::Table)
                    .add_column(ColumnDef::new(Merchant::PreferredNetworks).text().null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove preferred_networks column
        manager
            .alter_table(
                Table::alter()
                    .table(Merchant::Table)
                    .drop_column(Merchant::PreferredNetworks)
                    .to_owned(),
            )
            .await?;

        // Remove environment_type column
        manager
            .alter_table(
                Table::alter()
                    .table(Merchant::Table)
                    .drop_column(Merchant::EnvironmentType)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(Iden)]
enum Merchant {
    Table,
    EnvironmentType,
    PreferredNetworks,
}
