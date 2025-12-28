//! Standalone Short-Window Arbitrage Simulator
//!
//! A lightweight binary focused exclusively on simulating short-window (15m/1h)
//! Sum-<$1 arbitrage opportunities. Uses REST API for order book data.
//!
//! Usage:
//!   cargo run --release --bin sw_arb_sim
//!   # Or after building:
//!   ./target/release/sw_arb_sim
//!
//! Output:
//!   - Real-time opportunity detection
//!   - Simulated trade execution
//!   - P&L tracking with periodic stats
//!   - JSON export of all trades on exit

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use hfptm::{
    arb_engine::ShortWindowArbTracker,
    gamma_api::{GammaClient, Market},
    utils::Config,
};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Order book response from CLOB API
#[derive(Debug, Deserialize)]
struct BookResponse {
    market: Option<String>,
    asset_id: Option<String>,
    bids: Option<Vec<PriceLevel>>,
    asks: Option<Vec<PriceLevel>>,
}

#[derive(Debug, Deserialize)]
struct PriceLevel {
    price: String,
    size: String,
}

/// Simplified order book data
#[derive(Debug, Clone)]
struct SimpleOrderBook {
    best_ask_price: Decimal,
    best_ask_size: Decimal,
}

/// Short-window arb opportunity (simplified)
#[derive(Debug, Clone)]
struct SwArbOpportunity {
    market_id: String,
    market_question: String,
    minutes_to_expiry: i64,
    yes_price: Decimal,
    no_price: Decimal,
    sum_prices: Decimal,
    net_edge: Decimal,
    position_size: Decimal,
    expected_profit: Decimal,
    yes_asset_id: String,
    no_asset_id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Banner
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  🎯 SHORT-WINDOW ARBITRAGE SIMULATOR (REST API Mode)          ║");
    println!("║  Sum-<$1 Strategy on 15m/1h Up/Down Markets                   ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");
    println!();

