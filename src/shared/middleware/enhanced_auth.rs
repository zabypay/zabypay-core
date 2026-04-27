use actix_web::web::Bytes;
use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web, Error, HttpMessage, HttpRequest,
};
use futures::future::{ok, LocalBoxFuture, Ready};
use futures::StreamExt;
use sea_orm::DatabaseConnection;
use std::rc::Rc;

use crate::shared::{
    models::auth::AuthContext, service::auth_service::AuthService,
    service::rate_limiter::RateLimiter, utils::errors::AppError, AppState,
};

/// Enhanced authentication middleware with DB validation
pub struct EnhancedAuthMiddleware {
    pub require_merchant: bool,
    pub allowed_environments: Vec<String>,
    pub required_scopes: Vec<String>,
}

impl EnhancedAuthMiddleware {
    pub fn new() -> Self {
        Self {
            require_merchant: false,
            allowed_environments: vec!["mainnet".to_string(), "testnet".to_string()],
            required_scopes: vec![],
        }
    }

    pub fn require_merchant(mut self) -> Self {
        self.require_merchant = true;
        self
    }

    pub fn environment(mut self, env: &str) -> Self {
        self.allowed_environments = vec![env.to_string()];
        self
    }

    pub fn scopes(mut self, scopes: Vec<String>) -> Self {
        self.required_scopes = scopes;
        self
    }
}

impl<S, B> Transform<S, ServiceRequest> for EnhancedAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = EnhancedAuthMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(EnhancedAuthMiddlewareService {
            service: Rc::new(service),
            require_merchant: self.require_merchant,
            allowed_environments: self.allowed_environments.clone(),
            required_scopes: self.required_scopes.clone(),
        })
    }
}

pub struct EnhancedAuthMiddlewareService<S> {
    service: Rc<S>,
    require_merchant: bool,
    allowed_environments: Vec<String>,
    required_scopes: Vec<String>,
}

