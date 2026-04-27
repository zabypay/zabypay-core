pub mod handlers;
pub mod middleware;
pub mod models;
pub mod routes;
pub mod services;

pub mod auth {
    pub use crate::client::models::auth::*;
    pub use crate::client::services::auth_service::AuthService;
}

pub mod wallet {
    pub use crate::client::models::wallet::*;
    pub use crate::client::services::wallet_service::WalletService;
}
