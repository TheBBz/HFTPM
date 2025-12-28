use crate::gamma_api::Market;
use crate::orderbook::OrderBookManager;
use crate::risk::RiskManager;
use crate::utils::Config;
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    pub market_id: String,
    pub arb_type: ArbType,
    pub edges: Vec<ArbEdge>,
    pub total_edge: Decimal,
    pub min_liquidity: Decimal,
    pub position_size: Decimal,
    pub expected_profit_usd: Decimal,
    pub fee_cost: Decimal,
    pub net_profit: Decimal,
    pub timestamp: i64,
    pub detection_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArbType {
    Binary,
    MultiOutcome,
}

impl std::fmt::Display for ArbType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArbType::Binary => write!(f, "Binary"),
            ArbType::MultiOutcome => write!(f, "MultiOutcome"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbEdge {
    pub asset_id: String,
    pub outcome: String,
    pub price: Decimal,
    pub size: Decimal,
    pub expected_cost: Decimal,
}

impl std::fmt::Display for ArbitrageOpportunity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [{}]: {:.2}% edge, ${:.2} profit, ${:.2} position",
            self.arb_type,
            self.market_id,
            self.total_edge * Decimal::ONE_HUNDRED,
            self.net_profit,
            self.position_size
        )
    }
}

// =============================================================================
// Short-Window Arbitrage (gabagool-style 15m Sum-<$1 arb)
// =============================================================================
//
// Strategy: When YES + NO < $1 on short-duration markets (15-30 min), buy both
// outcomes. One will resolve to $1, guaranteeing profit.
//
// Why 15m markets are ideal:
// - Fast resolution = fast capital turnover (many cycles per day)
// - More volatile = more mispricing opportunities
// - gabagool made millions doing this on crypto price markets
//
// Key differences from standard arb:
// - Lower min_edge (0.8% vs 1.2%) because fast turnover compensates
// - Track minutes_to_expiry for position sizing
// - Priority scanning (every 500ms vs 15s)
// =============================================================================

/// Short-window arbitrage opportunity (gabagool-style)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortWindowArbOpportunity {
    pub market_id: String,
    pub market_question: String,
    /// Minutes until market resolution
    pub minutes_to_expiry: i64,
    /// YES price (ask)
    pub yes_price: Decimal,
    /// NO price (ask)
    pub no_price: Decimal,
    /// Sum of YES + NO (should be < 1.0)
    pub sum_prices: Decimal,
    /// Raw edge before fees (1.0 - sum_prices)
    pub raw_edge: Decimal,
    /// Net edge after 2% fees
    pub net_edge: Decimal,
    /// Position size for each side (buy equal amounts)
    pub position_size: Decimal,
    /// Expected profit after fees
    pub expected_profit: Decimal,
    /// YES asset ID for execution
    pub yes_asset_id: String,
    /// NO asset ID for execution
    pub no_asset_id: String,
    /// Minimum liquidity available
    pub min_liquidity: Decimal,
    /// Detection timestamp
    pub detected_at: i64,
    /// Annualized return (for comparison)
    pub annualized_return: Decimal,
}

impl std::fmt::Display for ShortWindowArbOpportunity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "⚡ 15m ARB: {} | YES={:.2}¢ + NO={:.2}¢ = {:.2}¢ | Edge: {:.2}% | Profit: ${:.2} | Expires: {}min",
            self.market_question.chars().take(30).collect::<String>(),
            self.yes_price * Decimal::ONE_HUNDRED,
            self.no_price * Decimal::ONE_HUNDRED,
            self.sum_prices * Decimal::ONE_HUNDRED,
            self.net_edge * Decimal::ONE_HUNDRED,
            self.expected_profit,
            self.minutes_to_expiry
        )
    }
}

