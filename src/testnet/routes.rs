use actix_web::{web, Scope};

pub fn configure_testnet_routes(cfg: &mut web::ServiceConfig) {
    // println!("=== Configuring testnet routes ===");
    // println!("Adding testnet public payment route: /testnet/public/payments/{{id}}");

    cfg.service(
        web::scope("/testnet")
            // Public info endpoint
            .route("/info", web::get().to(super::handlers::get_testnet_info))
            // Test API key management (JWT protected)
            .service(
                web::scope("/api-keys")
                    .wrap(crate::shared::middleware::temp_jwt::TempJwtMiddleware)
                    .route(
                        "/{merchant_id}/create",
                        web::post().to(super::handlers::create_test_api_key),
                    )
                    .route("/list", web::get().to(super::handlers::list_test_api_keys)),
            )
            // Payment endpoints (Test API key protected)
            .route(
                "/payment-requests",
                web::post().to(super::handlers::create_testnet_payment),
            )
            .route(
                "/payment-requests/{id}",
                web::get().to(super::handlers::get_testnet_payment),
            )
            .route(
                "/payments",
                web::post().to(super::handlers::create_testnet_payment),
            )
            .route(
                "/payments/{id}",
                web::get().to(super::handlers::get_testnet_payment),
            )
            // Testnet Monitoring endpoints (Test API key protected)
            .service(
                web::scope("/monitoring")
                    .service(super::monitoring_handlers::start_testnet_monitoring_service)
                    .service(super::monitoring_handlers::stop_testnet_monitoring_service)
                    .service(super::monitoring_handlers::get_testnet_monitoring_status)
                    .service(super::monitoring_handlers::start_monitoring_testnet_payment)
                    .service(super::monitoring_handlers::stop_monitoring_testnet_payment)
                    .service(super::monitoring_handlers::list_testnet_payments),
            )
            // Public payment endpoint (no authentication required)
            .service(web::scope("/public").route(
                "/payments/{id}",
                web::get().to(super::handlers::get_public_testnet_payment),
            )),
    );
}
