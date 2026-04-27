use std::{collections::HashMap, env, sync::Mutex};

use lazy_static::lazy_static;

use crate::shared::models::temp_auth::TempUser;

lazy_static! {
    pub static ref ADDRESS: String = set_address();
    pub static ref PORT: u16 = set_port();
    pub static ref DATABASE_URL: String = set_database();
    pub static ref REDIS_URL: String = set_redis();
    pub static ref SMTP_USER: String = set_smptuser();
    pub static ref SMTP_PASS: String = set_smptpass();
    pub static ref SMTP_HOST: u16 = set_smpthost();
    pub static ref VERIFICATION_CODES: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    pub static ref PENDING_USER: Mutex<HashMap<String, TempUser>> = Mutex::new(HashMap::new());
    pub static ref JWT_SECRET: String = set_secret();
    pub static ref PAYMENT_REQUEST_DOMAIN: String = set_domain();

    // TODO: Re-enable USDT support (ERC20 + BEP20) when stable
    pub static ref ENABLE_USDT: bool = set_enable_usdt();
}

fn set_address() -> String {
    env::var("ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn set_port() -> u16 {
    env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .expect("PORT must be a valid number")
}

fn set_database() -> String {
    env::var("DATABASE_URL").expect("DATABASE_URL must be set")
}

fn set_redis() -> String {
    env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

fn set_smptuser() -> String {
    env::var("SMTP_USER").unwrap_or_else(|_| "user".to_string())
}

fn set_smptpass() -> String {
    env::var("SMTP_PASS").unwrap_or_else(|_| "pass".to_string())
}

fn set_smpthost() -> u16 {
    env::var("SMTP_HOST")
        .unwrap_or_else(|_| "587".to_string())
        .parse()
        .expect("SMTP_HOST must be a valid number")
}

fn set_secret() -> String {
    env::var("JWT_SECRET").unwrap_or_else(|_| "your-256-bit-secret".to_string())
}

fn set_domain() -> String {
    env::var("PAYMENT_REQUEST_URL").unwrap_or_else(|_| "http://localhost:3000/pay".to_string())
}

// TODO: Re-enable USDT support (ERC20 + BEP20) when stable
fn set_enable_usdt() -> bool {
    env::var("ENABLE_USDT")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true"
}
