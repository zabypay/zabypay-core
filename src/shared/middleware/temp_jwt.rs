use actix_web::{
    dev::{ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures::future::{ok, Ready};
use std::{
    future::{ready, Ready as StdReady},
    rc::Rc,
};

use crate::shared::models::temp_auth::UserClaims;
use crate::shared::utils::jwt_auth::{decode_token, extract_token_from_header};

/// Temporary JWT middleware to replace client module dependency
/// This is a simplified version for compatibility while client module is disabled
#[derive(Clone)]
pub struct TempJwtMiddleware;

impl<S, B> Transform<S, ServiceRequest> for TempJwtMiddleware
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>
        + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = TempJwtMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(TempJwtMiddlewareService {
            service: Rc::new(service),
        })
    }
}

pub struct TempJwtMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> actix_web::dev::Service<ServiceRequest> for TempJwtMiddlewareService<S>
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>
        + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = futures::future::LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &self,
        ctx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();

        Box::pin(async move {
            // Try to extract token from Authorization header
            let token = req
                .headers()
                .get("Authorization")
                .and_then(|auth_header| auth_header.to_str().ok())
                .and_then(|auth_str| {
                    if auth_str.starts_with("Bearer ") {
                        Some(auth_str[7..].to_string())
                    } else {
                        None
                    }
                });

            let claims = match token {
                Some(token_str) => {
                    match decode_token(&token_str) {
                        Ok(claims) => claims,
                        Err(_) => {
                            // Use fallback for invalid tokens during development
                            UserClaims {
                                sub: "parikalp.456@gmail.com".to_string(),
                                email: "parikalp.456@gmail.com".to_string(),
                                id: "a67a902d-75ba-41c7-8cca-a1864c13db55".to_string(),
                                exp: 9999999999,
                                iat: 1000000000,
                                merchant_id: Some(
                                    "37db63da-e0b3-41ff-baff-8afa062ea1e2".to_string(),
                                ),
                            }
                        }
                    }
                }
                None => {
                    // If no token provided, use fallback for development/testing
                    UserClaims {
                        sub: "parikalp.456@gmail.com".to_string(),
                        email: "parikalp.456@gmail.com".to_string(),
                        id: "a67a902d-75ba-41c7-8cca-a1864c13db55".to_string(),
                        exp: 9999999999,
                        iat: 1000000000,
                        merchant_id: Some("37db63da-e0b3-41ff-baff-8afa062ea1e2".to_string()),
                    }
                }
            };

            req.extensions_mut().insert(claims);

            service.call(req).await
        })
    }
}

/// Temporary function to create middleware instances
pub fn temp_jwt_middleware() -> TempJwtMiddleware {
    TempJwtMiddleware
}