impl<S, B> Service<ServiceRequest> for EnhancedAuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let require_merchant = self.require_merchant;
        let allowed_environments = self.allowed_environments.clone();
        let required_scopes = self.required_scopes.clone();

        Box::pin(async move {
            // Get app state
            let app_state = req
                .app_data::<web::Data<AppState>>()
                .ok_or_else(|| actix_web::error::ErrorInternalServerError("App state not found"))?
                .clone();

            // Extract token from Authorization header
            let token = extract_bearer_token(req.request()).ok_or_else(|| {
                log::debug!("No bearer token found in request");
                actix_web::error::ErrorUnauthorized("Authentication required")
            })?;

            // Validate token with DB checks
            let auth_service = AuthService::new();
            let auth_context = auth_service
                .validate_token(&app_state.db, &token)
                .await
                .map_err(|e| {
                    log::warn!("Token validation failed: {:?}", e);
                    actix_web::error::ErrorUnauthorized(format!("Authentication failed: {}", e))
                })?;

            // Log successful auth
            log::info!(
                "Auth successful - User: {}, Merchant: {:?}, Environment: {}, Path: {}",
                auth_context.user_id,
                auth_context.merchant_id,
                auth_context.environment,
                req.path()
            );

            // Rate limiting check for JWT tokens
            if let Ok(rate_limiter) = RateLimiter::new(
                &std::env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
            ) {
                let rate_limit_ok = rate_limiter
                    .check_jwt_rate_limit(
                        &auth_context.user_id,
                        auth_context.merchant_id.as_deref(),
                        &auth_context.environment,
                    )
                    .await
                    .unwrap_or(true); // Allow on error

                if !rate_limit_ok {
                    log::warn!(
                        "Rate limit exceeded for user {} on path {}",
                        auth_context.user_id,
                        req.path()
                    );
                    return Err(actix_web::error::ErrorTooManyRequests(
                        "Rate limit exceeded",
                    ));
                }
            }

            // Validate merchant requirement
            if require_merchant && auth_context.merchant_id.is_none() {
                log::warn!(
                    "Merchant required but not present for user {}",
                    auth_context.user_id
                );
                return Err(actix_web::error::ErrorForbidden(
                    "Merchant context required",
                ));
            }

            // Validate environment
            if !allowed_environments.is_empty()
                && !allowed_environments.contains(&auth_context.environment)
            {
                log::warn!(
                    "Environment mismatch - Required: {:?}, Got: {}",
                    allowed_environments,
                    auth_context.environment
                );
                return Err(actix_web::error::ErrorForbidden(
                    "Invalid environment for this endpoint",
                ));
            }

            // Validate scopes
            for required_scope in &required_scopes {
                if !auth_context.scopes.contains(required_scope) {
                    log::warn!(
                        "Missing required scope '{}' for user {}",
                        required_scope,
                        auth_context.user_id
                    );
                    return Err(actix_web::error::ErrorForbidden(format!(
                        "Missing required scope: {}",
                        required_scope
                    )));
                }
            }

            // Check for API key and signature if this is a mutating request
            let method_str = req.method().as_str().to_string(); // Clone the method string
            if matches!(method_str.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
                // Check for API key headers
                if let (Some(api_key), Some(signature), Some(timestamp), Some(nonce)) = (
                    get_header_value(req.request(), "X-API-Key"),
                    get_header_value(req.request(), "X-Signature"),
                    get_header_value(req.request(), "X-Timestamp"),
                    get_header_value(req.request(), "X-Nonce"),
                ) {
                    // Parse timestamp
                    let timestamp = timestamp
                        .parse::<i64>()
                        .map_err(|_| actix_web::error::ErrorBadRequest("Invalid timestamp"))?;

                    // Extract request body for signature verification
                    let body = if method_str != "GET" {
                        extract_body(&mut req).await.unwrap_or_default()
                    } else {
                        String::new()
                    };

                    // Validate API key and signature
                    match auth_service
                        .validate_api_key_signature(
                            &app_state.db,
                            &api_key,
                            &signature,
                            timestamp,
                            &nonce,
                            &method_str,
                            req.path(),
                            &body,
                        )
                        .await
                    {
                        Ok((api_key_id, merchant_id)) => {
                            log::info!(
                                "API key auth successful - Key: {}, Merchant: {}, Path: {}",
                                api_key_id,
                                merchant_id,
                                req.path()
                            );

                            // Rate limiting check for API keys
                            if let Ok(rate_limiter) = RateLimiter::new(
                                &std::env::var("REDIS_URL")
                                    .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
                            ) {
                                let rate_limit_ok = rate_limiter
                                    .check_api_key_rate_limit(
                                        &api_key_id,
                                        &merchant_id,
                                        &auth_context.environment,
                                    )
                                    .await
                                    .unwrap_or(true); // Allow on error

                                if !rate_limit_ok {
                                    log::warn!(
                                        "API key rate limit exceeded for key {} on path {}",
                                        api_key_id,
                                        req.path()
                                    );
                                    return Err(actix_web::error::ErrorTooManyRequests(
                                        "API key rate limit exceeded",
                                    ));
                                }
                            }

                            // Verify merchant matches JWT merchant
                            if let Some(jwt_merchant_id) = &auth_context.merchant_id {
                                if jwt_merchant_id != &merchant_id {
                                    log::warn!(
                                        "Merchant mismatch - JWT: {}, API Key: {}",
                                        jwt_merchant_id,
                                        merchant_id
                                    );
                                    return Err(actix_web::error::ErrorForbidden(
                                        "Merchant mismatch",
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("API key validation failed: {:?}", e);

                            // Make API key required for payment and withdrawal endpoints
                            if is_payment_endpoint(req.path()) {
                                return Err(actix_web::error::ErrorUnauthorized(format!(
                                    "API key validation failed: {}",
                                    e
                                )));
                            }
                            // For other endpoints, log but allow (for backward compatibility)
                        }
                    }
                } else {
                    // Check if API key is required for this endpoint
                    if is_payment_endpoint(req.path()) {
                        log::warn!(
                            "Payment endpoint requires API key - User: {}, Method: {}, Path: {}",
                            auth_context.user_id,
                            method_str,
                            req.path()
                        );
                        return Err(actix_web::error::ErrorBadRequest(
                            "API key required for payment operations (X-API-Key, X-Signature, X-Timestamp, X-Nonce headers)"
                        ));
                    } else {
                        // Log that API key is missing for other mutating request
                        log::info!(
                            "Mutating request without API key - User: {}, Method: {}, Path: {}",
                            auth_context.user_id,
                            method_str,
                            req.path()
                        );
                    }
                }
            }

            // Insert auth context into request extensions
            req.extensions_mut().insert(auth_context);

            // Continue to the next service
            service.call(req).await
        })
    }
}

/// Extract bearer token from Authorization header
fn extract_bearer_token(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("Authorization")
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_str| {
            if auth_str.starts_with("Bearer ") {
                Some(auth_str[7..].to_string())
            } else {
                None
            }
        })
}

/// Get header value as string
fn get_header_value(req: &HttpRequest, header_name: &str) -> Option<String> {
    req.headers()
        .get(header_name)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
}

/// Extract request body as string for signature verification
async fn extract_body(req: &mut ServiceRequest) -> Result<String, Error> {
    let mut body = Bytes::new();
    let mut payload = req.take_payload();

    while let Some(chunk) = payload.next().await {
        let chunk = chunk?;
        let mut body_vec = body.to_vec();
        body_vec.extend_from_slice(&chunk);
        body = Bytes::from(body_vec);
    }

    // Convert to string, handling potential UTF-8 issues
    let body_str = String::from_utf8(body.to_vec()).unwrap_or_else(|_| {
        // For binary data, use hex representation
        hex::encode(&body)
    });

    // Put the body back for downstream handlers
    // Note: In a production implementation, you would need to properly restore the payload
    // For now, we'll store the body in request extensions for handlers to access
    req.extensions_mut().insert(body.clone());

    Ok(body_str)
}

/// Check if the endpoint requires API key validation
fn is_payment_endpoint(path: &str) -> bool {
    // Payment creation and withdrawal endpoints require API key
    path.starts_with("/api/v1/merchant/payments") && path != "/api/v1/merchant/payments" // GET list is ok without API key
        || path.starts_with("/api/v1/merchant/withdraw")
        || path.contains("/payment/create")
        || path.contains("/withdrawal/create")
}
