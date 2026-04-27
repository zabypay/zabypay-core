use actix_web::{web, Scope};
// use crate::client::middleware::jwt_middleware::JwtMiddleware;
use crate::shared::middleware::temp_jwt::TempJwtMiddleware as JwtMiddleware;

pub fn configure(cfg: &mut web::ServiceConfig) {
    // println!("=== Configuring merchant routes ===");

    // COMPLETELY PUBLIC routes (no authentication required) - SEPARATE from merchant scope
    cfg.service(web::scope("/public").route(
        "/payments/{id}",
        web::get().to(super::payments::get_public_payment_request),
    ));

    cfg.service(
        web::scope("/merchant")
            // Public health routes (no JWT required)
            .route("/health", web::get().to(|| async { actix_web::HttpResponse::Ok().body("Merchant routes working!") }))
            .route("/test", web::get().to(|| async { actix_web::HttpResponse::Ok().json(serde_json::json!({"message": "Merchant test working!", "success": true})) }))
            .route("/prices/health", web::get().to(super::handlers::price_handler::price_health_check))
            // Debug endpoints (no JWT required for testing)
            // USDT debug endpoints commented out - will be used later
            // .route("/debug/usdt-bep20-discovery", web::get().to(super::handlers::withdrawal_handler::debug_usdt_bep20_wallet_discovery))
            // .route("/debug/fix-bnb-usdt-currencies", web::get().to(super::handlers::withdrawal_handler::fix_bnb_usdt_wallet_currencies))
            // .route("/debug/verify-bnb-usdt-tx", web::get().to(super::handlers::withdrawal_handler::verify_bnb_usdt_transaction))
            .route("/debug/manual-confirm-payment", web::get().to(super::handlers::withdrawal_handler::manual_payment_confirmation))
            .route("/debug/trigger-payment-monitoring", web::get().to(super::handlers::withdrawal_handler::trigger_payment_monitoring))
            // Protected routes (JWT required)
            .service(
                web::scope("")
                    .wrap(JwtMiddleware)
                    .route("/merchants", web::post().to(super::management::create_merchant_handler))
                    .route("/merchants", web::get().to(super::management::list_merchants_handler))
                    .route("/merchants/{id}", web::get().to(super::management::get_merchant))
                    .route("/merchants/{id}", web::put().to(super::management::update_merchant))
                    .route("/merchants/{id}", web::delete().to(super::management::delete_merchant))
                    .route("/merchants/{id}/api-keys", web::post().to(super::management::create_api_key))
                    .route("/merchants/{id}/api-keys", web::get().to(super::management::list_api_keys))
                    .route("/merchants/{id}/api-keys/{key_id}", web::delete().to(super::management::delete_api_key))
                    .route("/merchants/{id}/api-keys/{key_id}/reveal", web::get().to(super::management::reveal_api_key))
                    .route("/networks/supported", web::get().to(super::management::get_supported_networks))
                    .route("/networks/currencies", web::get().to(super::management::get_supported_currencies))
                    .route("/payment-requests", web::get().to(super::payments::list_payment_requests_jwt))
                    .route("/payment-requests/{id}", web::get().to(super::payments::get_payment_request_jwt))
                    // Debug endpoints
                    .route("/debug/payments", web::get().to(super::handlers::debug_payments))
                    // Test endpoints for debugging withdrawal system
                    .route("/test/balance-sync", web::get().to(super::handlers::withdrawal_handler::test_balance_sync))
                    .route("/test/mainnet-balances", web::get().to(super::handlers::withdrawal_handler::test_mainnet_balance_sync))
                    .route("/test/withdrawal-list", web::get().to(super::handlers::withdrawal_handler::test_withdrawal_list))
                    .route("/test/payments", web::get().to(super::handlers::withdrawal_handler::test_payments_by_environment))
                    .route("/test/process-withdrawal/{id}", web::post().to(super::handlers::withdrawal_handler::debug_process_withdrawal))
                    .route("/test/debug-wallets", web::get().to(super::handlers::withdrawal_handler::debug_wallets))
                    // Withdrawal endpoints
                    .route("/balances", web::get().to(super::handlers::balances_handler::get_balances))
                    .route("/withdrawals", web::post().to(super::handlers::create_withdrawal))
                    .route("/withdrawals", web::get().to(super::handlers::list_withdrawals))
                    .route("/withdrawals/{id}", web::get().to(super::handlers::get_withdrawal))
                    .route("/withdrawals/{id}/cancel", web::post().to(super::handlers::cancel_withdrawal))
                    .route("/fee-estimate", web::get().to(super::handlers::estimate_fee))
                    .route("/max-withdrawable", web::get().to(super::handlers::withdrawal_handler::get_max_withdrawable))
                    // Multi-wallet endpoints (enhanced)
                    .route("/multi-wallet/balances", web::get().to(super::handlers::withdrawal_handler::get_enhanced_multi_wallet_balances))
                    .route("/multi-wallet/withdrawals/preview", web::post().to(super::handlers::withdrawal_handler::preview_enhanced_withdrawal_plan))
                    .route("/multi-wallet/withdrawals", web::post().to(super::handlers::withdrawal_handler::create_enhanced_multi_wallet_withdrawal))
                    .route("/multi-wallet/withdrawals/{id}", web::get().to(super::handlers::withdrawal_handler::get_enhanced_withdrawal_details))
                    // Price Oracle endpoints for USDT conversion
                    .route("/prices/usd", web::get().to(super::handlers::price_handler::get_usd_price))
                    .route("/prices/usd/multiple", web::post().to(super::handlers::price_handler::get_multiple_usd_prices))
                    .route("/prices/convert", web::post().to(super::handlers::price_handler::convert_currency))
                    .route("/prices/cache/stats", web::get().to(super::handlers::price_handler::get_cache_stats))
                    .route("/prices/cache/clear", web::post().to(super::handlers::price_handler::clear_cache))
            )
    );

    cfg.service(
        web::scope("/payments")
            .route(
                "/payment-requests",
                web::post().to(super::payments::create_payment_request),
            )
            .route(
                "/payment-requests",
                web::get().to(super::payments::list_payment_requests),
            )
            .route(
                "/payment-requests/{id}",
                web::get().to(super::payments::get_payment_request),
            )
            .route(
                "/payment-requests/{id}/status",
                web::put().to(super::payments::update_payment_status),
            )
            .route(
                "/payment-requests/{id}/confirm",
                web::post().to(super::payments::confirm_payment),
            ),
    );

    cfg.service(
        web::scope("/monitoring")
            .service(super::handlers::start_monitoring_service)
            .service(super::handlers::stop_monitoring_service)
            .service(super::handlers::get_monitoring_status)
            .service(super::handlers::get_monitoring_health) // Public health check
            .service(super::handlers::start_monitoring_payment)
            .service(super::handlers::stop_monitoring_payment)
            .service(super::handlers::manual_payment_check)
            .route(
                "/check-payments",
                web::post().to(super::handlers::check_payments_now),
            )
            .route(
                "/simple-stats",
                web::get().to(super::handlers::get_simple_monitoring_stats),
            ),
    );

    // WebSocket routes (temporarily disabled)
    // cfg.route("/ws/payments", web::get().to(super::handlers::websocket_payments));
    // cfg.service(
    //     web::scope("/websocket")
    //         .route("/stats", web::get().to(super::handlers::websocket_stats))
    // );

    // Payment verification routes
    cfg.service(
        web::scope("/verify")
            .route("/payment", web::post().to(super::handlers::verify_payment))
            .route(
                "/payment/{id}",
                web::get().to(super::handlers::force_check_payment),
            )
            .route(
                "/explorer",
                web::get().to(super::handlers::get_explorer_link),
            )
            .route(
                "/manual",
                web::post().to(super::handlers::verify_payment_manual),
            )
            .route(
                "/status/{id}",
                web::get().to(super::handlers::get_payment_verification_status),
            ),
    );

    // Webhook management routes
    cfg.service(
        web::scope("/webhooks")
            .wrap(JwtMiddleware)
            .route(
                "/merchants/{merchant_id}/webhooks",
                web::get().to(super::handlers::list_webhooks),
            )
            .route(
                "/webhooks/{id}",
                web::get().to(super::handlers::get_webhook_details),
            )
            .route(
                "/webhooks/{id}/resend",
                web::post().to(super::handlers::resend_webhook),
            )
            .route(
                "/merchants/{merchant_id}/webhooks/stats",
                web::get().to(super::handlers::get_webhook_stats),
            )
            .route(
                "/webhooks/test",
                web::post().to(super::handlers::test_webhook),
            ),
    );
}
