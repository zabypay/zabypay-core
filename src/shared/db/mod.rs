use std::io::{Error, ErrorKind};

use crate::shared::utils::constants;
use sea_orm::{Database, DatabaseConnection};

pub async fn run(database_url: &String) -> Result<DatabaseConnection, Error> {
    match Database::connect(database_url).await {
        Ok(pool) => Ok(pool),
        Err(e) => {
            log::error!("Failed to connect to DB: {}", e);
            return Err(Error::new(ErrorKind::Other, e.to_string()));
        }
    }
}

pub async fn get_db_connection() -> DatabaseConnection {
    use sea_orm::{ConnectOptions, DatabaseConnection};
    use std::time::Duration;

    let mut opt = ConnectOptions::new(&*constants::DATABASE_URL);
    opt.max_connections(100)
        .min_connections(5)
        .connect_timeout(Duration::from_secs(30))
        .acquire_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(3600))
        // Ensure read committed isolation level to avoid stale reads
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Debug);

    Database::connect(opt)
        .await
        .expect("Failed to connect to database")
}

pub async fn get_fresh_db_connection() -> Result<DatabaseConnection, sea_orm::DbErr> {
    use sea_orm::{ConnectOptions, DatabaseConnection};
    use std::time::Duration;

    let mut opt = ConnectOptions::new(&*constants::DATABASE_URL);
    opt.max_connections(10)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(900))
        // Force fresh connections for critical reads
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Info);

    Database::connect(opt).await
}