    // Load config
    let config = Config::load()?;

    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,hfptm=debug".into()),
        )
        .with_target(false)
        .with_thread_ids(false)
        .compact()
        .init();

    info!("📁 Config loaded from config/config.toml");
    info!("💰 Starting balance: ${} USDC", config.trading.bankroll);
    info!(
        "📊 Short-window settings: min_edge={:.2}%, max_size=${}",
        config.trading.short_window_min_edge * Decimal::from(100),
        config.trading.short_window_max_size
    );
    info!("🎮 SIMULATION MODE - All trades are simulated, no real execution");
    println!();

    // HTTP client for CLOB API
    let http_client = Arc::new(
        Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?,
    );

    // Initialize components  
    let gamma_client = GammaClient::new(&config.server.gamma_url);
    let mut tracker = ShortWindowArbTracker::new(Decimal::from(config.trading.bankroll));

    // Session tracking
    let session_start = Utc::now();
    let mut opportunities_detected: u64 = 0;
    let mut markets_scanned: u64 = 0;
    let mut total_edge_sum = Decimal::ZERO;
    let mut edge_count: u64 = 0;
    
    // Thresholds from config
    let min_edge = config.trading.short_window_min_edge;
    let max_size = Decimal::from(config.trading.short_window_max_size);
    let min_liquidity = Decimal::from(config.trading.min_liquidity);
    let fee_rate = Decimal::from_str("0.02")?; // 2% Polymarket fee

    info!("🚀 Starting simulation loop (Ctrl+C to stop)...");
    println!();

    // Main polling loop
    let mut scan_interval = tokio::time::interval(Duration::from_secs(10)); // Slower to respect rate limits
    let mut stats_interval = tokio::time::interval(Duration::from_secs(30));
    let mut market_refresh_interval = tokio::time::interval(Duration::from_secs(120));

    // Initial market fetch
    let mut short_window_markets = gamma_client.fetch_short_window_markets(&config.markets).await?;
    info!("📈 Loaded {} short-window markets", short_window_markets.len());

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("🛑 Shutdown signal received...");
                break;
            }

            _ = market_refresh_interval.tick() => {
                // Refresh short-window markets every 2 minutes
                match gamma_client.fetch_short_window_markets(&config.markets).await {
                    Ok(new_markets) => {
                        if new_markets.len() != short_window_markets.len() {
                            info!("🔄 Market list refreshed: {} -> {} markets", 
                                short_window_markets.len(), new_markets.len());
                        }
                        short_window_markets = new_markets;
                    }
                    Err(e) => {
                        warn!("⚠️  Failed to refresh markets: {}", e);
                    }
                }
            }

            _ = scan_interval.tick() => {
                // Auto-resolve expired trades
                tracker.auto_resolve_expired();

                // Scan each short-window market for arb opportunities
                for market in &short_window_markets {
                    markets_scanned += 1;
                    
                    // Analyze if still within short window
                    let short_info = market.analyze_short_window(&config.markets);
                    if !short_info.is_short_window {
                        continue;
                    }
                    
                    let minutes_to_expiry = short_info.minutes_to_expiry.unwrap_or(0);
                    if minutes_to_expiry < config.markets.min_minutes_to_expiry as i64 {
                        continue;
                    }
                    
                    // Need exactly 2 assets (YES/NO)
                    if market.assets_ids.len() != 2 {
                        continue;
                    }
                    
                    // Fetch order books for YES and NO tokens
                    let yes_asset_id = &market.assets_ids[0];
                    let no_asset_id = &market.assets_ids[1];
                    
                    let (yes_book, no_book) = match tokio::try_join!(
                        fetch_order_book(&http_client, &config.server.rest_url, yes_asset_id),
                        fetch_order_book(&http_client, &config.server.rest_url, no_asset_id)
                    ) {
                        Ok((Some(y), Some(n))) => (y, n),
                        Ok(_) => {
                            debug!("Missing order book data for {}", market.question);
                            continue;
                        }
                        Err(e) => {
                            debug!("Error fetching order book: {}", e);
                            continue;
                        }
                    };
                    
                    // Check for Sum-<$1 arb
                    let sum_prices = yes_book.best_ask_price + no_book.best_ask_price;
                    
                    if sum_prices >= Decimal::ONE {
                        debug!("No arb: {} sum={:.4}", market.question, sum_prices);
                        continue;
                    }
                    
                    let raw_edge = Decimal::ONE - sum_prices;
                    let net_edge = raw_edge - fee_rate;
                    
                    if net_edge < min_edge {
                        debug!("Edge too small: {} {:.2}% < {:.2}%", 
                            market.question, 
                            net_edge * Decimal::from(100),
                            min_edge * Decimal::from(100));
                        continue;
                    }
                    
                    // Check liquidity
                    let liquidity = yes_book.best_ask_size.min(no_book.best_ask_size);
                    if liquidity < min_liquidity {
                        debug!("Liquidity too low: {} ${}", market.question, liquidity);
                        continue;
                    }
                    
                    // Calculate position size
                    let position_size = liquidity.min(max_size);
                    let expected_profit = position_size * net_edge;
                    
                    let opp = SwArbOpportunity {
                        market_id: market.market.clone(),
                        market_question: market.question.clone(),
                        minutes_to_expiry,
                        yes_price: yes_book.best_ask_price,
                        no_price: no_book.best_ask_price,
                        sum_prices,
                        net_edge,
                        position_size,
                        expected_profit,
                        yes_asset_id: yes_asset_id.clone(),
                        no_asset_id: no_asset_id.clone(),
                    };
                    
                    opportunities_detected += 1;
                    total_edge_sum += net_edge;
                    edge_count += 1;

                    // Convert to lib format for tracker
                    let lib_opp = hfptm::arb_engine::ShortWindowArbOpportunity {
                        market_id: opp.market_id.clone(),
                        market_question: opp.market_question.clone(),
                        minutes_to_expiry: opp.minutes_to_expiry,
                        yes_price: opp.yes_price,
                        no_price: opp.no_price,
                        sum_prices: opp.sum_prices,
                        raw_edge: raw_edge,
                        net_edge: opp.net_edge,
                        position_size: opp.position_size,
                        expected_profit: opp.expected_profit,
                        yes_asset_id: opp.yes_asset_id.clone(),
                        no_asset_id: opp.no_asset_id.clone(),
                        min_liquidity: liquidity,
                        detected_at: chrono::Utc::now().timestamp_millis(),
                        annualized_return: Decimal::ZERO, // Not needed for sim
                    };
                    
                    // Simulate entry
                    let _trade = tracker.simulate_entry(&lib_opp);

                    // Pretty print the opportunity
                    println!("┌─────────────────────────────────────────────────────────────────┐");
                    println!("│ 🎯 OPPORTUNITY #{:<5}                                           │", opportunities_detected);
                    println!("├─────────────────────────────────────────────────────────────────┤");
                    println!("│ Market: {:<54} │", truncate_str(&opp.market_question, 54));
                    println!("│ YES: ${:<6.4}  NO: ${:<6.4}  SUM: ${:<6.4}                     │",
                        opp.yes_price, opp.no_price, opp.sum_prices);
                    println!("│ Edge: {:<5.2}%  Position: ${:<6.2}  Profit: ${:<6.2}               │",
                        opp.net_edge * Decimal::from(100),
                        opp.position_size,
                        opp.expected_profit);
                    println!("│ Expires in: {} minutes                                          │", opp.minutes_to_expiry);
                    println!("└─────────────────────────────────────────────────────────────────┘");
                    println!();
                }
            }

            _ = stats_interval.tick() => {
                let stats = tracker.get_stats();
                let avg_edge = if edge_count > 0 {
                    total_edge_sum / Decimal::from(edge_count)
                } else {
                    Decimal::ZERO
                };

                println!();
                println!("╔═══════════════════════════════════════════════════════════════╗");
                println!("║  📊 SIMULATION STATS                                          ║");
                println!("╠═══════════════════════════════════════════════════════════════╣");
                println!("║  Trades: {} entered, {} open, {} resolved                     ",
                    stats.trades_entered, stats.trades_open, stats.trades_won + stats.trades_lost);
                println!("║  Win Rate: {:.1}%                                              ",
                    stats.win_rate * Decimal::from(100));
                println!("║  Total P&L: ${:<10.2}  ROI: {:<6.2}%                        ",
                    stats.total_pnl, stats.roi);
                println!("║  Balance: ${:<10.2} (started: ${})                   ",
                    stats.simulated_balance, config.trading.bankroll);
                println!("║  Avg Edge: {:.2}%  Markets: {} SW, {} scanned              ",
                    avg_edge * Decimal::from(100), short_window_markets.len(), markets_scanned);
                println!("╚═══════════════════════════════════════════════════════════════╝");
                println!();
            }
        }
    }

    // Final summary
    let stats = tracker.get_stats();
    let session_end = Utc::now();
    let duration = session_end.signed_duration_since(session_start);

    println!();
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  🏁 SIMULATION COMPLETE                                       ║");
    println!("╠═══════════════════════════════════════════════════════════════╣");
    println!("║  Duration: {} minutes                                         ", duration.num_minutes());
    println!("║  Total Opportunities: {}                                      ", opportunities_detected);
    println!("║  Trades Simulated: {}                                         ", stats.trades_entered);
    println!("║  Final Balance: ${:<10.2}                                   ", stats.simulated_balance);
    println!("║  Total P&L: ${:<10.2}                                       ", stats.total_pnl);
    println!("║  ROI: {:.2}%                                                  ", stats.roi);
    println!("║  Win Rate: {:.1}%                                             ", stats.win_rate * Decimal::from(100));
    println!("╚═══════════════════════════════════════════════════════════════╝");

    // Export trades to JSON
    let export_path = format!("logs/sw_arb_sim_{}.json", session_start.format("%Y%m%d_%H%M%S"));
    if let Err(e) = export_trades(&tracker, &export_path, &session_start, &session_end, &stats) {
        warn!("Failed to export trades: {}", e);
    } else {
        info!("📁 Trades exported to: {}", export_path);
    }

    Ok(())
}

