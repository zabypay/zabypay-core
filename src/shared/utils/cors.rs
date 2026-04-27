use actix_cors::Cors;
use actix_web::http::header;

/// Build a CORS policy.
///
/// *In production tighten the origin list or read it from env.*
pub fn cors_middleware() -> Cors {
    Cors::default()
        .allowed_origin("http://localhost:3000")
        .allowed_origin("http://127.0.0.1:3000")
        .allowed_origin("http://localhost:3001")
        .allowed_origin("http://localhost:3002")
        .allowed_origin("http://127.0.0.1:3001")
        .allowed_origin("http://127.0.0.1:3002")
        .allowed_headers(vec![
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("x-api-key"),
        ])
        .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
        .supports_credentials()
        .max_age(3600)
}
