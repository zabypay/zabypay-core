use crate::merchant::models::merchant::MerchantEnvironmentType;
use crate::merchant::models::payment::{
    CreatePaymentRequest, PaymentListQuery, PaymentResponse, PaymentStatus,
};
use crate::merchant::services::wallet_generation_service::WalletGenerationService;
use crate::shared::entities::{merchant, payment_request, prelude::*, wallet};
use crate::shared::models::usd_pricing::{PaymentMetadata, UsdPricing};
use crate::shared::service::usd_pricing_service::USD_PRICING_SERVICE;
use crate::shared::service::wallet::{
    get_currency_decimals, validate_wallet_address, Currency, WalletService as CoreWalletService,
};
use crate::shared::utils::constants;
use crate::shared::utils::errors::AppError;
use crate::shared::utils::payment_uri::{PaymentUriGenerator, QrMetadata};
use crate::shared::AppState;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::str::FromStr;
use uuid::Uuid; // TODO: Re-enable USDT support (ERC20 + BEP20) when stable

/// Payment Processing Service
/// Handles payment request creation, validation, and status management
pub struct PaymentService;

impl PaymentService {
    /// Create a new payment request
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant` - Merchant making the request
    /// * `request` - Payment creation request
    ///
    /// # Returns
    /// * `Result<PaymentResponse, AppError>` - Created payment request
    pub async fn create_payment_request(
        app_state: &AppState,
        merchant: &merchant::Model,
        request: CreatePaymentRequest,
    ) -> Result<PaymentResponse, AppError> {
        // TODO: Re-enable USDT support (ERC20 + BEP20) when stable
        if !*constants::ENABLE_USDT {
            let currency_lower = request.currency.to_lowercase();
            if currency_lower.contains("usdt") || currency_lower == "usdt" {
                return Err(AppError::ValidationError(
                    "USDT payments are temporarily disabled. Please use ETH, BNB, or SOL instead."
                        .to_string(),
                ));
            }
        }

        // Get merchant's environment preference
        let merchant_environment = MerchantEnvironmentType::from(merchant.environment_type.clone());

        // Validate currency is supported for the merchant's environment
        if !WalletGenerationService::is_currency_supported(
            &request.currency,
            Some(merchant_environment.clone()),
        ) {
            return Err(AppError::ValidationError(format!(
                "Currency {} is not supported in {} environment",
                request.currency.to_uppercase(),
                String::from(merchant_environment.clone())
            )));
        }

        // Generate a new unique wallet address for this payment
        // Each payment should have its own unique wallet address
        let payment_id = Uuid::new_v4().to_string();
        let wallet_address = Self::generate_unique_payment_wallet(
            &app_state.db,
            &merchant.user_id,
            &request.currency,
            &payment_id,
            Some(merchant_environment.clone()),
        )
        .await?;

        // Validate and normalize currency for amount validation
        let base_currency_str = match merchant_environment {
            MerchantEnvironmentType::Testnet => "ethereum", // Base Sepolia uses ETH
            MerchantEnvironmentType::Mainnet => &request.currency,
        };
        let currency = Currency::from_str(base_currency_str)?;

        // Validate amount
        let amount = Self::validate_and_parse_amount(&request.amount, &currency)?;

        // Validate wallet address
        if !validate_wallet_address(&currency, &wallet_address)? {
            return Err(AppError::ValidationError(
                "Invalid wallet address format".to_string(),
            ));
        }

        // Set expiration time (hardcoded to 1 hour from now)
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(3600);

        // Store the effective currency (e.g., base_sepolia for testnet, original currency for mainnet)
        let effective_currency = match merchant_environment {
            MerchantEnvironmentType::Testnet => match request.currency.to_lowercase().as_str() {
                "ethereum" | "eth" | "usdt" | "usdc" | "bnb" => "base_sepolia",
                _ => &request.currency,
            },
            MerchantEnvironmentType::Mainnet => &request.currency,
        };

        // Payment ID was already generated above for unique wallet generation
        let mut custom_metadata = request.metadata;
        // Add environment info to metadata for tracking
        custom_metadata.insert(
            "environment".to_string(),
            serde_json::Value::String(String::from(merchant_environment.clone())),
        );
        custom_metadata.insert(
            "original_currency".to_string(),
            serde_json::Value::String(request.currency.clone()),
        );

        // Compute USD pricing for this payment
        let usd_pricing = USD_PRICING_SERVICE
            .compute_usd_pricing(
                amount,
                &request.currency,
                &String::from(merchant_environment.clone()),
            )
            .await;

        // Create metadata with USD pricing
        let metadata_with_usd = PaymentMetadata::from_json_with_usd_pricing(
            Some(serde_json::to_string(&custom_metadata).unwrap_or_default()),
            usd_pricing.clone(),
        )
        .unwrap_or_else(|_| serde_json::to_string(&custom_metadata).unwrap_or_default());

        let new_payment = payment_request::ActiveModel {
            id: Set(payment_id.clone()),
            merchant_id: Set(merchant.id.clone()),
            external_id: Set(Some(request.external_id.clone())),
            amount: Set(amount),
            currency: Set(effective_currency.to_string()),
            wallet_address: Set(wallet_address.clone()),
            status: Set(PaymentStatus::Pending.as_str().to_string()),
            metadata: Set(Some(metadata_with_usd)),
            payment_url: Set(Some(Self::generate_payment_url(&payment_id))),
            expires_at: Set(expires_at.into()),
            paid_at: Set(None),
            created_at: Set(chrono::Utc::now().into()),
            updated_at: Set(Some(chrono::Utc::now().into())),
            environment: Set(String::from(merchant_environment.clone())),
        };

        let saved_payment = new_payment.insert(&app_state.db).await?;

        println!(
            "✅ Payment request created: {} for {} {} (Merchant: {}, Environment: {:?})",
            saved_payment.id,
            saved_payment.amount,
            saved_payment.currency.to_uppercase(),
            merchant.name,
            merchant_environment
        );

        // TODO: Send webhook notification for payment created
        // let _ = Self::send_payment_webhook(app_state, &saved_payment, "payment.created").await;

        // Generate QR metadata with environment awareness
        let environment_str = match merchant_environment {
            MerchantEnvironmentType::Testnet => Some("testnet"),
            MerchantEnvironmentType::Mainnet => None,
        };

        // For QR generation, use the original currency the user requested, not the effective database currency
        let qr_currency = match merchant_environment {
            MerchantEnvironmentType::Testnet => &request.currency,
            MerchantEnvironmentType::Mainnet => &request.currency,
        };

        // Log QR generation inputs for debugging
        log::info!(
            "🔗 [QR_GENERATION] Building QR for BNB USDT: paymentId={}, currency={}, network={}, amount={}, address={}, environment={:?}",
            saved_payment.id,
            qr_currency,
            if qr_currency.to_lowercase().contains("usdt_bnb") || qr_currency.to_lowercase().contains("usdt_bep20") { "bsc" } else { "other" },
            saved_payment.amount,
            saved_payment.wallet_address,
            environment_str
        );

        let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
            qr_currency,
            &saved_payment.wallet_address,
            &saved_payment.amount.to_string(),
            &saved_payment.id,
            &saved_payment.external_id.as_ref().unwrap_or(&String::new()),
            environment_str,
        );

        // Log the generated QR data for debugging BNB USDT specifically
        if qr_currency.to_lowercase().contains("usdt_bnb")
            || qr_currency.to_lowercase().contains("usdt_bep20")
        {
            log::info!(
                " [BNB_USDT_QR] Generated QR data: paymentId={}, network=bsc, chainId={:?}, contractAddress={:?}, recipient={}, amountHuman={}(6dp), amountBaseUnits={:?}, finalUri={}",
                saved_payment.id,
                qr_metadata.chain_id,
                qr_metadata.token_contract,
                saved_payment.wallet_address,
                saved_payment.amount,
                qr_metadata.amount_in_base_units,
                qr_metadata.payment_uri
            );

            // Check for potential issues
            if qr_metadata.chain_id != Some(56) {
                log::warn!(
                    " [BNB_USDT_QR] Unexpected chain ID for BNB USDT: {:?}, expected: 56",
                    qr_metadata.chain_id
                );
            }
            if qr_metadata.token_contract.is_none() {
                log::warn!(" [BNB_USDT_QR] Missing token contract address for BNB USDT");
            }
            if qr_metadata.amount_in_base_units.is_none() {
                log::warn!(" [BNB_USDT_QR] Missing amount in base units for BNB USDT");
            }
        }

        // For the response, show the original currency the user requested
        let display_currency = match merchant_environment {
            MerchantEnvironmentType::Testnet => {
                // Extract original currency from metadata, fallback to effective currency
                let metadata_value: serde_json::Value = serde_json::from_str(
                    &saved_payment.metadata.as_ref().unwrap_or(&String::new()),
                )
                .unwrap_or_default();
                metadata_value
                    .get("original_currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&saved_payment.currency)
                    .to_string()
            }
            MerchantEnvironmentType::Mainnet => saved_payment.currency.clone(),
        };

        // Extract USD pricing from metadata
        let extracted_usd_pricing =
            PaymentMetadata::extract_usd_pricing(saved_payment.metadata.clone());

        Ok(PaymentResponse {
            id: saved_payment.id,
            merchant_id: saved_payment.merchant_id,
            external_id: saved_payment.external_id.unwrap_or_default(),
            amount: saved_payment.amount.to_string(),
            currency: display_currency.to_uppercase(),
            wallet_address: saved_payment.wallet_address,
            status: saved_payment.status,
            metadata: serde_json::from_str(
                &saved_payment.metadata.as_ref().unwrap_or(&String::new()),
            )
            .unwrap_or_default(),
            payment_url: saved_payment.payment_url.unwrap_or_default(),
            expires_at: saved_payment.expires_at.into(),
            paid_at: saved_payment.paid_at.map(|dt| dt.into()),
            created_at: saved_payment.created_at.into(),
            updated_at: saved_payment
                .updated_at
                .unwrap_or_else(|| chrono::Utc::now().into())
                .into(),
            qr_data: qr_metadata,
            usd_price: extracted_usd_pricing.usd_price,
            usd_value: extracted_usd_pricing.usd_value,
            price_snapshot_ts: extracted_usd_pricing.price_snapshot_ts,
            is_simulated_price: extracted_usd_pricing.is_simulated,
        })
    }

    /// Get payment request by ID
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `payment_id` - ID of the payment request
    /// * `merchant` - Merchant requesting the payment (for ownership validation)
    ///
    /// # Returns
    /// * `Result<PaymentResponse, AppError>` - Payment request data
    pub async fn get_payment_request(
        app_state: &AppState,
        payment_id: &str,
        merchant: &merchant::Model,
    ) -> Result<PaymentResponse, AppError> {
        let payment = PaymentRequest::find()
            .filter(payment_request::Column::Id.eq(payment_id))
            .filter(payment_request::Column::MerchantId.eq(&merchant.id))
            .one(&app_state.db)
            .await?;

        match payment {
            Some(p) => {
                // Generate QR metadata with environment context
                let environment_str = if p.environment == "testnet" {
                    Some("testnet")
                } else {
                    None
                };

                // Extract original currency from metadata if available (for proper QR generation)
                let metadata_value: serde_json::Value =
                    serde_json::from_str(&p.metadata.as_ref().unwrap_or(&String::new()))
                        .unwrap_or_default();
                let qr_currency = metadata_value
                    .get("original_currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&p.currency);

                let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
                    qr_currency,
                    &p.wallet_address,
                    &p.amount.to_string(),
                    &p.id,
                    &p.external_id.as_ref().unwrap_or(&String::new()),
                    environment_str,
                );

                // Extract USD pricing from metadata
                let extracted_usd_pricing =
                    PaymentMetadata::extract_usd_pricing(p.metadata.clone());

                Ok(PaymentResponse {
                    id: p.id,
                    merchant_id: p.merchant_id,
                    external_id: p.external_id.unwrap_or_default(),
                    amount: p.amount.to_string(),
                    currency: p.currency.to_uppercase(),
                    wallet_address: p.wallet_address,
                    status: p.status,
                    metadata: serde_json::from_str(&p.metadata.unwrap_or_default())
                        .unwrap_or_default(),
                    payment_url: p.payment_url.unwrap_or_default(),
                    expires_at: p.expires_at.into(),
                    paid_at: p.paid_at.map(|dt| dt.into()),
                    created_at: p.created_at.into(),
                    updated_at: p
                        .updated_at
                        .unwrap_or_else(|| chrono::Utc::now().into())
                        .into(),
                    qr_data: qr_metadata,
                    usd_price: extracted_usd_pricing.usd_price,
                    usd_value: extracted_usd_pricing.usd_value,
                    price_snapshot_ts: extracted_usd_pricing.price_snapshot_ts,
                    is_simulated_price: extracted_usd_pricing.is_simulated,
                })
            }
            None => Err(AppError::NotFound("Payment request not found".to_string())),
        }
    }

    /// List payment requests for a merchant with filtering and pagination
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant` - Merchant requesting the list
    /// * `query` - Query parameters for filtering and pagination
    ///
    /// # Returns
    /// * `Result<(Vec<PaymentResponse>, u64), AppError>` - (payments, total_count)
    pub async fn list_payment_requests(
        app_state: &AppState,
        merchant: &merchant::Model,
        query: PaymentListQuery,
    ) -> Result<(Vec<PaymentResponse>, u64), AppError> {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(10).min(100); // Max 100 items per page
        let offset = (page - 1) * limit;

        // Build query with filters
        let mut query_builder =
            PaymentRequest::find().filter(payment_request::Column::MerchantId.eq(&merchant.id));

        // Apply status filter
        if let Some(status) = &query.status {
            query_builder = query_builder.filter(payment_request::Column::Status.eq(status));
        }

        // Apply currency filter
        if let Some(currency) = &query.currency {
            let normalized_currency = currency.to_lowercase();
            query_builder =
                query_builder.filter(payment_request::Column::Currency.eq(normalized_currency));
        }

        // Apply date range filters
        if let Some(from_date) = query.from_date {
            query_builder = query_builder.filter(payment_request::Column::CreatedAt.gte(from_date));
        }

        if let Some(to_date) = query.to_date {
            query_builder = query_builder.filter(payment_request::Column::CreatedAt.lte(to_date));
        }

        // Get total count
        let total_count = query_builder.clone().count(&app_state.db).await?;

        // Get payments with pagination
        let payments = query_builder
            .order_by_desc(payment_request::Column::CreatedAt)
            .offset(offset)
            .limit(limit)
            .all(&app_state.db)
            .await?;

        let payment_responses: Vec<PaymentResponse> = payments
            .into_iter()
            .map(|p| {
                // Generate QR metadata for each payment with environment context
                let environment_str = if p.environment == "testnet" {
                    Some("testnet")
                } else {
                    None
                };

                // Extract original currency from metadata if available (for proper QR generation)
                let metadata_value: serde_json::Value =
                    serde_json::from_str(&p.metadata.as_ref().unwrap_or(&String::new()))
                        .unwrap_or_default();
                let qr_currency = metadata_value
                    .get("original_currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&p.currency);

                let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
                    qr_currency,
                    &p.wallet_address,
                    &p.amount.to_string(),
                    &p.id,
                    &p.external_id.as_ref().unwrap_or(&String::new()),
                    environment_str,
                );

                // Extract USD pricing from metadata
                let extracted_usd_pricing =
                    PaymentMetadata::extract_usd_pricing(p.metadata.clone());

                PaymentResponse {
                    id: p.id,
                    merchant_id: p.merchant_id,
                    external_id: p.external_id.unwrap_or_default(),
                    amount: p.amount.to_string(),
                    currency: p.currency.to_uppercase(),
                    wallet_address: p.wallet_address,
                    status: p.status,
                    metadata: serde_json::from_str(&p.metadata.unwrap_or_default())
                        .unwrap_or_default(),
                    payment_url: p.payment_url.unwrap_or_default(),
                    expires_at: p.expires_at.into(),
                    paid_at: p.paid_at.map(|dt| dt.into()),
                    created_at: p.created_at.into(),
                    updated_at: p
                        .updated_at
                        .unwrap_or_else(|| chrono::Utc::now().into())
                        .into(),
                    qr_data: qr_metadata,
                    usd_price: extracted_usd_pricing.usd_price,
                    usd_value: extracted_usd_pricing.usd_value,
                    price_snapshot_ts: extracted_usd_pricing.price_snapshot_ts,
                    is_simulated_price: extracted_usd_pricing.is_simulated,
                }
            })
            .collect();

        Ok((payment_responses, total_count))
    }