/// Fetch order book from CLOB REST API
async fn fetch_order_book(
    client: &Client,
    base_url: &str,
    token_id: &str,
) -> Result<Option<SimpleOrderBook>> {
    let url = format!("{}/book?token_id={}", base_url, token_id);
    
    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to fetch order book")?;
    
    if !response.status().is_success() {
        return Ok(None);
    }
    
    let book: BookResponse = response.json().await.context("Failed to parse order book")?;
    
    // Get best ask (lowest ask price)
    let best_ask = book.asks
        .and_then(|asks| asks.into_iter().next())
        .and_then(|level| {
            let price = Decimal::from_str(&level.price).ok()?;
            let size = Decimal::from_str(&level.size).ok()?;
            Some(SimpleOrderBook {
                best_ask_price: price,
                best_ask_size: size,
            })
        });
    
    Ok(best_ask)
}

/// Truncate string for display
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Export trades to JSON file
fn export_trades(
    tracker: &ShortWindowArbTracker,
    path: &str,
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
    stats: &hfptm::arb_engine::ShortWindowArbStats,
) -> Result<()> {
    // Ensure logs directory exists
    fs::create_dir_all("logs")?;

    let session = serde_json::json!({
        "session": {
            "started_at": start.to_rfc3339(),
            "ended_at": end.to_rfc3339(),
            "duration_minutes": end.signed_duration_since(*start).num_minutes(),
        },
        "stats": {
            "trades_entered": stats.trades_entered,
            "trades_won": stats.trades_won,
            "trades_lost": stats.trades_lost,
            "win_rate_percent": stats.win_rate * Decimal::from(100),
            "total_pnl": stats.total_pnl,
            "roi_percent": stats.roi,
            "simulated_balance": stats.simulated_balance,
            "total_capital_deployed": stats.total_capital_deployed,
        },
        "trades": tracker.get_trades(),
    });

    fs::write(path, serde_json::to_string_pretty(&session)?)?;
    Ok(())
}
