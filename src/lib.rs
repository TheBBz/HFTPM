pub mod websocket;
pub mod orderbook;
pub mod arb_engine;
pub mod executor;
pub mod risk;
pub mod monitoring;
pub mod gamma_api;
pub mod utils;
pub mod market_maker;
pub mod volume_farmer;

pub use websocket::WebSocketClient;
pub use orderbook::{OrderBook, OrderBookManager};
pub use arb_engine::{ArbEngine, ArbitrageOpportunity};
pub use executor::{OrderExecutor, SignedOrder};
pub use risk::{RiskManager, Position, Inventory};
pub use monitoring::{Metrics, Monitor};
pub use gamma_api::GammaClient;
pub use utils::{Config, LatencyTracker, setup_tracing, Strategy};
pub use market_maker::MarketMaker;
pub use volume_farmer::VolumeFarmer;

use anyhow::Result;
use tracing::info;

#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(not(feature = "jemalloc"))]
#[global_allocator]
static GLOBAL: std::alloc::System = std::alloc::System;

pub async fn run() -> Result<()> {
    let config = Config::load()?;

    utils::setup_tracing(&config.monitoring.log_level, &config.monitoring.log_file);

    info!("🚀 HFTPM Trading Bot Starting");

    // Log trading mode prominently
    match config.trading.trading_mode {
        utils::TradingMode::Live => {
            tracing::warn!("⚡ LIVE TRADING MODE - REAL MONEY AT RISK");
        }
        utils::TradingMode::Simulation => {
            info!("🎮 SIMULATION MODE - No real trades will be executed");
        }
    }

    // Log strategy
    match config.trading.strategy {
        utils::Strategy::Arbitrage => {
            info!("📈 Strategy: ARBITRAGE (YES+NO < $1.00)");
        }
        utils::Strategy::MarketMaking => {
            info!("📊 Strategy: MARKET MAKING (RN1-style)");
            info!("   Spread: {} bps | Order size: ${}",
                config.trading.mm_spread_bps, config.trading.mm_order_size);
        }
        utils::Strategy::VolumeFarming => {
            info!("🗑️  Strategy: VOLUME FARMING (trash farming for airdrop)");
            info!("   Max price: ${:.2} | Daily budget: ${}",
                config.trading.vf_max_price, config.trading.vf_daily_budget);
        }
        utils::Strategy::Hybrid => {
            info!("🔄 Strategy: HYBRID (all strategies combined)");
        }
    }

    info!("📊 Bankroll: ${} USDC", config.trading.bankroll);

    let mut orderbook_manager = OrderBookManager::new(&config)?;
    let mut arb_engine = ArbEngine::new(&config);
    let mut risk_manager = RiskManager::new(&config);
    let executor = OrderExecutor::new(&config).await?;
    let mut monitor = Monitor::new(&config).await?;
    let gamma_client = GammaClient::new(&config.server.gamma_url);

    // Initialize RN1-style components
    let mut market_maker = MarketMaker::new(&config);
    let mut volume_farmer = VolumeFarmer::new(&config);

    let markets = gamma_client.fetch_markets(&config.markets).await?;
    info!("📈 Loaded {} markets from Gamma API", markets.len());

    let mut ws_client = WebSocketClient::new(&config, &markets).await?;
    ws_client.subscribe_all_markets().await?;

    // For now, run strategies inline based on config
    // The WebSocket loop handles orderbook updates and arbitrage detection
    // Market making and volume farming will be logged but run periodically
    let strategy = config.trading.strategy.clone();

    // Log which strategies are active
    match strategy {
        Strategy::Hybrid => {
            info!("🔄 Hybrid mode: Arbitrage + Market Making + Volume Farming");
        }
        Strategy::MarketMaking => {
            info!("📊 Market Making mode active");
        }
        Strategy::VolumeFarming => {
            info!("🗑️  Volume Farming mode active");
        }
        Strategy::Arbitrage => {
            info!("📈 Pure Arbitrage mode active");
        }
    }

    tokio::select! {
        // Main WebSocket loop (for orderbook updates + arbitrage detection)
        result = ws_client.run(
            &mut orderbook_manager,
            &mut arb_engine,
            &mut risk_manager,
            &executor,
            &mut monitor,
        ) => {
            info!("🛑 WebSocket loop ended: {:?}", result);
        }
        // Periodic strategy execution
        _ = run_periodic_strategies(
            &strategy,
            &mut market_maker,
            &mut volume_farmer,
        ) => {
            info!("🛑 Strategy loop ended");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("🛑 Shutting down gracefully...");

            // Print final stats
            if matches!(strategy, Strategy::MarketMaking | Strategy::Hybrid) {
                info!("📊 Final Market Making Stats: {}", market_maker.get_stats());
            }
            if matches!(strategy, Strategy::VolumeFarming | Strategy::Hybrid) {
                info!("🗑️  Final Volume Farming Stats: {}", volume_farmer.get_stats());
            }
        }
    }

    Ok(())
}

/// Run periodic strategy stats logging
/// Note: Full market making/volume farming integration requires shared orderbook state
/// For now, this logs periodic stats while the WebSocket loop handles arbitrage
async fn run_periodic_strategies(
    strategy: &Strategy,
    market_maker: &mut MarketMaker,
    volume_farmer: &mut VolumeFarmer,
) -> Result<()> {
    use std::time::Duration;

    let mut stats_interval = tokio::time::interval(Duration::from_secs(60));

    loop {
        stats_interval.tick().await;

        // Log periodic stats based on strategy
        match strategy {
            Strategy::MarketMaking | Strategy::Hybrid => {
                info!("📊 {}", market_maker.get_stats());
            }
            _ => {}
        }

        match strategy {
            Strategy::VolumeFarming | Strategy::Hybrid => {
                // Check if we should reset daily budget
                if volume_farmer.should_reset_budget() {
                    volume_farmer.reset_daily_budget();
                }
                info!("🗑️  {}", volume_farmer.get_stats());
            }
            _ => {}
        }
    }
}