    /// Calculate USD volume breakdown for payment responses
    pub fn calculate_usd_volume_breakdown(
        payments: &[PaymentResponse],
    ) -> Result<
        (
            Option<Decimal>,
            Option<crate::merchant::models::payment::VolumeBreakdown>,
        ),
        AppError,
    > {
        use crate::merchant::models::payment::VolumeBreakdown;
        use rust_decimal::prelude::*;

        let mut pending_usd = Decimal::ZERO;
        let mut paid_usd = Decimal::ZERO;
        let mut expired_usd = Decimal::ZERO;
        let mut failed_usd = Decimal::ZERO;
        let mut has_any_usd_values = false;

        for payment in payments {
            if let Some(usd_value) = payment.usd_value {
                has_any_usd_values = true;
                match payment.status.as_str() {
                    "pending" => pending_usd += usd_value,
                    "paid" => paid_usd += usd_value,
                    "expired" => expired_usd += usd_value,
                    "failed" => failed_usd += usd_value,
                    _ => {} // Unknown status, skip
                }
            }
        }

        if !has_any_usd_values {
            return Ok((None, None));
        }

        let total_volume_usd = pending_usd + paid_usd + expired_usd + failed_usd;

        let breakdown = VolumeBreakdown {
            pending_usd,
            paid_usd,
            expired_usd,
            failed_usd,
        };

        Ok((Some(total_volume_usd), Some(breakdown)))
    }

