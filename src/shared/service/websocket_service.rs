use actix::prelude::*;
use actix_web_actors::ws;
use chrono;
use log;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// WebSocket connection handler for real-time payment updates
pub struct PaymentWebSocket {
    id: Uuid,
    payment_id: Option<String>,
    connection_manager: Arc<RwLock<WebSocketConnectionManager>>,
}

impl PaymentWebSocket {
    pub fn new(connection_manager: Arc<RwLock<WebSocketConnectionManager>>) -> Self {
        Self {
            id: Uuid::new_v4(),
            payment_id: None,
            connection_manager,
        }
    }
}

impl Actor for PaymentWebSocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::info!("🔗 WebSocket connection established: {}", self.id);

        let connection_manager = self.connection_manager.clone();
        let connection_id = self.id;
        let addr = ctx.address();

        tokio::spawn(async move {
            let mut manager = connection_manager.write().await;
            manager.add_connection(connection_id, addr);
            log::info!("📝 WebSocket connection {} registered", connection_id);
        });
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        log::info!(" WebSocket connection closed: {}", self.id);

        let connection_manager = self.connection_manager.clone();
        let connection_id = self.id;

        tokio::spawn(async move {
            let mut manager = connection_manager.write().await;
            manager.remove_connection(&connection_id);
            log::info!("🗑️ WebSocket connection {} unregistered", connection_id);
        });
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for PaymentWebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Pong(_)) => {
                log::debug!("🏓 WebSocket pong received from {}", self.id);
            }
            Ok(ws::Message::Text(text)) => {
                log::info!("📨 WebSocket message from {}: {}", self.id, text);

                // Handle subscription to payment updates
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(action) = parsed.get("action").and_then(|a| a.as_str()) {
                        match action {
                            "subscribe" => {
                                if let Some(payment_id) =
                                    parsed.get("payment_id").and_then(|p| p.as_str())
                                {
                                    self.payment_id = Some(payment_id.to_string());
                                    log::info!(
                                        " WebSocket {} subscribed to payment {}",
                                        self.id,
                                        payment_id
                                    );

                                    let response = json!({
                                        "type": "subscription_confirmed",
                                        "payment_id": payment_id,
                                        "connection_id": self.id.to_string()
                                    });
                                    ctx.text(response.to_string());

                                    // Register this connection for payment updates
                                    let connection_manager = self.connection_manager.clone();
                                    let connection_id = self.id;
                                    let payment_id_clone = payment_id.to_string();

                                    tokio::spawn(async move {
                                        let mut manager = connection_manager.write().await;
                                        manager
                                            .subscribe_to_payment(connection_id, payment_id_clone);
                                    });
                                }
                            }
                            "unsubscribe" => {
                                if let Some(payment_id) = &self.payment_id {
                                    log::info!(
                                        "🔕 WebSocket {} unsubscribed from payment {}",
                                        self.id,
                                        payment_id
                                    );

                                    let connection_manager = self.connection_manager.clone();
                                    let connection_id = self.id;
                                    let payment_id_clone = payment_id.clone();

                                    tokio::spawn(async move {
                                        let mut manager = connection_manager.write().await;
                                        manager.unsubscribe_from_payment(
                                            &connection_id,
                                            &payment_id_clone,
                                        );
                                    });

                                    self.payment_id = None;
                                }
                            }
                            "ping" => {
                                let pong = json!({
                                    "type": "pong",
                                    "timestamp": chrono::Utc::now().to_rfc3339()
                                });
                                ctx.text(pong.to_string());
                            }
                            _ => {
                                log::warn!("🚫 Unknown WebSocket action: {}", action);
                            }
                        }
                    }
                }
            }
            Ok(ws::Message::Binary(_)) => {
                log::warn!("📦 WebSocket binary message not supported");
            }
            Ok(ws::Message::Close(reason)) => {
                log::info!("👋 WebSocket close received: {:?}", reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

/// Manages WebSocket connections and handles broadcasting payment updates
#[derive(Default)]
pub struct WebSocketConnectionManager {
    connections: HashMap<Uuid, Addr<PaymentWebSocket>>,
    payment_subscriptions: HashMap<String, Vec<Uuid>>,
}

impl WebSocketConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_connection(&mut self, id: Uuid, addr: Addr<PaymentWebSocket>) {
        self.connections.insert(id, addr);
        log::info!(
            "🔗 Added WebSocket connection: {} (total: {})",
            id,
            self.connections.len()
        );
    }

    pub fn remove_connection(&mut self, id: &Uuid) {
        self.connections.remove(id);

        // Remove from all payment subscriptions
        for subscribers in self.payment_subscriptions.values_mut() {
            subscribers.retain(|conn_id| conn_id != id);
        }

        // Clean up empty subscription lists
        self.payment_subscriptions
            .retain(|_, subscribers| !subscribers.is_empty());

        log::info!(
            "🗑️ Removed WebSocket connection: {} (remaining: {})",
            id,
            self.connections.len()
        );
    }

    pub fn subscribe_to_payment(&mut self, connection_id: Uuid, payment_id: String) {
        let subscribers = self
            .payment_subscriptions
            .entry(payment_id.clone())
            .or_insert_with(Vec::new);
        if !subscribers.contains(&connection_id) {
            subscribers.push(connection_id);
            log::info!(
                " Connection {} subscribed to payment {} (total subscribers: {})",
                connection_id,
                payment_id,
                subscribers.len()
            );
        }
    }

    pub fn unsubscribe_from_payment(&mut self, connection_id: &Uuid, payment_id: &str) {
        if let Some(subscribers) = self.payment_subscriptions.get_mut(payment_id) {
            subscribers.retain(|id| id != connection_id);
            log::info!(
                "🔕 Connection {} unsubscribed from payment {}",
                connection_id,
                payment_id
            );

            if subscribers.is_empty() {
                self.payment_subscriptions.remove(payment_id);
                log::info!(
                    "🗑️ Removed empty subscription list for payment {}",
                    payment_id
                );
            }
        }
    }

    /// Broadcast payment status update to all subscribed connections
    pub async fn broadcast_payment_update(
        &self,
        payment_id: &str,
        status: &str,
        transaction_hash: Option<&str>,
    ) {
        if let Some(subscriber_ids) = self.payment_subscriptions.get(payment_id) {
            log::info!(
                "📢 Broadcasting payment update for {} to {} subscribers",
                payment_id,
                subscriber_ids.len()
            );

            let update = json!({
                "type": "payment_update",
                "payment_id": payment_id,
                "status": status,
                "transaction_hash": transaction_hash,
                "timestamp": chrono::Utc::now().to_rfc3339()
            });

            let message_text = update.to_string();
            let mut successful_broadcasts = 0;

            for subscriber_id in subscriber_ids {
                if let Some(connection) = self.connections.get(subscriber_id) {
                    connection.do_send(PaymentUpdateMessage {
                        content: message_text.clone(),
                    });
                    successful_broadcasts += 1;
                } else {
                    log::warn!(
                        "🚫 Could not find connection for subscriber: {}",
                        subscriber_id
                    );
                }
            }

            log::info!(
                "✅ Broadcasted payment update to {}/{} subscribers",
                successful_broadcasts,
                subscriber_ids.len()
            );
        } else {
            log::debug!("📭 No subscribers for payment {}", payment_id);
        }
    }

    /// Get connection statistics
    pub fn get_stats(&self) -> (usize, usize) {
        (self.connections.len(), self.payment_subscriptions.len())
    }
}

/// Message for sending payment updates to WebSocket connections
#[derive(Message)]
#[rtype(result = "()")]
pub struct PaymentUpdateMessage {
    pub content: String,
}

impl Handler<PaymentUpdateMessage> for PaymentWebSocket {
    type Result = ();

    fn handle(&mut self, msg: PaymentUpdateMessage, ctx: &mut Self::Context) {
        ctx.text(msg.content);
    }
}

/// WebSocket service for managing connections and broadcasting updates
pub struct WebSocketPaymentService {
    connection_manager: Arc<RwLock<WebSocketConnectionManager>>,
}

impl WebSocketPaymentService {
    pub fn new() -> Self {
        Self {
            connection_manager: Arc::new(RwLock::new(WebSocketConnectionManager::new())),
        }
    }

    pub fn get_connection_manager(&self) -> Arc<RwLock<WebSocketConnectionManager>> {
        self.connection_manager.clone()
    }

    /// Create a new WebSocket connection handler
    pub fn create_websocket(&self) -> PaymentWebSocket {
        PaymentWebSocket::new(self.connection_manager.clone())
    }

    /// Notify all subscribers about a payment status change
    pub async fn notify_payment_update(
        &self,
        payment_id: &str,
        status: &str,
        transaction_hash: Option<&str>,
    ) {
        let manager = self.connection_manager.read().await;
        manager
            .broadcast_payment_update(payment_id, status, transaction_hash)
            .await;
    }

    /// Get service statistics
    pub async fn get_stats(&self) -> (usize, usize) {
        let manager = self.connection_manager.read().await;
        manager.get_stats()
    }
}

impl Default for WebSocketPaymentService {
    fn default() -> Self {
        Self::new()
    }
}