impl ArbitrageOpportunity {
    /// Calculate quality score based on RN1 strategy metrics
    /// Returns score 0-10 (higher = better opportunity)
    pub fn calculate_quality_score(&self) -> Decimal {
        use rust_decimal_macros::dec;

        // 1. Edge quality (weight: 40%)
        let edge_score = (self.total_edge * dec!(100)).min(dec!(10));

        // 2. Liquidity depth (weight: 30%)
        let liquidity_score = (self.min_liquidity / dec!(1000)).min(dec!(10));

        // 3. Position size (weight: 20%)
        let size_score = (self.position_size / dec!(500)).min(dec!(10));

        // 4. Expected profit (weight: 10%)
        let profit_score = (self.net_profit / dec!(50)).min(dec!(10));

        // Weighted score
        (edge_score * dec!(0.4))
            + (liquidity_score * dec!(0.3))
            + (size_score * dec!(0.2))
            + (profit_score * dec!(0.1))
    }
}

pub struct ArbEngine {
    config: Arc<Config>,
    detections: u64,
    executions: u64,
    latency_tracker: crate::utils::LatencyTracker,
}

impl ArbEngine {
    pub fn new(config: &Config) -> Self {
        Self {
            config: Arc::new(config.clone()),
            detections: 0,
            executions: 0,
            latency_tracker: crate::utils::LatencyTracker::new(),
        }
    }

    #[inline]
    pub fn detect_arbitrage(
        &mut self,
        orderbook_manager: &OrderBookManager,
        market_id: &str,
        risk_manager: &RiskManager,
    ) -> Result<Option<ArbitrageOpportunity>> {
        let start = std::time::Instant::now();

        let market_books = orderbook_manager
            .get_market_books(market_id)
            .context("Market not found")?;

        let best_asks = orderbook_manager
            .get_best_asks_for_market(market_id)
            .context("Failed to get best asks")?;

        if best_asks.is_empty() {
            return Ok(None);
        }

        let arb_op = if market_books.is_binary() {
            self.detect_binary_arbitrage(market_id, &market_books, &best_asks, risk_manager)?
        } else {
            self.detect_multi_outcome_arbitrage(market_id, &market_books, &best_asks, risk_manager)?
        };

        // Record latency after detection is done
        let elapsed = start.elapsed().as_nanos() as u64;
        self.latency_tracker.record(elapsed);

        if let Some(ref op) = arb_op {
            self.detections += 1;
            info!(
                "🎯 Arbitrage detected #{}: {} (latency: {:.2}ms)",
                self.detections,
                op,
                self.latency_tracker.avg_latency_ms()
            );
        }

        Ok(arb_op)
    }

    /// RN1 strategy: Only execute high-quality opportunities
    pub fn should_execute_opportunity(&self, arb_op: &ArbitrageOpportunity) -> bool {
        use rust_decimal_macros::dec;

        let quality_score = arb_op.calculate_quality_score();
        let min_quality = dec!(5.0); // Top 50% threshold

        if quality_score < min_quality {
            debug!(
                "⏭️  Skipping low-quality opportunity: {} (score: {:.2}/10, min: {:.2})",
                arb_op.market_id, quality_score, min_quality
            );
            return false;
        }

        info!(
            "✅ High-quality opportunity: {} (score: {:.2}/10)",
            arb_op.market_id, quality_score
        );
        true
    }

