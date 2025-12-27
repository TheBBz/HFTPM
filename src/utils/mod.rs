use config::{Config as ConfigLoader, Environment};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use std::path::Path;
use tracing::info;
use anyhow::{Result, Context};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TradingMode {
    Live,
    Simulation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub credentials: CredentialsConfig,
    pub trading: TradingConfig,
    pub risk: RiskConfig,
    pub markets: MarketsConfig,
    pub execution: ExecutionConfig,
    pub monitoring: MonitoringConfig,
    pub alerts: AlertsConfig,
    pub latency: LatencyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub wss_url: String,
    pub rest_url: String,
    pub gamma_url: String,
    pub polygon_rpc_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialsConfig {
    #[serde(skip_serializing)]
    pub private_key: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    #[serde(skip_serializing)]
    pub api_secret: String,
    #[serde(skip_serializing)]
    pub api_passphrase: String,
    pub funder_address: String,
    pub signature_type: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub trading_mode: TradingMode,
    pub bankroll: u64,
    pub max_arb_size: u64,
    pub min_edge: rust_decimal::Decimal,
    pub min_liquidity: u64,
    pub max_order_books: usize,
    pub tick_size: String,
    pub order_type: String,
    pub slippage_tolerance: rust_decimal::Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_exposure_per_market: u64,
    pub max_exposure_per_event: u64,
    pub max_concurrent_arbs: usize,
    pub daily_loss_limit: u64,
    pub max_gas_gwei: u64,
    pub position_timeout_seconds: u64,
    pub inventory_drift_threshold: rust_decimal::Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketsConfig {
    pub prioritize_categories: Vec<String>,
    pub blacklisted_markets: Vec<String>,
    pub min_volume_24h: u64,
    pub min_traders_24h: u64,
    pub min_order_book_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub max_latency_ms: u64,
    pub websocket_ping_interval_secs: u64,
    pub websocket_reconnect_delay_ms: u64,
    pub max_retries: usize,
    pub retry_backoff_ms: u64,
    pub http_timeout_secs: u64,
    pub connection_pool_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    pub log_level: String,
    pub enable_dashboard: bool,
    pub dashboard_port: u16,
    pub enable_tracing: bool,
    pub log_file: String,
    pub metrics_retention_hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertsConfig {
    pub enable_telegram: bool,
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
    pub alert_on_trade_usd: u64,
    pub alert_on_error: bool,
    pub alert_on_latency_spike: bool,
    pub latency_spike_threshold_ms: u64,
    pub alert_on_pnl_drawdown: bool,
    pub pnl_drawdown_threshold_usd: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyConfig {
    pub enable_cpu_pinning: bool,
    pub target_cpu_core: usize,
    pub use_jemalloc: bool,
    pub max_orderbook_updates_per_sec: usize,
    pub enable_zero_copy: bool,
}

impl Config {
    pub fn load() -> Result<Self> {
        let settings = ConfigLoader::builder()
            .add_source(
                Environment::default().prefix("HFTPM").separator("__")
            )
            .build()?;

        let config: Config = settings.try_deserialize()
            .context("Failed to deserialize config")?;

        if config.server.wss_url.is_empty() || config.server.rest_url.is_empty() {
            anyhow::bail!("Server URLs must be configured");
        }

        if config.credentials.private_key.is_empty() {
            anyhow::bail!("Private key must be set");
        }

        if config.credentials.api_key.is_empty() {
            anyhow::bail!("API key must be set");
        }

        if config.credentials.funder_address.is_empty() {
            anyhow::bail!("Funder address must be set");
        }

        info!("✅ Configuration loaded successfully");
        Ok(config)
    }
}

pub fn setup_tracing(log_level: &str, log_file: &str) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    let file_appender = tracing_appender::rolling::daily(
        Path::new(log_file).parent().unwrap_or_else(|| Path::new(".")),
        Path::new(log_file).file_name().unwrap_or_else(|| std::ffi::OsStr::new("hfptm.log")),
    );

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_target(false)
                .with_thread_ids(true)
                .with_line_number(true)
        )
        .with(tracing_subscriber::fmt::layer().json().with_writer(file_appender))
        .init();
}

#[derive(Debug, Clone, Default)]
pub struct LatencyTracker {
    detection_count: u64,
    total_latency_ns: u64,
    min_latency_ns: Option<u64>,
    max_latency_ns: Option<u64>,
    last_update: Option<Instant>,
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, latency_ns: u64) {
        self.detection_count += 1;
        self.total_latency_ns += latency_ns;

        self.min_latency_ns = Some(
            self.min_latency_ns
                .map(|min| min.min(latency_ns))
                .unwrap_or(latency_ns)
        );

        self.max_latency_ns = Some(
            self.max_latency_ns
                .map(|max| max.max(latency_ns))
                .unwrap_or(latency_ns)
        );

        self.last_update = Some(Instant::now());
    }

    pub fn avg_latency_ns(&self) -> u64 {
        if self.detection_count == 0 {
            return 0;
        }
        self.total_latency_ns / self.detection_count
    }

    pub fn avg_latency_ms(&self) -> f64 {
        self.avg_latency_ns() as f64 / 1_000_000.0
    }

    pub fn p50_latency_ns(&self) -> u64 {
        self.avg_latency_ns()
    }

    pub fn p99_latency_ns(&self) -> u64 {
        self.max_latency_ns.unwrap_or(0)
    }

    pub fn count(&self) -> u64 {
        self.detection_count
    }
}

pub struct ScopedTimer<'a> {
    name: &'a str,
    tracker: Option<&'a mut LatencyTracker>,
    start: Instant,
}

impl<'a> ScopedTimer<'a> {
    pub fn new(name: &'a str, tracker: Option<&'a mut LatencyTracker>) -> Self {
        Self {
            name,
            tracker,
            start: Instant::now(),
        }
    }
}

impl<'a> Drop for ScopedTimer<'a> {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_nanos() as u64;

        if let Some(tracker) = &mut self.tracker {
            tracker.record(elapsed);
        }

        tracing::debug!(
            "⏱️  {} took {}μs ({}ns)",
            self.name,
            elapsed / 1000,
            elapsed
        );
    }
}