    /// Update payment status (typically called by blockchain monitoring)
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `payment_id` - ID of the payment request
    /// * `new_status` - New payment status
    /// * `transaction_hash` - Optional blockchain transaction hash
    ///
    /// # Returns
    /// * `Result<PaymentResponse, AppError>` - Updated payment request
    pub async fn update_payment_status(
        app_state: &AppState,
        payment_id: &str,
        new_status: PaymentStatus,
        _transaction_hash: Option<String>,
    ) -> Result<PaymentResponse, AppError> {
        let payment = PaymentRequest::find_by_id(payment_id)
            .one(&app_state.db)
            .await?;

        let payment = match payment {
            Some(p) => p,
            None => return Err(AppError::NotFound("Payment request not found".to_string())),
        };

        // Update payment status
        let mut update_model: payment_request::ActiveModel = payment.into();
        update_model.status = Set(new_status.as_str().to_string());
        update_model.updated_at = Set(Some(chrono::Utc::now().into()));

        // Set paid_at timestamp if payment is confirmed
        if matches!(new_status, PaymentStatus::Paid) {
            update_model.paid_at = Set(Some(chrono::Utc::now().into()));
        }

        let updated_payment = update_model.update(&app_state.db).await?;

        println!(
            "✅ Payment status updated: {} -> {}",
            updated_payment.id,
            new_status.as_str()
        );

        // TODO: Create transaction record if transaction_hash is provided
        // TODO: Send webhook notification for status change

        // Generate QR metadata with environment context
        let environment_str = if updated_payment.environment == "testnet" {
            Some("testnet")
        } else {
            None
        };

        // Extract original currency from metadata if available (for proper QR generation)
        let metadata_value: serde_json::Value =
            serde_json::from_str(&updated_payment.metadata.as_ref().unwrap_or(&String::new()))
                .unwrap_or_default();
        let qr_currency = metadata_value
            .get("original_currency")
            .and_then(|v| v.as_str())
            .unwrap_or(&updated_payment.currency);

        let qr_metadata = PaymentUriGenerator::get_qr_metadata_with_environment(
            qr_currency,
            &updated_payment.wallet_address,
            &updated_payment.amount.to_string(),
            &updated_payment.id,
            &updated_payment
                .external_id
                .as_ref()
                .unwrap_or(&String::new()),
            environment_str,
        );

        // Extract USD pricing from metadata
        let extracted_usd_pricing =
            PaymentMetadata::extract_usd_pricing(updated_payment.metadata.clone());

        Ok(PaymentResponse {
            id: updated_payment.id,
            merchant_id: updated_payment.merchant_id,
            external_id: updated_payment.external_id.unwrap_or_default(),
            amount: updated_payment.amount.to_string(),
            currency: updated_payment.currency.to_uppercase(),
            wallet_address: updated_payment.wallet_address,
            status: updated_payment.status,
            metadata: serde_json::from_str(&updated_payment.metadata.unwrap_or_default())
                .unwrap_or_default(),
            payment_url: updated_payment.payment_url.unwrap_or_default(),
            expires_at: updated_payment.expires_at.into(),
            paid_at: updated_payment.paid_at.map(|dt| dt.into()),
            created_at: updated_payment.created_at.into(),
            updated_at: updated_payment
                .updated_at
                .unwrap_or_else(|| chrono::Utc::now().into())
                .into(),
            qr_data: qr_metadata,
            usd_price: extracted_usd_pricing.usd_price,
            usd_value: extracted_usd_pricing.usd_value,
            price_snapshot_ts: extracted_usd_pricing.price_snapshot_ts,
            is_simulated_price: extracted_usd_pricing.is_simulated,
        })
    }

