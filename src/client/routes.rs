use crate::client::middleware::jwt_middleware::JwtMiddleware;
use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    // println!("=== Configuring client routes ===");

    cfg.service(
        web::scope("/client")
            // Public routes (no JWT middleware)
            .service(crate::client::handlers::auth_handler::register)
            .service(crate::client::handlers::auth_handler::login)
            .service(crate::client::handlers::auth_handler::test_login)
            .service(crate::client::handlers::wallet_handler::test_handler)
            // Wallet routes with JWT middleware
            .service(
                web::scope("/wallet")
                    .wrap(JwtMiddleware)
                    .service(crate::client::handlers::wallet_handler::test_simple_handler)
                    .service(crate::client::handlers::wallet_handler::test_jwt_handler)
                    .service(crate::client::handlers::wallet_handler::generate_wallet_handler)
                    .service(crate::client::handlers::wallet_handler::get_wallets),
            ),
    );
}
