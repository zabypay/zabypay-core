pub mod handlers;
pub mod models;
pub mod routes;
pub mod services; // New organized services // New handlers module

// Legacy files - to be migrated to services
pub mod api_keys;
pub mod management;
pub mod payments;

// Re-export new services for easy access
pub use services::{ApiKeyService, PaymentService};
