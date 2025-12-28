pub mod arb_engine;
pub mod executor;
pub mod gamma_api;
pub mod market_maker;
pub mod monitoring;
pub mod orderbook;
pub mod parallel_scanner;
pub mod risk;
pub mod utils;
pub mod volume_farmer;
pub mod websocket;

pub use arb_engine::{ArbEngine, ArbitrageOpportunity};
pub use executor::{OrderExecutor, SignedOrder};
pub use gamma_api::GammaClient;
pub use market_maker::MarketMaker;
pub use monitoring::{Metrics, Monitor};
pub use orderbook::{OrderBook, OrderBookManager};
pub use parallel_scanner::ParallelScanner;
pub use risk::{Inventory, Position, RiskManager};
pub use utils::{setup_tracing, Config, LatencyTracker, Strategy};
pub use volume_farmer::VolumeFarmer;
pub use websocket::WebSocketClient;

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
            info!(
                "   Spread: {} bps | Order size: ${}",
                config.trading.mm_spread_bps, config.trading.mm_order_size
            );
        }
        utils::Strategy::VolumeFarming => {
            info!("🗑️  Strategy: VOLUME FARMING (trash farming for airdrop)");
            info!(
                "   Max price: ${:.2} | Daily budget: ${}",
                config.trading.vf_max_price, config.trading.vf_daily_budget
            );
        }
        utils::Strategy::Hybrid => {
            info!("🔄 Strategy: HYBRID (all strategies combined)");
        }
    }

    info!("📊 Bankroll: ${} USDC", config.trading.bankroll);

    // Use Arc for thread-safe sharing of OrderBookManager
    let orderbook_manager = std::sync::Arc::new(OrderBookManager::new(&config)?);
    let orderbook_manager_scanner = orderbook_manager.clone();

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

    // Initialize parallel scanner for 16-core optimization
    let parallel_scanner = std::sync::Arc::new(ParallelScanner::new(&config, markets.clone()));
    let parallel_scanner_loop = parallel_scanner.clone();

    // Build correlation graph (uses 64GB RAM for caching relationships)
    info!("🔗 Building cross-market correlation graph (utilizing 64GB RAM)...");
    parallel_scanner.build_correlation_graph().await;
    info!(
        "✅ Correlation graph built: {} market pairs",
        parallel_scanner.num_correlations().await
    );

    info!("🔌 Creating WebSocket client...");
    let mut ws_client = WebSocketClient::new(&config, &markets).await?;
    info!("📡 Subscribing to {} markets...", markets.len());
    ws_client.subscribe_all_markets().await?;
    info!("✅ Subscribed to all markets, starting main loop...");

    // Get strategy for the loops
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
            &orderbook_manager,
            &mut arb_engine,
            &mut risk_manager,
            &executor,
            &mut monitor,
        ) => {
            info!("🛑 WebSocket loop ended: {:?}", result);
        }
        // Periodic strategy execution + parallel scanning
        _ = run_periodic_strategies(
            &strategy,
            &mut market_maker,
            &mut volume_farmer,
            &parallel_scanner_loop,
            &orderbook_manager_scanner,
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
            info!("🔬 Final Scanner Stats: {}", parallel_scanner.get_stats().await);
        }
    }

    Ok(())
}

/// Run periodic strategy execution + parallel market scanning
/// Integrates with the 16-core parallel scanner for cross-market and multi-outcome detection
async fn run_periodic_strategies(
    strategy: &Strategy,
    market_maker: &mut MarketMaker,
    volume_farmer: &mut VolumeFarmer,
    parallel_scanner: &std::sync::Arc<ParallelScanner>,
    orderbook_manager: &std::sync::Arc<OrderBookManager>,
) -> Result<()> {
    use std::time::Duration;

    // Stats logging every 60 seconds
    let mut stats_interval = tokio::time::interval(Duration::from_secs(60));
    // Parallel scanning every 5 seconds (fast enough to catch opportunities)
    let mut scan_interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = stats_interval.tick() => {
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

                // Log scanner stats
                let loaded_markets = orderbook_manager.get_all_market_ids().len();
                info!("🔬 {} | 📚 {} books loaded", parallel_scanner.get_stats().await, loaded_markets);
            }
            _ = scan_interval.tick() => {
                // Run parallel scans for arbitrage opportunities
                match strategy {
                    Strategy::Arbitrage | Strategy::Hybrid => {
                        // Scan for multi-outcome arbitrage (sum of all outcomes < $1)
                        let multi_opps = parallel_scanner.scan_multi_outcome_parallel(orderbook_manager).await;
                        if !multi_opps.is_empty() {
                            for opp in multi_opps.iter().take(3) {
                                info!(
                                    "💰 MULTI-OUTCOME: {} | {} outcomes @ ${:.4} | Edge: {:.2}% | Est. Profit: ${:.2}",
                                    opp.market_question.chars().take(40).collect::<String>(),
                                    opp.num_outcomes,
                                    opp.total_price,
                                    opp.edge * rust_decimal::Decimal::from(100),
                                    opp.expected_profit
                                );
                            }
                        }

                        // Scan for cross-market arbitrage (logical inconsistencies)
                        let cross_opps = parallel_scanner.scan_cross_market_parallel(orderbook_manager).await;
                        if !cross_opps.is_empty() {
                            for opp in cross_opps.iter().take(3) {
                                info!(
                                    "🔗 CROSS-MARKET: {:?} | A:{} B:{} | Edge: {:.2}% | Est. Profit: ${:.2}",
                                    opp.arb_type,
                                    opp.market_a_question,  // Now contains "YES@price" or "NO@price"
                                    opp.market_b_question,
                                    opp.edge * rust_decimal::Decimal::from(100),
                                    opp.expected_profit
                                );
                            }

                            // TODO: Execute cross-market trades
                            // For now, just log that we found opportunities
                            info!("📊 Found {} cross-market opportunities (execution not yet implemented)", cross_opps.len());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
