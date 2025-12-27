//! Market Making Module - RN1-style limit order placement for spread + rewards
//!
//! This module implements the core market-making strategy:
//! 1. Place limit orders at midpoint +/- spread to earn the spread
//! 2. Qualify for Polymarket's liquidity rewards (orders near midpoint)
//! 3. Track open orders and manage inventory
//! 4. Use synthetic hedging instead of selling (avoid taker fees)

use crate::utils::Config;
use crate::orderbook::OrderBookManager;
use crate::gamma_api::Market;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::Result;
use tracing::{info, debug};
use serde::{Serialize, Deserialize};

/// Represents an open limit order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenOrder {
    pub order_id: String,
    pub market_id: String,
    pub asset_id: String,
    pub side: OrderSide,
    pub price: Decimal,
    pub size: Decimal,
    pub created_at: i64,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderSide {
    Bid,  // Buy order
    Ask,  // Sell order
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderStatus {
    Open,
    PartialFill,
    Filled,
    Cancelled,
}

/// Market making opportunity
#[derive(Debug, Clone)]
pub struct MMOpportunity {
    pub market_id: String,
    pub asset_id: String,
    pub midpoint: Decimal,
    pub bid_price: Decimal,
    pub ask_price: Decimal,
    pub size: Decimal,
    pub spread_bps: Decimal,
    pub estimated_reward: Decimal,
}

/// Statistics for a market we're making
#[derive(Debug, Clone, Default)]
pub struct MarketStats {
    pub orders_placed: u64,
    pub orders_filled: u64,
    pub volume_provided: Decimal,
    pub spread_earned: Decimal,
    pub estimated_rewards: Decimal,
    pub last_update: Option<Instant>,
}

/// The Market Maker engine
pub struct MarketMaker {
    config: Arc<Config>,
    open_orders: HashMap<String, OpenOrder>,  // order_id -> order
    market_stats: HashMap<String, MarketStats>,
    total_volume: Decimal,
    total_rewards_estimate: Decimal,
    last_refresh: Instant,
    simulated_balance: Decimal,
    initial_balance: Decimal,
}

impl MarketMaker {
    pub fn new(config: &Config) -> Self {
        let initial_balance = Decimal::from(config.trading.bankroll);

        info!("📊 Market Maker initialized");
        info!("   Spread: {} bps", config.trading.mm_spread_bps);
        info!("   Order size: ${}", config.trading.mm_order_size);
        info!("   Max orders/market: {}", config.trading.mm_max_orders_per_market);

        Self {
            config: Arc::new(config.clone()),
            open_orders: HashMap::new(),
            market_stats: HashMap::new(),
            total_volume: Decimal::ZERO,
            total_rewards_estimate: Decimal::ZERO,
            last_refresh: Instant::now(),
            simulated_balance: initial_balance,
            initial_balance,
        }
    }

    /// Find market making opportunities from current orderbook state
    pub fn find_opportunities(
        &self,
        orderbook_manager: &OrderBookManager,
        markets: &[Market],
    ) -> Vec<MMOpportunity> {
        let mut opportunities = Vec::new();
        let spread_decimal = Decimal::from(self.config.trading.mm_spread_bps) / dec!(10000);
        let order_size = Decimal::from(self.config.trading.mm_order_size);

        for market in markets {
            // Get best bid and ask for each asset in the market
            for asset_id in &market.assets_ids {
                if let Some((best_bid, best_ask)) = self.get_best_prices(orderbook_manager, &market.market, asset_id) {
                    // Calculate midpoint
                    let midpoint = (best_bid + best_ask) / dec!(2);

                    // Our bid and ask prices (inside the current spread if possible)
                    let half_spread = midpoint * spread_decimal / dec!(2);
                    let our_bid = midpoint - half_spread;
                    let our_ask = midpoint + half_spread;

                    // Only make markets where we can place competitive orders
                    let current_spread = best_ask - best_bid;
                    let current_spread_bps = (current_spread / midpoint) * dec!(10000);

                    // Skip if spread is too tight (we can't compete)
                    if current_spread_bps < Decimal::from(self.config.trading.mm_spread_bps / 2) {
                        debug!("Spread too tight for {}: {} bps", asset_id, current_spread_bps);
                        continue;
                    }

                    // Estimate daily reward (rough approximation)
                    // Polymarket rewards ~$1 per $846 liquidity provided
                    let estimated_daily_reward = order_size * dec!(2) / dec!(846);

                    opportunities.push(MMOpportunity {
                        market_id: market.market.clone(),
                        asset_id: asset_id.clone(),
                        midpoint,
                        bid_price: our_bid,
                        ask_price: our_ask,
                        size: order_size,
                        spread_bps: Decimal::from(self.config.trading.mm_spread_bps),
                        estimated_reward: estimated_daily_reward,
                    });
                }
            }
        }

        // Sort by estimated reward (highest first)
        opportunities.sort_by(|a, b| b.estimated_reward.cmp(&a.estimated_reward));

        opportunities
    }

    /// Get best bid and ask prices for an asset
    fn get_best_prices(
        &self,
        orderbook_manager: &OrderBookManager,
        market_id: &str,
        asset_id: &str,
    ) -> Option<(Decimal, Decimal)> {
        let book = orderbook_manager.get_book(market_id, asset_id)?;

        let best_bid = book.bids.iter()
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(price, _)| *price)?;

        let best_ask = book.asks.iter()
            .min_by(|a, b| a.0.cmp(&b.0))
            .map(|(price, _)| *price)?;

        Some((best_bid, best_ask))
    }

    /// Simulate placing market making orders (simulation mode)
    pub async fn simulate_mm_orders(
        &mut self,
        opportunities: &[MMOpportunity],
    ) -> Result<Vec<SimulatedMMResult>> {
        let mut results = Vec::new();
        let max_markets = self.config.trading.max_order_books.min(opportunities.len());

        for opp in opportunities.iter().take(max_markets) {
            // Check if we already have orders in this market
            let existing_orders = self.open_orders.values()
                .filter(|o| o.asset_id == opp.asset_id)
                .count();

            if existing_orders >= self.config.trading.mm_max_orders_per_market {
                continue;
            }

            // Simulate placing bid order
            let bid_order = self.simulate_order(
                &opp.market_id,
                &opp.asset_id,
                OrderSide::Bid,
                opp.bid_price,
                opp.size,
            ).await?;

            // Simulate placing ask order (we're selling to close, so this is like a synthetic hedge)
            let ask_order = self.simulate_order(
                &opp.market_id,
                &opp.asset_id,
                OrderSide::Ask,
                opp.ask_price,
                opp.size,
            ).await?;

            results.push(SimulatedMMResult {
                market_id: opp.market_id.clone(),
                asset_id: opp.asset_id.clone(),
                bid_order_id: bid_order.order_id.clone(),
                ask_order_id: ask_order.order_id.clone(),
                bid_price: opp.bid_price,
                ask_price: opp.ask_price,
                size: opp.size,
                estimated_reward: opp.estimated_reward,
            });

            // Update stats
            let stats = self.market_stats.entry(opp.market_id.clone()).or_default();
            stats.orders_placed += 2;
            stats.volume_provided += opp.size * dec!(2);
            stats.estimated_rewards += opp.estimated_reward;
            stats.last_update = Some(Instant::now());

            self.total_volume += opp.size * dec!(2);
            self.total_rewards_estimate += opp.estimated_reward;
        }

        Ok(results)
    }

    /// Simulate a single order placement
    async fn simulate_order(
        &mut self,
        market_id: &str,
        asset_id: &str,
        side: OrderSide,
        price: Decimal,
        size: Decimal,
    ) -> Result<OpenOrder> {
        let order_id = format!("SIM_MM_{}", uuid::Uuid::new_v4());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let order = OpenOrder {
            order_id: order_id.clone(),
            market_id: market_id.to_string(),
            asset_id: asset_id.to_string(),
            side,
            price,
            size,
            created_at: timestamp,
            status: OrderStatus::Open,
        };

        self.open_orders.insert(order_id, order.clone());

        debug!(
            "🎮 [SIM] Placed {:?} order: {} @ ${:.4} x {}",
            side, asset_id, price, size
        );

        Ok(order)
    }

    /// Simulate order fills based on price movement
    pub async fn simulate_fills(
        &mut self,
        orderbook_manager: &OrderBookManager,
    ) -> Vec<SimulatedFill> {
        let mut fills = Vec::new();
        let mut orders_to_fill = Vec::new();

        for (order_id, order) in &self.open_orders {
            if order.status != OrderStatus::Open {
                continue;
            }

            // Get current best prices
            if let Some((best_bid, best_ask)) = self.get_best_prices(
                orderbook_manager,
                &order.market_id,
                &order.asset_id,
            ) {
                let should_fill = match order.side {
                    // Bid fills when market ask drops to our bid
                    OrderSide::Bid => best_ask <= order.price,
                    // Ask fills when market bid rises to our ask
                    OrderSide::Ask => best_bid >= order.price,
                };

                if should_fill {
                    orders_to_fill.push(order_id.clone());
                }
            }
        }

        for order_id in orders_to_fill {
            if let Some(order) = self.open_orders.get_mut(&order_id) {
                order.status = OrderStatus::Filled;

                let cost = order.price * order.size;

                // Update simulated balance
                match order.side {
                    OrderSide::Bid => {
                        // We bought - deduct cost
                        self.simulated_balance -= cost;
                    }
                    OrderSide::Ask => {
                        // We sold - add proceeds
                        self.simulated_balance += cost;
                    }
                }

                // Update stats
                if let Some(stats) = self.market_stats.get_mut(&order.market_id) {
                    stats.orders_filled += 1;
                }

                fills.push(SimulatedFill {
                    order_id: order_id.clone(),
                    market_id: order.market_id.clone(),
                    asset_id: order.asset_id.clone(),
                    side: order.side,
                    price: order.price,
                    size: order.size,
                });

                info!(
                    "🎮 [SIM] FILLED {:?}: {} @ ${:.4} | Balance: ${:.2}",
                    order.side, order.asset_id, order.price, self.simulated_balance
                );
            }
        }

        fills
    }

    /// Check if orders need refreshing
    pub fn needs_refresh(&self) -> bool {
        self.last_refresh.elapsed() > Duration::from_secs(self.config.trading.mm_order_refresh_secs)
    }

    /// Cancel and refresh stale orders
    pub async fn refresh_orders(&mut self) {
        let stale_threshold = Duration::from_secs(self.config.trading.mm_order_refresh_secs);
        let now = Instant::now();

        // Cancel stale open orders
        let stale_orders: Vec<_> = self.open_orders.iter()
            .filter(|(_, order)| {
                order.status == OrderStatus::Open &&
                now.duration_since(Instant::now()) > stale_threshold
            })
            .map(|(id, _)| id.clone())
            .collect();

        for order_id in stale_orders {
            if let Some(order) = self.open_orders.get_mut(&order_id) {
                order.status = OrderStatus::Cancelled;
                debug!("🎮 [SIM] Cancelled stale order: {}", order_id);
            }
        }

        self.last_refresh = Instant::now();
    }

    /// Get current statistics
    pub fn get_stats(&self) -> MMStats {
        let pnl = self.simulated_balance - self.initial_balance;

        MMStats {
            total_orders_placed: self.open_orders.len() as u64,
            open_orders: self.open_orders.values().filter(|o| o.status == OrderStatus::Open).count() as u64,
            filled_orders: self.open_orders.values().filter(|o| o.status == OrderStatus::Filled).count() as u64,
            total_volume: self.total_volume,
            estimated_rewards: self.total_rewards_estimate,
            simulated_balance: self.simulated_balance,
            pnl,
            markets_active: self.market_stats.len() as u64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SimulatedMMResult {
    pub market_id: String,
    pub asset_id: String,
    pub bid_order_id: String,
    pub ask_order_id: String,
    pub bid_price: Decimal,
    pub ask_price: Decimal,
    pub size: Decimal,
    pub estimated_reward: Decimal,
}

#[derive(Debug, Clone)]
pub struct SimulatedFill {
    pub order_id: String,
    pub market_id: String,
    pub asset_id: String,
    pub side: OrderSide,
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Debug, Clone)]
pub struct MMStats {
    pub total_orders_placed: u64,
    pub open_orders: u64,
    pub filled_orders: u64,
    pub total_volume: Decimal,
    pub estimated_rewards: Decimal,
    pub simulated_balance: Decimal,
    pub pnl: Decimal,
    pub markets_active: u64,
}

impl std::fmt::Display for MMStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MM Stats: {} orders ({} open, {} filled) | Vol: ${:.0} | Est. Rewards: ${:.2} | Balance: ${:.2} (P&L: ${:.2})",
            self.total_orders_placed,
            self.open_orders,
            self.filled_orders,
            self.total_volume,
            self.estimated_rewards,
            self.simulated_balance,
            self.pnl
        )
    }
}