    /// Mark expired payments as expired (typically called by background job)
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    ///
    /// # Returns
    /// * `Result<u64, AppError>` - Number of payments marked as expired
    pub async fn mark_expired_payments(app_state: &AppState) -> Result<u64, AppError> {
        let now = chrono::Utc::now();

        // Find all pending payments that have expired
        let expired_payments = PaymentRequest::find()
            .filter(payment_request::Column::Status.eq(PaymentStatus::Pending.as_str()))
            .filter(payment_request::Column::ExpiresAt.lt(now))
            .all(&app_state.db)
            .await?;

        let mut count = 0;
        for payment in expired_payments {
            let mut update_model: payment_request::ActiveModel = payment.into();
            update_model.status = Set(PaymentStatus::Expired.as_str().to_string());
            update_model.updated_at = Set(Some(now.into()));

            update_model.update(&app_state.db).await?;
            count += 1;
        }

        if count > 0 {
            println!("✅ Marked {} payments as expired", count);
        }

        Ok(count)
    }

    /// Validate and parse payment amount
    fn validate_and_parse_amount(
        amount_str: &str,
        currency: &Currency,
    ) -> Result<Decimal, AppError> {
        let amount = Decimal::from_str(amount_str)
            .map_err(|_| AppError::ValidationError("Invalid amount format".to_string()))?;

        // Validate amount is positive
        if amount <= Decimal::ZERO {
            return Err(AppError::ValidationError(
                "Payment amount must be greater than zero".to_string(),
            ));
        }

        // Validate decimal places don't exceed currency precision
        let max_decimals = get_currency_decimals(currency);
        let scale = amount.scale();

        if scale > max_decimals as u32 {
            return Err(AppError::ValidationError(format!(
                "{} supports maximum {} decimal places",
                currency.as_str().to_uppercase(),
                max_decimals
            )));
        }

        // Validate minimum amount (prevent dust attacks)
        let min_amount = Self::get_minimum_amount(currency);
        if amount < min_amount {
            return Err(AppError::ValidationError(format!(
                "Amount too small. Minimum: {} {}",
                min_amount,
                currency.as_str().to_uppercase()
            )));
        }

        // Validate maximum amount (prevent overflow)
        let max_amount = Self::get_maximum_amount(currency);
        if amount > max_amount {
            return Err(AppError::ValidationError(format!(
                "Amount too large. Maximum: {} {}",
                max_amount,
                currency.as_str().to_uppercase()
            )));
        }

        Ok(amount)
    }

