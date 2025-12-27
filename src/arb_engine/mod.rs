use crate::orderbook::OrderBookManager;
use crate::risk::RiskManager;
use crate::utils::Config;
use crate::utils::ScopedTimer;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use std::sync::Arc;
use anyhow::{Result, Context};
use tracing::{info, debug};

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
        write!(f,
            "{} [{}]: {:.2}% edge, ${:.2} profit, ${:.2} position",
            self.arb_type,
            self.market_id,
            self.total_edge * Decimal::ONE_HUNDRED,
            self.net_profit,
            self.position_size
        )
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
        let _timer = ScopedTimer::new("arb_detection", Some(&mut self.latency_tracker));

        let market_books = orderbook_manager.get_market_books(market_id)
            .context("Market not found")?;

        let best_asks = orderbook_manager.get_best_asks_for_market(market_id)
            .context("Failed to get best asks")?;

        if best_asks.is_empty() {
            return Ok(None);
        }

        let arb_op = if market_books.is_binary() {
            self.detect_binary_arbitrage(
                market_id,
                &market_books,
                &best_asks,
                risk_manager,
            )?
        } else {
            self.detect_multi_outcome_arbitrage(
                market_id,
                &market_books,
                &best_asks,
                risk_manager,
            )?
        };

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

    #[inline]
    fn detect_binary_arbitrage(
        &self,
        market_id: &str,
        market_books: &crate::orderbook::MarketBooks,
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
            min_liquidity: min_liquidity,
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
        market_books: &crate::orderbook::MarketBooks,
        best_asks: &[(String, Decimal, Decimal)],
        risk_manager: &RiskManager,
    ) -> Result<Option<ArbitrageOpportunity>> {
        if best_asks.len() < 2 {
            return Ok(None);
        }

        let sum_prices: Decimal = best_asks
            .iter()
            .map(|(_, price, _)| *price)
            .sum();

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
            .map(|(i, (asset_id, price, size))| ArbEdge {
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
        bankroll: u64,
    ) -> Decimal {
        let edge_ratio = raw_edge / min_edge;
        let base_max = Decimal::from(self.config.trading.max_arb_size);

        if edge_ratio > Decimal::ONE {
            base_max * edge_ratio.min(Decimal::from(2))
        } else {
            base_max * edge_ratio
        }
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