    #[inline]
    fn detect_binary_arbitrage(
        &self,
        market_id: &str,
        _market_books: &crate::orderbook::MarketBooks,
        best_asks: &[(String, Decimal, Decimal)],
        risk_manager: &RiskManager,
    ) -> Result<Option<ArbitrageOpportunity>> {
        if best_asks.len() != 2 {
            return Ok(None);
        }

        let (asset_yes, price_yes, size_yes) = &best_asks[0];
        let (asset_no, price_no, size_no) = &best_asks[1];

        let min_liquidity = (*size_yes).min(*size_no);

        if min_liquidity < self.config.trading.min_liquidity.into() {
            debug!(
                "Insufficient liquidity for {}: ${} < ${}",
                market_id, min_liquidity, self.config.trading.min_liquidity
            );
            return Ok(None);
        }

        let sum_prices = *price_yes + *price_no;

        if sum_prices >= Decimal::ONE {
            debug!(
                "No arbitrage for {}: sum = {:.4} >= 1.0",
                market_id, sum_prices
            );
            return Ok(None);
        }

        let raw_edge = Decimal::ONE - sum_prices;
        let fee_rate = Decimal::from(2) / Decimal::ONE_HUNDRED;

        let max_position_by_edge = self.calculate_max_position(
            raw_edge,
            self.config.trading.min_edge,
            self.config.trading.bankroll,
        );

        let max_position_by_liquidity = min_liquidity;
        let max_position_by_limit = self.config.trading.max_arb_size.into();

        let position_size = max_position_by_edge
            .min(max_position_by_liquidity)
            .min(max_position_by_limit);

        if position_size < self.config.trading.min_liquidity.into() {
            debug!("Position too small for {}: ${}", market_id, position_size);
            return Ok(None);
        }

        let expected_cost = position_size * sum_prices;
        let expected_payout = position_size * Decimal::ONE;
        let fee_cost = expected_payout * fee_rate;
        let net_profit = expected_payout - expected_cost - fee_cost;

        if net_profit <= Decimal::ZERO {
            debug!("No profit after fees for {}: ${}", market_id, net_profit);
            return Ok(None);
        }

        let total_edge = net_profit / position_size;

        if total_edge < self.config.trading.min_edge {
            debug!(
                "Edge too small for {}: {:.2}% < {:.2}%",
                market_id,
                total_edge * Decimal::ONE_HUNDRED,
                self.config.trading.min_edge * Decimal::ONE_HUNDRED
            );
            return Ok(None);
        }

        if risk_manager.is_market_blacklisted(market_id) {
            debug!("Market {} is blacklisted", market_id);
            return Ok(None);
        }

        let arb_op = ArbitrageOpportunity {
            market_id: market_id.to_string(),
            arb_type: ArbType::Binary,
            edges: vec![
                ArbEdge {
                    asset_id: asset_yes.clone(),
                    outcome: "YES".to_string(),
                    price: *price_yes,
                    size: position_size,
                    expected_cost: position_size * *price_yes,
                },
                ArbEdge {
                    asset_id: asset_no.clone(),
                    outcome: "NO".to_string(),
                    price: *price_no,
                    size: position_size,
                    expected_cost: position_size * *price_no,
                },
            ],
            total_edge,
            min_liquidity,
            position_size,
            expected_profit_usd: net_profit + fee_cost,
            fee_cost,
            net_profit,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            detection_latency_ms: None,
        };

        Ok(Some(arb_op))
    }

    #[inline]
    fn detect_multi_outcome_arbitrage(
        &self,
        market_id: &str,
        _market_books: &crate::orderbook::MarketBooks,
        best_asks: &[(String, Decimal, Decimal)],
        risk_manager: &RiskManager,
    ) -> Result<Option<ArbitrageOpportunity>> {
        if best_asks.len() < 2 {
            return Ok(None);
        }

        let sum_prices: Decimal = best_asks.iter().map(|(_, price, _)| *price).sum();

        if sum_prices >= Decimal::ONE {
            return Ok(None);
        }

        let min_liquidity: Decimal = best_asks
            .iter()
            .map(|(_, _, size)| *size)
            .min()
            .unwrap_or(Decimal::ZERO);

        if min_liquidity < self.config.trading.min_liquidity.into() {
            return Ok(None);
        }

        let raw_edge = Decimal::ONE - sum_prices;
        let fee_rate = Decimal::from(2) / Decimal::ONE_HUNDRED;

        let max_position_by_edge = self.calculate_max_position(
            raw_edge,
            self.config.trading.min_edge,
            self.config.trading.bankroll,
        );

        let max_position_by_liquidity = min_liquidity * Decimal::from(best_asks.len() as i64);
        let max_position_by_limit = self.config.trading.max_arb_size.into();

        let position_size = max_position_by_edge
            .min(max_position_by_liquidity)
            .min(max_position_by_limit);

        if position_size < self.config.trading.min_liquidity.into() {
            return Ok(None);
        }

        let per_outcome_position = position_size / Decimal::from(best_asks.len() as i64);

        let expected_cost = position_size * sum_prices;
        let expected_payout = position_size * Decimal::ONE;
        let fee_cost = expected_payout * fee_rate;
        let net_profit = expected_payout - expected_cost - fee_cost;

        if net_profit <= Decimal::ZERO {
            return Ok(None);
        }

        let total_edge = net_profit / position_size;

        if total_edge < self.config.trading.min_edge {
            return Ok(None);
        }

        if risk_manager.is_market_blacklisted(market_id) {
            return Ok(None);
        }

        let edges: Vec<ArbEdge> = best_asks
            .iter()
            .enumerate()
            .map(|(i, (asset_id, price, _size))| ArbEdge {
                asset_id: asset_id.clone(),
                outcome: format!("Outcome_{}", i),
                price: *price,
                size: per_outcome_position,
                expected_cost: per_outcome_position * *price,
            })
            .collect();

        let arb_op = ArbitrageOpportunity {
            market_id: market_id.to_string(),
            arb_type: ArbType::MultiOutcome,
            edges,
            total_edge,
            min_liquidity,
            position_size,
            expected_profit_usd: net_profit + fee_cost,
            fee_cost,
            net_profit,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            detection_latency_ms: None,
        };

        Ok(Some(arb_op))
    }