    /// Get minimum payment amount for a currency
    fn get_minimum_amount(currency: &Currency) -> Decimal {
        match currency {
            Currency::Bitcoin => Decimal::from_str("0.00001").unwrap(), // 1000 satoshis
            Currency::Ethereum => Decimal::from_str("0.001").unwrap(),  // 0.001 ETH
            Currency::USDT => Decimal::from_str("0.01").unwrap(),       // 0.01 USDT
            Currency::Solana => Decimal::from_str("0.001").unwrap(),    // 0.001 SOL
            Currency::BNB => Decimal::from_str("0.001").unwrap(),       // 0.001 BNB
        }
    }

    /// Get maximum payment amount for a currency
    fn get_maximum_amount(currency: &Currency) -> Decimal {
        match currency {
            Currency::Bitcoin => Decimal::from_str("100").unwrap(), // 100 BTC
            Currency::Ethereum => Decimal::from_str("1000").unwrap(), // 1000 ETH
            Currency::USDT => Decimal::from_str("100000").unwrap(), // 100k USDT
            Currency::Solana => Decimal::from_str("100000").unwrap(), // 100k SOL
            Currency::BNB => Decimal::from_str("100000").unwrap(),  // 100k BNB
        }
    }

    /// Generate payment URL for frontend display
    fn generate_payment_url(payment_id: &str) -> String {
        // In production, this would be your actual payment page URL
        format!("https://your-domain.com/pay/{}", payment_id)
    }

