pub mod websocket;
pub mod orderbook;
pub mod arb_engine;
pub mod executor;
pub mod risk;
pub mod monitoring;
pub mod gamma_api;
pub mod utils;

pub use websocket::WebSocketClient;
pub use orderbook::{OrderBook, OrderBookManager};
pub use arb_engine::{ArbEngine, ArbitrageOpportunity};
pub use executor::{OrderExecutor, SignedOrder};
pub use risk::{RiskManager, Position, Inventory};
pub use monitoring::{Metrics, Monitor};
pub use gamma_api::GammaClient;
pub use utils::{Config, LatencyTracker, setup_tracing};

use anyhow::Result;
use tracing::info;
use std::sync::Arc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;

#[cfg(not(feature = "jemalloc"))]
use std::alloc::System;

#[cfg(not(feature = "jemalloc"))]
type Jemalloc = System;

pub async fn run() -> Result<()> {
    let config = Config::load()?;

    utils::setup_tracing(&config.monitoring.log_level, &config.monitoring.log_file);

    info!("🚀 HFTPM Ultra-Low-Latency Arbitrage Bot Starting");
    info!("📊 Bankroll: ${} USDC", config.trading.bankroll);
    info!("🎯 Min Edge: {:.2}%", config.trading.min_edge * rust_decimal::Decimal::ONE_HUNDRED);

    let orderbook_manager = OrderBookManager::new(&config)?;
    let arb_engine = ArbEngine::new(&config);
    let risk_manager = RiskManager::new(&config);
    let executor = OrderExecutor::new(&config).await?;
    let monitor = Monitor::new(&config, executor).await?;
    let gamma_client = GammaClient::new(&config.server.gamma_url);

    let markets = gamma_client.fetch_markets(&config.markets).await?;
    info!("📈 Loaded {} markets from Gamma API", markets.len());

    let ws_client = WebSocketClient::new(&config, &markets).await?;
    ws_client.subscribe_all_markets().await?;

    tokio::select! {
        result = ws_client.run(
            &mut orderbook_manager,
            &mut arb_engine,
            &mut risk_manager,
            &executor,
            &mut monitor,
        ) => {
            info!("🛑 WebSocket loop ended: {:?}", result);
        }
        _ = tokio::signal::ctrl_c() => {
            info!("🛑 Shutting down gracefully...");
        }
    }

    Ok(())
}