    #[inline]
    fn calculate_max_position(
        &self,
        raw_edge: Decimal,
        min_edge: Decimal,
        _bankroll: u64,
    ) -> Decimal {
        let edge_ratio = raw_edge / min_edge;
        let base_max = Decimal::from(self.config.trading.max_arb_size);

        if edge_ratio > Decimal::ONE {
            base_max * edge_ratio.min(Decimal::from(2))
        } else {
            base_max * edge_ratio
        }
    }

    // =========================================================================
    // Short-Window Arbitrage Detection (gabagool-style)
    // =========================================================================

    /// Detect arbitrage opportunities in short-window markets (15-30 min)
    /// Uses lower edge threshold since fast turnover = more cycles = more profit
    pub fn detect_short_window_arbitrage(
        &mut self,
        orderbook_manager: &OrderBookManager,
        market: &Market,
        markets_config: &crate::utils::MarketsConfig,
        risk_manager: &RiskManager,
    ) -> Result<Option<ShortWindowArbOpportunity>> {
        use rust_decimal_macros::dec;

        // Check if this is a short-window market
        let short_window_info = market.analyze_short_window(markets_config);
        if !short_window_info.is_short_window {
            return Ok(None);
        }

        let minutes_to_expiry = short_window_info.minutes_to_expiry.unwrap_or(0);

        // Skip if too close to expiry (settlement risk)
        if minutes_to_expiry < markets_config.min_minutes_to_expiry as i64 {
            debug!(
                "Short-window market too close to expiry: {} ({}min < {}min)",
                market.question, minutes_to_expiry, markets_config.min_minutes_to_expiry
            );
            return Ok(None);
        }

        // Get best asks for YES and NO
        let best_asks = orderbook_manager
            .get_best_asks_for_market(&market.market)
            .context("Failed to get best asks for short-window market")?;

        if best_asks.len() != 2 {
            return Ok(None); // Must be binary market
        }

        let (yes_asset_id, yes_price, yes_size) = &best_asks[0];
        let (no_asset_id, no_price, no_size) = &best_asks[1];

        // Calculate sum and edge
        let sum_prices = *yes_price + *no_price;

        // The magic: if YES + NO < $1, we have guaranteed profit
        if sum_prices >= Decimal::ONE {
            debug!(
                "No short-window arb: {} sum = {:.4} >= 1.0",
                market.question, sum_prices
            );
            return Ok(None);
        }

        let raw_edge = Decimal::ONE - sum_prices;
        let fee_rate = dec!(0.02); // 2% Polymarket fee

        // Net edge after fees
        let net_edge = raw_edge - fee_rate;

        // Use lower threshold for short-window markets
        let min_edge = self.config.trading.short_window_min_edge;
        if net_edge < min_edge {
            debug!(
                "Short-window edge too small: {} {:.2}% < {:.2}%",
                market.question,
                net_edge * Decimal::ONE_HUNDRED,
                min_edge * Decimal::ONE_HUNDRED
            );
            return Ok(None);
        }

        // Check liquidity
        let min_liquidity = (*yes_size).min(*no_size);
        if min_liquidity < Decimal::from(self.config.trading.min_liquidity) {
            debug!(
                "Short-window liquidity too low: {} ${} < ${}",
                market.question, min_liquidity, self.config.trading.min_liquidity
            );
            return Ok(None);
        }

        // Check blacklist
        if risk_manager.is_market_blacklisted(&market.market) {
            return Ok(None);
        }

        // Calculate position size (conservative for short-window)
        let max_size = Decimal::from(self.config.trading.short_window_max_size);
        let position_size = min_liquidity.min(max_size);

        // Expected profit = position * net_edge
        let expected_profit = position_size * net_edge;

        // Calculate annualized return for comparison
        // If 15min resolution with 2% edge = (2% * 4 * 24 * 365) = 70,080% annualized!
        let cycles_per_year =
            Decimal::from(365 * 24 * 60) / Decimal::from(minutes_to_expiry.max(1));
        let annualized_return = net_edge * cycles_per_year;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let opportunity = ShortWindowArbOpportunity {
            market_id: market.market.clone(),
            market_question: market.question.clone(),
            minutes_to_expiry,
            yes_price: *yes_price,
            no_price: *no_price,
            sum_prices,
            raw_edge,
            net_edge,
            position_size,
            expected_profit,
            yes_asset_id: yes_asset_id.clone(),
            no_asset_id: no_asset_id.clone(),
            min_liquidity,
            detected_at: now,
            annualized_return,
        };

        self.detections += 1;
        info!("🎯 SHORT-WINDOW ARB #{}: {}", self.detections, opportunity);

        // Log the annualized return for perspective
        if annualized_return > dec!(1000) {
            warn!(
                "🚀 MASSIVE annualized return: {:.0}% ({:.2}% edge × {} cycles/year)",
                annualized_return,
                net_edge * Decimal::ONE_HUNDRED,
                cycles_per_year
            );
        }

        Ok(Some(opportunity))
    }

