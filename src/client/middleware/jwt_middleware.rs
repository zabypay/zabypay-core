use crate::shared::utils::jwt_auth::{
    decode_token, extract_token_from_cookie, extract_token_from_header,
};
use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use futures::future::{ok, LocalBoxFuture, Ready};
use std::rc::Rc;

pub struct JwtMiddleware;

impl<S, B> Transform<S, ServiceRequest> for JwtMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = JwtMiddlewareMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(JwtMiddlewareMiddleware {
            service: Rc::new(service),
        })
    }
}

pub struct JwtMiddlewareMiddleware<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for JwtMiddlewareMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();

        Box::pin(async move {
            let token = extract_token_from_header(req.request());

            match token {
                Some(token_str) => {
                    match decode_token(&token_str) {
                        Ok(claims) => {
                            req.extensions_mut().insert(claims);
                            service.call(req).await
                        }
                        Err(_) => Err(actix_web::error::ErrorUnauthorized("Invalid token")),
                    }
                }
                None => Err(actix_web::error::ErrorUnauthorized(
                    "Authentication required",
                )),
            }
        })
    }
}
