use crate::shared::{service::simple_payment_monitor::SimplePaymentMonitor, AppState};
use log;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

/// Automatic payment monitoring scheduler
pub struct PaymentScheduler {
    app_state: Arc<AppState>,
    interval_seconds: u64,
    is_running: Arc<tokio::sync::RwLock<bool>>,
}

impl PaymentScheduler {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self {
            app_state,
            interval_seconds: 30, // Check every 30 seconds
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
        }
    }

    /// Start the payment monitoring scheduler
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut is_running = self.is_running.write().await;
        if *is_running {
            log::warn!(" Payment scheduler is already running");
            return Ok(());
        }
        *is_running = true;
        drop(is_running);

        log::info!(
            " Starting payment monitoring scheduler (interval: {}s)",
            self.interval_seconds
        );

        let app_state = self.app_state.clone();
        let interval_seconds = self.interval_seconds;
        let is_running_flag = self.is_running.clone();

        tokio::spawn(async move {
            Self::monitoring_loop(app_state, interval_seconds, is_running_flag).await;
        });

        Ok(())
    }

    /// Stop the payment monitoring scheduler
    pub async fn stop(&self) {
        let mut is_running = self.is_running.write().await;
        *is_running = false;
        log::info!("⏹️ Payment monitoring scheduler stopped");
    }

    /// Check if the scheduler is currently running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    /// Main monitoring loop
    async fn monitoring_loop(
        app_state: Arc<AppState>,
        interval_seconds: u64,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) {
        log::info!("🔄 Payment monitoring loop started");

        let mut interval_timer = interval(Duration::from_secs(interval_seconds));
        interval_timer.tick().await; // Skip first immediate tick

        let mut cycle_count = 0u64;

        loop {
            // Check if we should stop
            {
                let running = is_running.read().await;
                if !*running {
                    log::info!("🛑 Payment monitoring loop stopping");
                    break;
                }
            }

            // Wait for next interval
            interval_timer.tick().await;
            cycle_count += 1;

            log::info!(
                " [SCHEDULER] Cycle #{} - Starting payment check",
                cycle_count
            );

            // Run payment monitoring
            match SimplePaymentMonitor::check_all_pending_payments(&app_state).await {
                Ok(confirmed_count) => {
                    if confirmed_count > 0 {
                        log::info!(
                            "✅ [SCHEDULER] Cycle #{} - Paid {} payments",
                            cycle_count,
                            confirmed_count
                        );
                    } else {
                        log::debug!(
                            "📭 [SCHEDULER] Cycle #{} - No pending payments found",
                            cycle_count
                        );
                    }
                }
                Err(e) => {
                    log::error!(
                        " [SCHEDULER] Cycle #{} - Monitoring error: {}",
                        cycle_count,
                        e
                    );
                }
            }
        }

        log::info!(
            "🏁 Payment monitoring loop exited after {} cycles",
            cycle_count
        );
    }
}

/// Initialize and start the global payment scheduler
pub async fn start_payment_scheduler(
    app_state: Arc<AppState>,
) -> Result<Arc<PaymentScheduler>, Box<dyn std::error::Error + Send + Sync>> {
    let scheduler = Arc::new(PaymentScheduler::new(app_state));
    scheduler.start().await?;

    log::info!("✅ Payment scheduler initialized and started");
    Ok(scheduler)
}
