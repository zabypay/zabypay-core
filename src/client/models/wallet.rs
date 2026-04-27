use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
pub struct WalletGenerationRequest {
    #[validate(length(
        min = 2,
        max = 10,
        message = "Currency must be between 2 and 10 characters"
    ))]
    pub currency: String,
    #[serde(default)]
    pub index: u32,
}

#[derive(Debug, Serialize)]
pub struct WalletResponse {
    pub address: String,
    pub currency: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct UserDetailsResponse {
    pub id: String,
    pub name: String,
    pub email: String,
    pub wallet: String,
}

#[derive(Serialize)]
pub struct WalletInfo {
    pub id: String,
    pub currency: String,
    pub public_key: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct DashboardResponse {
    pub user: UserInfo,
    pub wallets: Vec<WalletInfo>,
    pub total_wallets: usize,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
    pub email: String,
    pub created_at: String,
}