    /// Generate a unique wallet address for each payment
    /// This ensures every payment has its own dedicated wallet address
    async fn generate_unique_payment_wallet(
        db: &sea_orm::DatabaseConnection,
        user_id: &str,
        currency_str: &str,
        payment_id: &str,
        environment: Option<MerchantEnvironmentType>,
    ) -> Result<String, AppError> {
        let env_type = environment.unwrap_or_default();

        // For testnet, use base_sepolia for Ethereum-based currencies
        let effective_currency = match env_type {
            MerchantEnvironmentType::Testnet => match currency_str.to_lowercase().as_str() {
                "ethereum" | "eth" | "usdt" | "usdc" | "bnb" => "base_sepolia".to_string(),
                other => other.to_string(),
            },
            MerchantEnvironmentType::Mainnet => currency_str.to_string(),
        };

        // Create a unique currency key that includes payment_id to ensure uniqueness
        let unique_currency_key = format!(
            "{}_{}_{}",
            effective_currency.to_lowercase(),
            String::from(env_type.clone()),
            payment_id
        );

        // Convert string currency to Currency enum for wallet generation
        let base_currency_str = match env_type {
            MerchantEnvironmentType::Testnet => {
                match effective_currency.as_str() {
                    "base_sepolia" => "ethereum", // Base uses Ethereum-compatible addresses
                    other => other,
                }
            }
            MerchantEnvironmentType::Mainnet => effective_currency.as_str(),
        };

        let currency = Currency::from_str(base_currency_str)?;

        // Use the core wallet service to generate a new wallet with unique key
        let wallet_service = CoreWalletService::new();
        let (address, _mnemonic) = wallet_service
            .generate_wallet_with_key(user_id, currency, &unique_currency_key, db)
            .await?;

        log::info!(
            "Generated unique {:?} {} wallet for payment {}: {}",
            env_type,
            effective_currency,
            payment_id,
            address
        );

        Ok(address)
    }