    /// Scan all short-window markets for arbitrage opportunities
    pub fn scan_short_window_markets(
        &mut self,
        orderbook_manager: &OrderBookManager,
        markets: &[Market],
        markets_config: &crate::utils::MarketsConfig,
        risk_manager: &RiskManager,
    ) -> Vec<ShortWindowArbOpportunity> {
        let mut opportunities = Vec::new();

        for market in markets {
            if let Ok(Some(opp)) = self.detect_short_window_arbitrage(
                orderbook_manager,
                market,
                markets_config,
                risk_manager,
            ) {
                opportunities.push(opp);
            }
        }

        // Sort by expected profit (highest first)
        opportunities.sort_by(|a, b| b.expected_profit.cmp(&a.expected_profit));

        if !opportunities.is_empty() {
            info!(
                "⚡ Found {} short-window arb opportunities (best: ${:.2} profit)",
                opportunities.len(),
                opportunities
                    .first()
                    .map(|o| o.expected_profit)
                    .unwrap_or_default()
            );
        }

        opportunities
    }

    pub fn get_statistics(&self) -> (u64, u64, f64) {
        (
            self.detections,
            self.executions,
            if self.detections > 0 {
                self.executions as f64 / self.detections as f64
            } else {
                0.0
            },
        )
    }

    pub fn get_latency_stats(&self) -> (f64, u64, u64) {
        (
            self.latency_tracker.avg_latency_ms(),
            self.latency_tracker.p50_latency_ns(),
            self.latency_tracker.p99_latency_ns(),
        )
    }
}

// =============================================================================
// Short-Window Arb Simulation & Tracking
// =============================================================================
//
// Tracks simulated trades to measure strategy performance before going live.
// Each trade records: entry, expected resolution, and outcome.
// =============================================================================

