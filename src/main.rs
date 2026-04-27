mod client;
mod merchant;
mod shared;
mod testnet;

use shared::db::get_db_connection;
use shared::service::payment_monitor::PaymentMonitorService;
use shared::service::payment_scheduler::start_payment_scheduler;
use shared::service::webhook_service::start_webhook_retry_service;
use shared::utils::constants;
use shared::utils::cors::cors_middleware;
use shared::utils::production_config::ProductionConfig;
use shared::AppState;

use actix_web::{
    middleware::{Compress, Logger},
    web, App, HttpServer,
};
use log::{error, info};
use sea_orm::{ConnectionTrait, DatabaseConnection};
use std::sync::Arc;

/// Validate required environment variables at startup
fn validate_required_env_vars() -> Result<(), String> {
    let required_vars = ["DATABASE_URL", "ENVIRONMENT"];

    for var in &required_vars {
        std::env::var(var)
            .map_err(|_| format!("Required environment variable {} is missing", var))?;
    }

    Ok(())
}

/// Validate database schema at startup with proper migration version check
async fn validate_database_schema(db: &DatabaseConnection) -> Result<String, String> {
    use sea_orm::Statement;

    // Check migration version
    let version_query = Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT version FROM seaql_migrations ORDER BY version DESC LIMIT 1",
    );

    let version_result = db
        .query_one(version_query)
        .await
        .map_err(|e| format!("Failed to query migration version: {}", e))?;

    let current_version = match version_result {
        Some(row) => {
            let version: String = row
                .try_get("", "version")
                .map_err(|e| format!("Failed to extract version: {}", e))?;
            version
        }
        None => return Err("No migrations found in database".to_string()),
    };

    // Check required tables exist using INFORMATION_SCHEMA
    let required_tables = ["merchant", "wallet", "wallet_balance", "payment_request"];

    for table in &required_tables {
        let table_check = Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name = '{}'", table)
        );

        let table_result = db
            .query_one(table_check)
            .await
            .map_err(|e| format!("Failed to check table {}: {}", table, e))?;

        if table_result.is_none() {
            return Err(format!("Required table '{}' not found", table));
        }
    }

    // Check required columns for wallet_balance table
    let required_columns = [
        ("wallet_balance", "available_balance"),
        ("wallet_balance", "pending_balance"),
        ("wallet_balance", "total_balance"),
        ("wallet", "currency"),
        ("wallet", "user_id"),
        ("merchant", "id"),
        ("merchant", "user_id"),
    ];

    for (table, column) in &required_columns {
        let column_check = Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT column_name FROM information_schema.columns WHERE table_name = '{}' AND column_name = '{}'", table, column)
        );

        let column_result = db
            .query_one(column_check)
            .await
            .map_err(|e| format!("Failed to check column {}.{}: {}", table, column, e))?;

        if column_result.is_none() {
            return Err(format!("Required column '{}.{}' not found", table, column));
        }
    }

    Ok(current_version)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Starting Crypto Payment Gateway...");

    // Load environment variables
    dotenv::dotenv().ok();

    // Initialize production configuration
    ProductionConfig::init_logging();

    // Validate required environment variables (fail fast)
    if let Err(e) = validate_required_env_vars() {
        error!("Required environment variable validation failed: {}", e);
        std::process::exit(1);
    }

    // Validate additional environment variables
    if let Err(e) = ProductionConfig::validate_env_vars() {
        error!("Environment validation failed: {}", e);
        std::process::exit(1);
    }

    let is_production = ProductionConfig::is_production();
    info!("Production mode: {}", is_production);

    println!("Server Configuration:");
    println!(
        "   Main API: http://{}:{}",
        constants::ADDRESS.as_str(),
        *constants::PORT
    );

    // Database connection
    let db = get_db_connection().await;

    // Log database connection info (without exposing credentials)
    info!("🗃️  Database connection established successfully");

    // Validate database schema at startup
    println!(" Validating database schema...");
    match validate_database_schema(&db).await {
        Ok(version) => {
            println!("  Database schema validation passed");
            info!("📋 Schema version: {}", version);
        }
        Err(e) => {
            error!("Database schema validation failed: {}", e);
            error!("This indicates the database schema is out of date or corrupted");
            error!("Please run migrations: cargo run --bin migration");
            std::process::exit(1);
        }
    }

    // Initialize Payment Monitoring Service first
    println!("Initializing Payment Monitoring Service...");
    let temp_app_state = Arc::new(AppState {
        db: db.clone(),
        payment_monitor: None,
    });
    let payment_monitor = Arc::new(PaymentMonitorService::new(temp_app_state));

    // Create app state with monitoring service
    let app_state = Arc::new(AppState {
        db,
        payment_monitor: Some(payment_monitor.clone()),
    });
    let webhook_app_state = app_state.clone();

    // Start webhook retry service
    println!("Starting webhook retry service...");
    start_webhook_retry_service(webhook_app_state).await;

    // Auto-start monitoring service (optional - can be controlled via API)
    match payment_monitor.start_monitoring().await {
        Ok(_) => println!("Payment monitoring service started successfully"),
        Err(e) => {
            println!("Payment monitoring service failed to start: {}", e);
            println!("You can start it manually via API: POST /api/v1/merchant/monitoring/start");
        }
    }

    // Start automatic payment scheduler
    println!("Starting automatic payment scheduler...");
    match start_payment_scheduler(app_state.clone()).await {
        Ok(_) => println!("Automatic payment scheduler started successfully"),
        Err(e) => {
            println!("Payment scheduler failed to start: {}", e);
            println!("Payment monitoring will need to be triggered manually");
        }
    }

    println!("Configuring routes...");
    println!("Merchant routes: /merchant/*");
    println!("Monitoring routes: /merchant/monitoring/*");
    println!("Testnet routes: /testnet/*");

    println!(
        "Main API Server starting on http://{}:{}",
        constants::ADDRESS.as_str(),
        *constants::PORT
    );

    HttpServer::new(move || {
        let app = App::new()
            .app_data(web::Data::from(app_state.clone()))
            .wrap(Logger::default())
            .wrap(cors_middleware())
            .wrap(Compress::default())
            .service(
                web::scope("/api/v1")
                    .configure(client::routes::configure)
                    .configure(merchant::routes::configure)
                    .configure(testnet::routes::configure_testnet_routes),
            )
            .route(
                "/health",
                web::get().to(|| async { web::Json(ProductionConfig::get_health_check_info()) }),
            );

        if is_production {
            info!("Production mode enabled with enhanced security");
        }

        app
    })
    .bind(format!(
        "{}:{}",
        constants::ADDRESS.as_str(),
        *constants::PORT
    ))?
    .run()
    .await
}
