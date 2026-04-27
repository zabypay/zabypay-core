use sea_orm::DatabaseConnection;
use std::sync::Arc;

pub mod db;
pub mod entities;
pub mod models;
pub mod service;
// pub mod services; // Temporarily disabled - conflicts with service
pub mod middleware;
pub mod utils;
pub mod worker;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub payment_monitor: Option<Arc<service::payment_monitor::PaymentMonitorService>>,
    // pub websocket_service: Option<Arc<service::websocket_service::WebSocketPaymentService>>, // Temporarily disabled
}