    /// Get payment statistics for a merchant
    ///
    /// # Arguments
    /// * `app_state` - Database connection state
    /// * `merchant` - Merchant to get statistics for
    /// * `days` - Number of days to look back (default: 30)
    ///
    /// # Returns
    /// * `Result<serde_json::Value, AppError>` - Payment statistics
    pub async fn get_payment_statistics(
        app_state: &AppState,
        merchant: &merchant::Model,
        days: Option<u32>,
    ) -> Result<serde_json::Value, AppError> {
        let days = days.unwrap_or(30);
        let since = chrono::Utc::now() - chrono::Duration::days(days as i64);

        // Get total payments
        let total_payments = PaymentRequest::find()
            .filter(payment_request::Column::MerchantId.eq(&merchant.id))
            .filter(payment_request::Column::CreatedAt.gte(since))
            .count(&app_state.db)
            .await?;

        // Get successful payments
        let successful_payments = PaymentRequest::find()
            .filter(payment_request::Column::MerchantId.eq(&merchant.id))
            .filter(payment_request::Column::CreatedAt.gte(since))
            .filter(payment_request::Column::Status.eq(PaymentStatus::Paid.as_str()))
            .count(&app_state.db)
            .await?;

        // Get pending payments
        let pending_payments = PaymentRequest::find()
            .filter(payment_request::Column::MerchantId.eq(&merchant.id))
            .filter(payment_request::Column::Status.eq(PaymentStatus::Pending.as_str()))
            .count(&app_state.db)
            .await?;

        // Calculate success rate
        let success_rate = if total_payments > 0 {
            let success_f64 = successful_payments as f64;
            let total_f64 = total_payments as f64;
            (success_f64 / total_f64) * 100.0
        } else {
            0.0
        };

        Ok(serde_json::json!({
            "period_days": days,
            "total_payments": total_payments,
            "successful_payments": successful_payments,
            "pending_payments": pending_payments,
            "failed_payments": total_payments.saturating_sub(successful_payments + pending_payments),
            "success_rate_percent": format!("{:.1}", success_rate),
            "merchant_id": merchant.id,
            "merchant_name": merchant.name,
        }))
    }
}
