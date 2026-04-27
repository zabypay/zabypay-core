#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test;
    use sea_orm::{Database, DatabaseConnection};

    // Mock database connection for testing
    async fn create_test_db() -> DatabaseConnection {
        // In a real test, you would use a test database
        // For now, this is a placeholder
        Database::connect("sqlite::memory:")
            .await
            .expect("Failed to create test database")
    }

    #[tokio::test]
    async fn test_balances_service_new() {
        let service = BalancesService::new();
        assert!(service.cache_ttl_seconds >= 1);
    }

    #[tokio::test]
    async fn test_validation_error_creation() {
        let error = BalancesError::validation_error("Test error".to_string());
        assert_eq!(error.code, "VALIDATION_ERROR");
        assert_eq!(error.message, "Test error");
        assert!(error.retry_after.is_none());
    }

    #[tokio::test]
    async fn test_db_unavailable_error() {
        let error = BalancesError::db_unavailable(None);
        assert_eq!(error.code, "DB_UNAVAILABLE");
        assert_eq!(error.retry_after, Some(30));
    }

    #[tokio::test]
    async fn test_schema_out_of_date_error() {
        let error = BalancesError::schema_out_of_date(None);
        assert_eq!(error.code, "DB_SCHEMA_OUT_OF_DATE");
        assert!(error.retry_after.is_none());
    }

    #[tokio::test]
    async fn test_extract_currency_and_network() {
        let service = BalancesService::new();

        // Test BNB USDT
        let (currency, network) = service.extract_currency_and_network("usdt_bnb_mainnet", "mainnet");
        assert_eq!(currency, "usdt_bnb");
        assert_eq!(network, "usdt_bep20");

        // Test ETH USDT
        let (currency, network) = service.extract_currency_and_network("usdt_mainnet", "mainnet");
        assert_eq!(currency, "usdt");
        assert_eq!(network, "usdt_erc20");

        // Test ETH
        let (currency, network) = service.extract_currency_and_network("eth_mainnet", "mainnet");
        assert_eq!(currency, "eth");
        assert_eq!(network, "eth");

        // Test BTC
        let (currency, network) = service.extract_currency_and_network("btc_mainnet", "mainnet");
        assert_eq!(currency, "btc");
        assert_eq!(network, "btc");
    }

    #[tokio::test]
    async fn test_get_network_info() {
        let service = BalancesService::new();

        // Test ETH
        let info = service.get_network_info("eth", "eth", "mainnet");
        assert_eq!(info.symbol, "ETH");
        assert_eq!(info.decimals, 18);

        // Test BTC
        let info = service.get_network_info("btc", "btc", "mainnet");
        assert_eq!(info.symbol, "BTC");
        assert_eq!(info.decimals, 8);

        // Test USDT BNB
        let info = service.get_network_info("usdt_bnb", "usdt_bep20", "mainnet");
        assert_eq!(info.symbol, "USDT");
        assert_eq!(info.decimals, 18);

        // Test USDT ETH
        let info = service.get_network_info("usdt", "usdt_erc20", "mainnet");
        assert_eq!(info.symbol, "USDT");
        assert_eq!(info.decimals, 6);
    }
}