/// A simulated short-window arb trade
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedShortWindowTrade {
    pub id: String,
    pub market_id: String,
    pub market_question: String,
    /// When we "entered" the trade (detected the opportunity)
    pub entry_time: i64,
    /// Expected resolution time (entry + minutes_to_expiry)
    pub expected_resolution_time: i64,
    /// Minutes until resolution at entry
    pub minutes_to_expiry: i64,
    /// YES price at entry
    pub yes_price: Decimal,
    /// NO price at entry
    pub no_price: Decimal,
    /// Sum of YES + NO at entry
    pub sum_prices: Decimal,
    /// Position size (same for YES and NO)
    pub position_size: Decimal,
    /// Total cost to enter (position * sum_prices)
    pub entry_cost: Decimal,
    /// Expected profit if one side resolves to $1
    pub expected_profit: Decimal,
    /// Trade status
    pub status: SimulatedTradeStatus,
    /// Actual profit (set when resolved)
    pub actual_profit: Option<Decimal>,
    /// Resolution time (set when resolved)
    pub resolution_time: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SimulatedTradeStatus {
    /// Trade is open, waiting for resolution
    Open,
    /// Trade resolved with profit (one side hit $1)
    ResolvedProfit,
    /// Trade resolved with loss (shouldn't happen in theory)
    ResolvedLoss,
    /// Trade expired without resolution (market issue)
    Expired,
}

/// Tracker for simulated short-window arb trades
#[derive(Debug, Clone)]
pub struct ShortWindowArbTracker {
    /// All simulated trades (persisted)
    trades: Vec<SimulatedShortWindowTrade>,
    /// Running P&L
    total_pnl: Decimal,
    /// Total trades entered
    trades_entered: u64,
    /// Trades resolved with profit
    trades_won: u64,
    /// Trades resolved with loss
    trades_lost: u64,
    /// Total capital deployed
    total_capital_deployed: Decimal,
    /// Win rate
    win_rate: Decimal,
    /// Average profit per trade
    avg_profit: Decimal,
    /// Simulated balance
    simulated_balance: Decimal,
    /// Initial balance
    initial_balance: Decimal,
}

impl ShortWindowArbTracker {
    pub fn new(initial_balance: Decimal) -> Self {
        info!(
            "📊 Short-Window Arb Tracker initialized with ${} balance",
            initial_balance
        );
        Self {
            trades: Vec::new(),
            total_pnl: Decimal::ZERO,
            trades_entered: 0,
            trades_won: 0,
            trades_lost: 0,
            total_capital_deployed: Decimal::ZERO,
            win_rate: Decimal::ZERO,
            avg_profit: Decimal::ZERO,
            simulated_balance: initial_balance,
            initial_balance,
        }
    }

    /// Simulate entering a short-window arb trade
    pub fn simulate_entry(&mut self, opp: &ShortWindowArbOpportunity) -> SimulatedShortWindowTrade {
        let trade_id = format!("SIM_SW_{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let entry_cost = opp.position_size * opp.sum_prices;

        // Deduct from simulated balance
        self.simulated_balance -= entry_cost;
        self.total_capital_deployed += entry_cost;
        self.trades_entered += 1;

        let trade = SimulatedShortWindowTrade {
            id: trade_id.clone(),
            market_id: opp.market_id.clone(),
            market_question: opp.market_question.clone(),
            entry_time: now,
            expected_resolution_time: now + (opp.minutes_to_expiry * 60),
            minutes_to_expiry: opp.minutes_to_expiry,
            yes_price: opp.yes_price,
            no_price: opp.no_price,
            sum_prices: opp.sum_prices,
            position_size: opp.position_size,
            entry_cost,
            expected_profit: opp.expected_profit,
            status: SimulatedTradeStatus::Open,
            actual_profit: None,
            resolution_time: None,
        };

        info!(
            "🎮 [SIM] Entered short-window arb: {} | Cost: ${:.2} | Expected: ${:.2} profit | Balance: ${:.2}",
            opp.market_question.chars().take(30).collect::<String>(),
            entry_cost,
            opp.expected_profit,
            self.simulated_balance
        );

        self.trades.push(trade.clone());
        trade
    }

    /// Simulate resolution of a trade (called when market resolves)
    /// In theory, one side always pays $1, so we always profit if sum < $1
    pub fn simulate_resolution(&mut self, trade_id: &str, won: bool) {
        if let Some(trade) = self.trades.iter_mut().find(|t| t.id == trade_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            trade.resolution_time = Some(now);

            if won {
                // One side resolved to $1, we get position_size back
                let payout = trade.position_size;
                let profit = payout - trade.entry_cost;

                trade.status = SimulatedTradeStatus::ResolvedProfit;
                trade.actual_profit = Some(profit);

                self.simulated_balance += payout;
                self.total_pnl += profit;
                self.trades_won += 1;

                info!(
                    "✅ [SIM] Trade WON: {} | Profit: ${:.2} | Total P&L: ${:.2}",
                    trade.market_question.chars().take(30).collect::<String>(),
                    profit,
                    self.total_pnl
                );
            } else {
                // This shouldn't happen in Sum-<$1 arb, but track it
                trade.status = SimulatedTradeStatus::ResolvedLoss;
                trade.actual_profit = Some(-trade.entry_cost);

                self.total_pnl -= trade.entry_cost;
                self.trades_lost += 1;

                warn!(
                    "❌ [SIM] Trade LOST: {} | Loss: ${:.2} | Total P&L: ${:.2}",
                    trade.market_question.chars().take(30).collect::<String>(),
                    trade.entry_cost,
                    self.total_pnl
                );
            }

            self.update_stats();
        }
    }

    /// Auto-resolve open trades that have passed their expected resolution time
    /// Assumes they won (since Sum-<$1 arb should always win)
    pub fn auto_resolve_expired(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let expired_ids: Vec<String> = self
            .trades
            .iter()
            .filter(|t| t.status == SimulatedTradeStatus::Open && now > t.expected_resolution_time)
            .map(|t| t.id.clone())
            .collect();

        for trade_id in expired_ids {
            // Assume win for Sum-<$1 arb (one side always pays $1)
            self.simulate_resolution(&trade_id, true);
        }
    }

    fn update_stats(&mut self) {
        let total_resolved = self.trades_won + self.trades_lost;
        if total_resolved > 0 {
            self.win_rate = Decimal::from(self.trades_won) / Decimal::from(total_resolved);
            self.avg_profit = self.total_pnl / Decimal::from(total_resolved);
        }
    }

    /// Get current statistics
    pub fn get_stats(&self) -> ShortWindowArbStats {
        ShortWindowArbStats {
            trades_entered: self.trades_entered,
            trades_open: self
                .trades
                .iter()
                .filter(|t| t.status == SimulatedTradeStatus::Open)
                .count() as u64,
            trades_won: self.trades_won,
            trades_lost: self.trades_lost,
            win_rate: self.win_rate,
            total_pnl: self.total_pnl,
            avg_profit_per_trade: self.avg_profit,
            total_capital_deployed: self.total_capital_deployed,
            simulated_balance: self.simulated_balance,
            roi: if self.initial_balance > Decimal::ZERO {
                (self.simulated_balance - self.initial_balance) / self.initial_balance
                    * Decimal::ONE_HUNDRED
            } else {
                Decimal::ZERO
            },
        }
    }

    /// Get all trades (for persistence/export)
    pub fn get_trades(&self) -> &[SimulatedShortWindowTrade] {
        &self.trades
    }

    /// Get open trades only
    pub fn get_open_trades(&self) -> Vec<&SimulatedShortWindowTrade> {
        self.trades
            .iter()
            .filter(|t| t.status == SimulatedTradeStatus::Open)
            .collect()
    }
}

/// Statistics for short-window arb tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortWindowArbStats {
    pub trades_entered: u64,
    pub trades_open: u64,
    pub trades_won: u64,
    pub trades_lost: u64,
    pub win_rate: Decimal,
    pub total_pnl: Decimal,
    pub avg_profit_per_trade: Decimal,
    pub total_capital_deployed: Decimal,
    pub simulated_balance: Decimal,
    pub roi: Decimal,
}

impl std::fmt::Display for ShortWindowArbStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SW Arb Stats: {} trades ({} open, {} won, {} lost) | Win: {:.1}% | P&L: ${:.2} | ROI: {:.2}% | Balance: ${:.2}",
            self.trades_entered,
            self.trades_open,
            self.trades_won,
            self.trades_lost,
            self.win_rate * Decimal::ONE_HUNDRED,
            self.total_pnl,
            self.roi,
            self.simulated_balance
        )
    }
}
