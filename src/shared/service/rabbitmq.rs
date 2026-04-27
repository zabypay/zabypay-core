use crate::shared::utils::errors::AppError;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use lapin::{
    options::*, publisher_confirm::Confirmation, types::FieldTable, BasicProperties, Connection,
    ConnectionProperties, Consumer, Result as LapinResult,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

// Message types for wallet operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WalletMessage {
    CreateWallet {
        user_id: String,
        currency: String,
        request_id: String,
        timestamp: DateTime<Utc>,
    },
    UpdateBalance {
        wallet_id: String,
        currency: String,
        amount: Decimal,
        transaction_type: String, // "deposit", "withdrawal", "payment"
        transaction_id: String,
        metadata: Option<serde_json::Value>,
        timestamp: DateTime<Utc>,
    },
    DetectDeposit {
        wallet_id: String,
        address: String,
        currency: String,
        network: String,
        tx_hash: String,
        amount: Decimal,
        block_number: Option<u64>,
        confirmations: u32,
        timestamp: DateTime<Utc>,
    },
    ConfirmDeposit {
        deposit_id: String,
        tx_hash: String,
        confirmations: u32,
        final_amount: Decimal,
        timestamp: DateTime<Utc>,
    },
    ProcessWithdrawal {
        wallet_id: String,
        to_address: String,
        currency: String,
        amount: Decimal,
        fee: Option<Decimal>,
        request_id: String,
        timestamp: DateTime<Utc>,
    },
    SyncWallet {
        wallet_id: String,
        currency: String,
        network: String,
        last_synced_block: Option<u64>,
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMetadata {
    pub message_id: String,
    pub correlation_id: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct RabbitMQConfig {
    pub connection_url: String,
    pub exchange_name: String,
    pub wallet_queue: String,
    pub balance_queue: String,
    pub deposit_queue: String,
    pub withdrawal_queue: String,
    pub sync_queue: String,
    pub dead_letter_queue: String,
    pub max_retries: u32,
    pub message_ttl: u32, // seconds
}

impl Default for RabbitMQConfig {
    fn default() -> Self {
        Self {
            connection_url: "amqp://127.0.0.1:5672".to_string(),
            exchange_name: "wallet_exchange".to_string(),
            wallet_queue: "wallet_operations".to_string(),
            balance_queue: "balance_updates".to_string(),
            deposit_queue: "deposit_tracking".to_string(),
            withdrawal_queue: "withdrawal_processing".to_string(),
            sync_queue: "wallet_sync".to_string(),
            dead_letter_queue: "wallet_dlq".to_string(),
            max_retries: 3,
            message_ttl: 3600, // 1 hour
        }
    }
}

pub struct RabbitMQService {
    connection: Arc<Connection>,
    config: RabbitMQConfig,
    consumers: Arc<Mutex<Vec<Consumer>>>,
}

impl RabbitMQService {
    pub async fn new(config: RabbitMQConfig) -> Result<Self, AppError> {
        let connection =
            Connection::connect(&config.connection_url, ConnectionProperties::default())
                .await
                .map_err(|e| {
                    AppError::InternalServerError(format!("Failed to connect to RabbitMQ: {}", e))
                })?;

        let service = Self {
            connection: Arc::new(connection),
            config,
            consumers: Arc::new(Mutex::new(Vec::new())),
        };

        service.setup_queues().await?;
        Ok(service)
    }

    async fn setup_queues(&self) -> Result<(), AppError> {
        let channel = self.connection.create_channel().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create channel: {}", e))
        })?;

        // Declare exchange
        channel
            .exchange_declare(
                &self.config.exchange_name,
                lapin::ExchangeKind::Topic,
                ExchangeDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to declare exchange: {}", e))
            })?;

        // Setup dead letter queue first
        let mut dlq_args = FieldTable::default();
        dlq_args.insert(
            "x-message-ttl".into(),
            (self.config.message_ttl * 1000).into(),
        );

        channel
            .queue_declare(
                &self.config.dead_letter_queue,
                QueueDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                dlq_args,
            )
            .await
            .map_err(|e| AppError::InternalServerError(format!("Failed to declare DLQ: {}", e)))?;

        // Setup main queues with DLQ routing
        let queues = vec![
            (&self.config.wallet_queue, "wallet.*"),
            (&self.config.balance_queue, "balance.*"),
            (&self.config.deposit_queue, "deposit.*"),
            (&self.config.withdrawal_queue, "withdrawal.*"),
            (&self.config.sync_queue, "sync.*"),
        ];

        for (queue_name, routing_key) in queues {
            let mut queue_args = FieldTable::default();
            queue_args.insert(
                "x-dead-letter-exchange".into(),
                lapin::types::AMQPValue::LongString("".into()),
            );
            queue_args.insert(
                "x-dead-letter-routing-key".into(),
                lapin::types::AMQPValue::LongString(self.config.dead_letter_queue.clone().into()),
            );
            queue_args.insert(
                "x-message-ttl".into(),
                (self.config.message_ttl * 1000).into(),
            );

            channel
                .queue_declare(
                    queue_name,
                    QueueDeclareOptions {
                        durable: true,
                        ..Default::default()
                    },
                    queue_args,
                )
                .await
                .map_err(|e| {
                    AppError::InternalServerError(format!(
                        "Failed to declare queue {}: {}",
                        queue_name, e
                    ))
                })?;

            channel
                .queue_bind(
                    queue_name,
                    &self.config.exchange_name,
                    routing_key,
                    QueueBindOptions::default(),
                    FieldTable::default(),
                )
                .await
                .map_err(|e| {
                    AppError::InternalServerError(format!(
                        "Failed to bind queue {}: {}",
                        queue_name, e
                    ))
                })?;
        }

        Ok(())
    }

    pub async fn publish_wallet_message(
        &self,
        message: WalletMessage,
        routing_key: &str,
    ) -> Result<(), AppError> {
        let channel = self.connection.create_channel().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create channel: {}", e))
        })?;

        let metadata = MessageMetadata {
            message_id: Uuid::new_v4().to_string(),
            correlation_id: None,
            retry_count: 0,
            max_retries: self.config.max_retries,
            created_at: Utc::now(),
            expires_at: Some(
                Utc::now() + chrono::Duration::seconds(self.config.message_ttl as i64),
            ),
        };

        let payload = serde_json::json!({
            "message": message,
            "metadata": metadata
        });

        let payload_bytes = serde_json::to_vec(&payload).map_err(|e| {
            AppError::InternalServerError(format!("Failed to serialize message: {}", e))
        })?;

        let properties = BasicProperties::default()
            .with_message_id(metadata.message_id.clone().into())
            .with_timestamp(metadata.created_at.timestamp() as u64)
            .with_expiration((self.config.message_ttl * 1000).to_string().into())
            .with_delivery_mode(2); // Persistent

        let confirmation = channel
            .basic_publish(
                &self.config.exchange_name,
                routing_key,
                BasicPublishOptions::default(),
                &payload_bytes,
                properties,
            )
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to publish message: {}", e))
            })?
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to confirm message: {}", e))
            })?;

        match confirmation {
            Confirmation::Ack(_) => {
                log::info!("Message published successfully: {}", metadata.message_id);
                Ok(())
            }
            Confirmation::Nack(_) => Err(AppError::InternalServerError(
                "Message was not acknowledged".to_string(),
            )),
            Confirmation::NotRequested => Err(AppError::InternalServerError(
                "Publisher confirmation was not requested".to_string(),
            )),
        }
    }

    // Convenience methods for specific message types
    pub async fn create_wallet(&self, user_id: String, currency: String) -> Result<(), AppError> {
        let message = WalletMessage::CreateWallet {
            user_id,
            currency,
            request_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        };
        self.publish_wallet_message(message, "wallet.create").await
    }

    pub async fn update_balance(
        &self,
        wallet_id: String,
        currency: String,
        amount: Decimal,
        transaction_type: String,
        transaction_id: String,
        metadata: Option<serde_json::Value>,
    ) -> Result<(), AppError> {
        let message = WalletMessage::UpdateBalance {
            wallet_id,
            currency,
            amount,
            transaction_type,
            transaction_id,
            metadata,
            timestamp: Utc::now(),
        };
        self.publish_wallet_message(message, "balance.update").await
    }

    pub async fn detect_deposit(
        &self,
        wallet_id: String,
        address: String,
        currency: String,
        network: String,
        tx_hash: String,
        amount: Decimal,
        block_number: Option<u64>,
        confirmations: u32,
    ) -> Result<(), AppError> {
        let message = WalletMessage::DetectDeposit {
            wallet_id,
            address,
            currency,
            network,
            tx_hash,
            amount,
            block_number,
            confirmations,
            timestamp: Utc::now(),
        };
        self.publish_wallet_message(message, "deposit.detected")
            .await
    }

    pub async fn confirm_deposit(
        &self,
        deposit_id: String,
        tx_hash: String,
        confirmations: u32,
        final_amount: Decimal,
    ) -> Result<(), AppError> {
        let message = WalletMessage::ConfirmDeposit {
            deposit_id,
            tx_hash,
            confirmations,
            final_amount,
            timestamp: Utc::now(),
        };
        self.publish_wallet_message(message, "deposit.confirmed")
            .await
    }

    pub async fn process_withdrawal(
        &self,
        wallet_id: String,
        to_address: String,
        currency: String,
        amount: Decimal,
        fee: Option<Decimal>,
        request_id: String,
    ) -> Result<(), AppError> {
        let message = WalletMessage::ProcessWithdrawal {
            wallet_id,
            to_address,
            currency,
            amount,
            fee,
            request_id,
            timestamp: Utc::now(),
        };
        self.publish_wallet_message(message, "withdrawal.process")
            .await
    }

    pub async fn sync_wallet(
        &self,
        wallet_id: String,
        currency: String,
        network: String,
        last_synced_block: Option<u64>,
    ) -> Result<(), AppError> {
        let message = WalletMessage::SyncWallet {
            wallet_id,
            currency,
            network,
            last_synced_block,
            timestamp: Utc::now(),
        };
        self.publish_wallet_message(message, "sync.wallet").await
    }

    pub async fn start_consumer<F, Fut>(&self, queue_name: &str, handler: F) -> Result<(), AppError>
    where
        F: Fn(WalletMessage, MessageMetadata) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), AppError>> + Send + 'static,
    {
        let channel = self.connection.create_channel().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create channel: {}", e))
        })?;

        let consumer = channel
            .basic_consume(
                queue_name,
                &format!("consumer_{}", Uuid::new_v4()),
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to create consumer: {}", e))
            })?;

        let handler = Arc::new(handler);
        let channel = Arc::new(channel);

        tokio::spawn(async move {
            let mut consumer = consumer;
            while let Some(delivery) = consumer.next().await {
                match delivery {
                    Ok(delivery) => {
                        let payload: Result<serde_json::Value, _> =
                            serde_json::from_slice(&delivery.data);

                        match payload {
                            Ok(json) => {
                                if let (Some(message_json), Some(metadata_json)) =
                                    (json.get("message"), json.get("metadata"))
                                {
                                    let message_result: Result<WalletMessage, _> =
                                        serde_json::from_value(message_json.clone());
                                    let metadata_result: Result<MessageMetadata, _> =
                                        serde_json::from_value(metadata_json.clone());

                                    match (message_result, metadata_result) {
                                        (Ok(message), Ok(metadata)) => {
                                            match handler(message, metadata).await {
                                                Ok(_) => {
                                                    if let Err(e) = delivery
                                                        .ack(BasicAckOptions::default())
                                                        .await
                                                    {
                                                        log::error!("Failed to ack message: {}", e);
                                                    }
                                                }
                                                Err(e) => {
                                                    log::error!("Handler failed: {}", e);
                                                    if let Err(e) = delivery
                                                        .nack(BasicNackOptions {
                                                            requeue: false,
                                                            ..Default::default()
                                                        })
                                                        .await
                                                    {
                                                        log::error!(
                                                            "Failed to nack message: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        (Err(e), _) | (_, Err(e)) => {
                                            log::error!("Failed to deserialize message: {}", e);
                                            if let Err(e) = delivery
                                                .nack(BasicNackOptions {
                                                    requeue: false,
                                                    ..Default::default()
                                                })
                                                .await
                                            {
                                                log::error!("Failed to nack message: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to parse JSON: {}", e);
                                if let Err(e) = delivery
                                    .nack(BasicNackOptions {
                                        requeue: false,
                                        ..Default::default()
                                    })
                                    .await
                                {
                                    log::error!("Failed to nack message: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Consumer error: {}", e);
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn get_queue_info(&self, queue_name: &str) -> Result<(u32, u32), AppError> {
        let channel = self.connection.create_channel().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create channel: {}", e))
        })?;

        let queue = channel
            .queue_declare(
                queue_name,
                QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to get queue info: {}", e))
            })?;

        Ok((queue.message_count(), queue.consumer_count()))
    }

    pub async fn purge_queue(&self, queue_name: &str) -> Result<u32, AppError> {
        let channel = self.connection.create_channel().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create channel: {}", e))
        })?;

        let purged = channel
            .queue_purge(queue_name, QueuePurgeOptions::default())
            .await
            .map_err(|e| AppError::InternalServerError(format!("Failed to purge queue: {}", e)))?;

        Ok(purged)
    }
}
