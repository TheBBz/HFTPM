use crate::executor::ExecutionResult;
use crate::arb_engine::ArbitrageOpportunity;
use crate::risk::RiskManager;
use crate::utils::{Config, LatencyTracker};
use axum::{
    extract::{Query, State},
    response::Json,
    routing::get,
    Router,
};
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use std::time::Instant;
use std::collections::VecDeque;
use std::str::FromStr;
use anyhow::Result;
use tracing::{info, warn, error};
use chrono::Utc;
use rust_decimal::Decimal;

const MAX_RECENT_TRADES: usize = 100;
#[allow(dead_code)]
const MAX_METRICS_RETENTION_HOURS: u64 = 24;

#[derive(Debug, Deserialize)]
struct LimitQuery {
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub uptime_seconds: u64,
    pub arb_detections: u64,
    pub arb_executions: u64,
    pub arb_missed: u64,
    pub total_pnl: rust_decimal::Decimal,
    pub avg_latency_ms: f64,
    pub p50_latency_ns: u64,
    pub p99_latency_ns: u64,
    pub websocket_connected: bool,
    pub active_positions: usize,
    pub active_arbs: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeRecord {
    pub timestamp: i64,
    pub market_id: String,
    pub arb_type: String,
    pub position_size: rust_decimal::Decimal,
    pub expected_profit: rust_decimal::Decimal,
    pub actual_profit: rust_decimal::Decimal,
    pub execution_time_ms: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub alert_type: AlertType,
    pub message: String,
    pub timestamp: i64,
    pub severity: AlertSeverity,
}

#[derive(Debug, Clone, Serialize)]
pub enum AlertType {
    TradeExecuted,
    ArbitrageDetected,
    LatencySpike,
    Error,
    PnlDrawdown,
    RiskLimitBreached,
}

#[derive(Debug, Clone, Serialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

pub struct Monitor {
    config: Arc<Config>,
    metrics: Arc<tokio::sync::RwLock<Metrics>>,
    recent_trades: Arc<tokio::sync::RwLock<VecDeque<TradeRecord>>>,
    alerts: Arc<tokio::sync::RwLock<VecDeque<Alert>>>,
    start_time: Instant,
    latency_tracker: LatencyTracker,
    websocket_connected: Arc<tokio::sync::RwLock<bool>>,
}

impl Monitor {
    pub async fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            config: Arc::new(config.clone()),
            metrics: Arc::new(tokio::sync::RwLock::new(Self::empty_metrics())),
            recent_trades: Arc::new(tokio::sync::RwLock::new(VecDeque::with_capacity(MAX_RECENT_TRADES))),
            alerts: Arc::new(tokio::sync::RwLock::new(VecDeque::with_capacity(500))),
            start_time: Instant::now(),
            latency_tracker: LatencyTracker::new(),
            websocket_connected: Arc::new(tokio::sync::RwLock::new(false)),
        })
    }

    #[inline]
    fn empty_metrics() -> Metrics {
        Metrics {
            uptime_seconds: 0,
            arb_detections: 0,
            arb_executions: 0,
            arb_missed: 0,
            total_pnl: rust_decimal::Decimal::ZERO,
            avg_latency_ms: 0.0,
            p50_latency_ns: 0,
            p99_latency_ns: 0,
            websocket_connected: false,
            active_positions: 0,
            active_arbs: 0,
        }
    }

    #[inline]
    pub async fn record_arbitrage_detected(&self, arb_op: &ArbitrageOpportunity) {
        let mut metrics = self.metrics.write().await;
        metrics.arb_detections += 1;

        let alert = Alert {
            alert_type: AlertType::ArbitrageDetected,
            message: format!("Arbitrage detected: {} ({:.2}% edge)", arb_op.market_id, arb_op.total_edge * rust_decimal::Decimal::ONE_HUNDRED),
            timestamp: Utc::now().timestamp(),
            severity: if arb_op.total_edge > Decimal::from_str("0.04").unwrap() {
                AlertSeverity::Info
            } else {
                AlertSeverity::Warning
            },
        };

        let mut alerts = self.alerts.write().await;
        alerts.push_back(alert.clone());

        while alerts.len() > 500 {
            alerts.pop_front();
        }

        drop(alerts);

        info!(
            "🎯 Arbitrage #{} detected: {} ({:.2}% edge, ${:.2} profit)",
            metrics.arb_detections,
            arb_op.market_id,
            arb_op.total_edge * rust_decimal::Decimal::ONE_HUNDRED,
            arb_op.net_profit
        );

        if self.config.alerts.enable_telegram && arb_op.position_size >= self.config.alerts.alert_on_trade_usd.into() {
            self.send_telegram_alert(&alert).await;
        }
    }

    #[inline]
    pub async fn record_arbitrage_executed(
        &mut self,
        arb_op: &ArbitrageOpportunity,
        result: &ExecutionResult,
        execution_time: std::time::Duration,
    ) {
        let mut metrics = self.metrics.write().await;

        if result.success {
            metrics.arb_executions += 1;
            metrics.total_pnl += result.total_cost;
        } else {
            metrics.arb_missed += 1;
        }

        let latency = execution_time.as_nanos() as u64;
        self.latency_tracker.record(latency);

        let trade_record = TradeRecord {
            timestamp: Utc::now().timestamp(),
            market_id: arb_op.market_id.clone(),
            arb_type: format!("{:?}", arb_op.arb_type),
            position_size: arb_op.position_size,
            expected_profit: arb_op.net_profit,
            actual_profit: result.total_cost,
            execution_time_ms: execution_time.as_millis() as u64,
            success: result.success,
        };

        let mut recent_trades = self.recent_trades.write().await;
        recent_trades.push_back(trade_record);

        while recent_trades.len() > MAX_RECENT_TRADES {
            recent_trades.pop_front();
        }

        drop(recent_trades);

        info!(
            "✅ Arbitrage executed: {} in {:.2}ms (success: {})",
            arb_op.market_id,
            execution_time.as_secs_f64() * 1000.0,
            result.success
        );

        if self.config.alerts.enable_telegram && arb_op.position_size >= self.config.alerts.alert_on_trade_usd.into() {
            let alert = Alert {
                alert_type: AlertType::TradeExecuted,
                message: format!(
                    "Trade executed: ${:.2} profit in {:.2}ms on {}",
                    arb_op.net_profit,
                    execution_time.as_millis(),
                    arb_op.market_id
                ),
                timestamp: Utc::now().timestamp(),
                severity: AlertSeverity::Info,
            };

            self.send_telegram_alert(&alert).await;
        }
    }

    #[inline]
    pub async fn alert_latency_spike(&self, current_latency_ms: u64, threshold_ms: u64) {
        if current_latency_ms > threshold_ms {
            let alert = Alert {
                alert_type: AlertType::LatencySpike,
                message: format!("Latency spike detected: {}ms > {}ms", current_latency_ms, threshold_ms),
                timestamp: Utc::now().timestamp(),
                severity: AlertSeverity::Warning,
            };

            let mut alerts = self.alerts.write().await;
            alerts.push_back(alert.clone());

            while alerts.len() > 500 {
                alerts.pop_front();
            }

            drop(alerts);

            warn!("⚠️  Latency spike: {}ms", current_latency_ms);

            if self.config.alerts.alert_on_latency_spike {
                self.send_telegram_alert(&alert).await;
            }
        }
    }

    #[inline]
    pub async fn alert_error(&self, error_message: &str) {
        let alert = Alert {
            alert_type: AlertType::Error,
            message: format!("Error: {}", error_message),
            timestamp: Utc::now().timestamp(),
            severity: AlertSeverity::Error,
        };

        let mut alerts = self.alerts.write().await;
        alerts.push_back(alert.clone());

        while alerts.len() > 500 {
            alerts.pop_front();
        }

        drop(alerts);

        error!("❌ {}", error_message);

        if self.config.alerts.enable_telegram && self.config.alerts.alert_on_error {
            self.send_telegram_alert(&alert).await;
        }
    }

    #[inline]
    pub async fn update_metrics(&self, risk_manager: &RiskManager) {
        let mut metrics = self.metrics.write().await;

        metrics.uptime_seconds = self.start_time.elapsed().as_secs();
        metrics.avg_latency_ms = self.latency_tracker.avg_latency_ms();
        metrics.p50_latency_ns = self.latency_tracker.p50_latency_ns();
        metrics.p99_latency_ns = self.latency_tracker.p99_latency_ns();
        metrics.websocket_connected = *self.websocket_connected.read().await;

        let risk_summary = risk_manager.get_risk_summary();
        metrics.active_positions = risk_summary.active_positions;
        metrics.active_arbs = risk_summary.active_arbitrages;

        drop(metrics);
    }

    #[inline]
    async fn send_telegram_alert(&self, alert: &Alert) {
        if !self.config.alerts.enable_telegram {
            return;
        }

        let bot_token = &self.config.alerts.telegram_bot_token;
        let chat_id = &self.config.alerts.telegram_chat_id;

        if bot_token.is_empty() || chat_id.is_empty() {
            return;
        }

        let severity_icon = match alert.severity {
            AlertSeverity::Info => "ℹ️",
            AlertSeverity::Warning => "⚠️",
            AlertSeverity::Error => "❌",
            AlertSeverity::Critical => "🚨",
        };

        let message = format!(
            "{} HFTPM Alert\n\n{}",
            severity_icon,
            alert.message
        );

        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage?chat_id={}&text={}",
            bot_token,
            chat_id,
            urlencoding::encode(&message)
        );

        if let Err(e) = reqwest::get(&url).await {
            error!("Failed to send Telegram alert: {:?}", e);
        }
    }

    pub async fn start_dashboard(&self) {
        if !self.config.monitoring.enable_dashboard {
            return;
        }

        let config = self.config.clone();
        let metrics = Arc::clone(&self.metrics);
        let recent_trades = Arc::clone(&self.recent_trades);
        let alerts = Arc::clone(&self.alerts);

        let app = Router::new()
            .route("/metrics", get(Self::metrics_handler))
            .route("/trades", get(Self::trades_handler))
            .route("/alerts", get(Self::alerts_handler))
            .route("/health", get(Self::health_handler))
            .with_state((metrics, recent_trades, alerts));

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.monitoring.dashboard_port))
            .await
            .unwrap();

        info!("🌐 Dashboard started on http://0.0.0.0:{}", config.monitoring.dashboard_port);

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    async fn metrics_handler(
        State((metrics, _, _)): State<(Arc<tokio::sync::RwLock<Metrics>>, Arc<tokio::sync::RwLock<VecDeque<TradeRecord>>>, Arc<tokio::sync::RwLock<VecDeque<Alert>>>)>
    ) -> Json<Metrics> {
        Json(metrics.read().await.clone())
    }

    async fn trades_handler(
        State((_, recent_trades, _)): State<(Arc<tokio::sync::RwLock<Metrics>>, Arc<tokio::sync::RwLock<VecDeque<TradeRecord>>>, Arc<tokio::sync::RwLock<VecDeque<Alert>>>)>,
        Query(query): Query<LimitQuery>,
    ) -> Json<Vec<TradeRecord>> {
        let trades = recent_trades.read().await;
        let limit = query.limit.unwrap_or(50).min(MAX_RECENT_TRADES);

        Json(
            trades
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect()
        )
    }

    async fn alerts_handler(
        State((_, _, alerts)): State<(Arc<tokio::sync::RwLock<Metrics>>, Arc<tokio::sync::RwLock<VecDeque<TradeRecord>>>, Arc<tokio::sync::RwLock<VecDeque<Alert>>>)>,
        Query(query): Query<LimitQuery>,
    ) -> Json<Vec<Alert>> {
        let alerts_list = alerts.read().await;
        let limit = query.limit.unwrap_or(50);

        Json(
            alerts_list
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect()
        )
    }

    async fn health_handler() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "status": "healthy",
            "timestamp": Utc::now().to_rfc3339(),
        }))
    }

    #[inline]
    pub fn get_metrics(&self) -> Metrics {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.metrics.read().await.clone()
            })
        })
    }
}
