use crate::arb_engine::ArbitrageOpportunity;
use crate::executor::ExecutionResult;
use crate::utils::Config;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use anyhow::{Result, Context};
use tracing::{info, warn, debug};
use chrono::Utc;

#[derive(Debug, Clone)]
pub struct Position {
    pub market_id: String,
    pub asset_id: String,
    pub outcome: String,
    pub position_type: PositionType,
    pub size: Decimal,
    pub avg_price: Decimal,
    pub total_cost: Decimal,
    pub entry_time: i64,
    pub current_pnl: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PositionType {
    Long,
    SyntheticShort,
}

#[derive(Debug, Clone)]
pub struct Inventory {
    pub net_delta: Decimal,
    pub total_exposure: Decimal,
    pub market_count: usize,
    pub last_update: i64,
}

#[derive(Debug, Clone)]
pub struct DailyPnlTracker {
    pub date: String,
    pub realized_pnl: Decimal,
    pub unrealized_pnl: Decimal,
    pub total_pnl: Decimal,
    pub trade_count: u64,
    pub arb_count: u64,
}

pub struct RiskManager {
    config: Arc<Config>,
    positions: HashMap<String, Position>,
    market_exposure: HashMap<String, Decimal>,
    event_exposure: HashMap<String, Decimal>,
    daily_pnl: DailyPnlTracker,
    active_arbs: usize,
    last_cleanup: i64,
}

impl RiskManager {
    pub fn new(config: &Config) -> Self {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        Self {
            config: Arc::new(config.clone()),
            positions: HashMap::new(),
            market_exposure: HashMap::new(),
            event_exposure: HashMap::new(),
            daily_pnl: DailyPnlTracker {
                date: today.clone(),
                realized_pnl: Decimal::ZERO,
                unrealized_pnl: Decimal::ZERO,
                total_pnl: Decimal::ZERO,
                trade_count: 0,
                arb_count: 0,
            },
            active_arbs: 0,
            last_cleanup: Utc::now().timestamp(),
        }
    }

    #[inline]
    pub fn can_execute_arbitrage(
        &mut self,
        arb_op: &ArbitrageOpportunity,
    ) -> Result<bool> {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        if self.daily_pnl.date != today {
            self.reset_daily_pnl(&today);
        }

        if self.active_arbs >= self.config.risk.max_concurrent_arbs {
            debug!(
                "Max concurrent arbs reached: {} >= {}",
                self.active_arbs,
                self.config.risk.max_concurrent_arbs
            );
            return Ok(false);
        }

        if self.daily_pnl.total_pnl < -Decimal::from(self.config.risk.daily_loss_limit) {
            warn!(
                "⚠️  Daily loss limit reached: ${:.2} < ${}",
                self.daily_pnl.total_pnl,
                self.config.risk.daily_loss_limit
            );
            return Ok(false);
        }

        let current_market_exposure = self.market_exposure
            .get(&arb_op.market_id)
            .copied()
            .unwrap_or(Decimal::ZERO);

        let new_market_exposure = current_market_exposure + arb_op.position_size;

        if new_market_exposure > self.config.risk.max_exposure_per_market.into() {
            debug!(
                "Market exposure limit: ${:.2} > ${}",
                new_market_exposure,
                self.config.risk.max_exposure_per_market
            );
            return Ok(false);
        }

        let new_inventory = self.calculate_inventory_change(arb_op)?;

        if (self.calculate_current_inventory().net_delta + new_inventory.net_delta).abs()
            > self.config.risk.inventory_drift_threshold
        {
            warn!(
                "⚠️  Inventory drift too large: {:.2} > {:.2}",
                (self.calculate_current_inventory().net_delta + new_inventory.net_delta).abs(),
                self.config.risk.inventory_drift_threshold
            );
            return Ok(false);
        }

        if arb_op.min_liquidity < self.config.trading.min_liquidity.into() {
            debug!("Insufficient liquidity: ${}", arb_op.min_liquidity);
            return Ok(false);
        }

        Ok(true)
    }

    #[inline]
    pub fn record_arbitrage_execution(
        &mut self,
        arb_op: &ArbitrageOpportunity,
        result: &ExecutionResult,
    ) -> Result<()> {
        if result.success || result.partial_fill {
            self.active_arbs += 1;

            for edge in &arb_op.edges {
                self.add_position(
                    arb_op.market_id.clone(),
                    edge.asset_id.clone(),
                    edge.outcome.clone(),
                    PositionType::Long,
                    edge.size,
                    edge.price,
                    edge.expected_cost,
                )?;

                *self.market_exposure
                    .entry(arb_op.market_id.clone())
                    .or_insert(Decimal::ZERO) += edge.size;

                *self.event_exposure
                    .entry(arb_op.market_id.clone())
                    .or_insert(Decimal::ZERO) += edge.size;
            }

            self.daily_pnl.arb_count += 1;
            self.daily_pnl.trade_count += 1;

            if result.filled {
                self.daily_pnl.realized_pnl += result.total_cost;
            }

            info!(
                "📊 Recorded arbitrage execution: ${:.2} profit, {} active arbs",
                arb_op.net_profit,
                self.active_arbs
            );
        }

        self.cleanup_stale_positions();

        Ok(())
    }

    #[inline]
    pub fn is_market_blacklisted(&self, market_id: &str) -> bool {
        self.config.markets.blacklisted_markets
            .iter()
            .any(|blacklisted| market_id.contains(blacklisted))
    }

    #[inline]
    pub fn get_inventory(&self) -> Inventory {
        self.calculate_current_inventory()
    }

    #[inline]
    pub fn get_daily_pnl(&self) -> &DailyPnlTracker {
        &self.daily_pnl
    }

    #[inline]
    pub fn get_position(&self, asset_id: &str) -> Option<&Position> {
        self.positions.get(asset_id)
    }

    #[inline]
    pub fn get_market_exposure(&self, market_id: &str) -> Decimal {
        self.market_exposure
            .get(market_id)
            .copied()
            .unwrap_or(Decimal::ZERO)
    }

    #[inline]
    pub fn get_event_exposure(&self, event_id: &str) -> Decimal {
        self.event_exposure
            .get(event_id)
            .copied()
            .unwrap_or(Decimal::ZERO)
    }

    #[inline]
    fn add_position(
        &mut self,
        market_id: String,
        asset_id: String,
        outcome: String,
        position_type: PositionType,
        size: Decimal,
        price: Decimal,
        cost: Decimal,
    ) -> Result<()> {
        let entry_time = Utc::now().timestamp();

        let position = Position {
            market_id: market_id.clone(),
            asset_id: asset_id.clone(),
            outcome: outcome.clone(),
            position_type: position_type.clone(),
            size,
            avg_price: price,
            total_cost: cost,
            entry_time,
            current_pnl: Decimal::ZERO,
        };

        self.positions.insert(asset_id.clone(), position);

        debug!(
            "➕ Added position: {} {} @ {:.4} = ${:.2}",
            asset_id, outcome, price, cost
        );

        Ok(())
    }

    #[inline]
    fn calculate_inventory_change(&self, arb_op: &ArbitrageOpportunity) -> Result<Inventory> {
        let mut net_delta = Decimal::ZERO;
        let mut total_exposure = Decimal::ZERO;

        for edge in &arb_op.edges {
            net_delta += edge.size;
            total_exposure += edge.size * edge.price;
        }

        Ok(Inventory {
            net_delta,
            total_exposure,
            market_count: 1,
            last_update: arb_op.timestamp,
        })
    }

    #[inline]
    fn calculate_current_inventory(&self) -> Inventory {
        let mut net_delta = Decimal::ZERO;
        let mut total_exposure = Decimal::ZERO;

        for position in self.positions.values() {
            net_delta += position.size;
            total_exposure += position.total_cost;
        }

        Inventory {
            net_delta,
            total_exposure,
            market_count: self.market_exposure.len(),
            last_update: Utc::now().timestamp(),
        }
    }

    #[inline]
    fn cleanup_stale_positions(&mut self) {
        let now = Utc::now().timestamp();
        let timeout_secs = self.config.risk.position_timeout_seconds;
        let mut stale_asset_ids = Vec::new();

        for (asset_id, position) in &self.positions {
            if now - position.entry_time > timeout_secs as i64 {
                stale_asset_ids.push(asset_id.clone());

                *self.market_exposure
                    .get_mut(&position.market_id)
                    .unwrap() -= position.size;

                self.active_arbs = self.active_arbs.saturating_sub(1);

                info!(
                    "🗑️  Cleaned up stale position: {} (age: {}s)",
                    asset_id,
                    now - position.entry_time
                );
            }
        }

        for asset_id in stale_asset_ids {
            self.positions.remove(&asset_id);
        }

        self.last_cleanup = now;
    }

    #[inline]
    fn reset_daily_pnl(&mut self, today: &str) {
        info!(
            "🔄 Resetting daily PnL: ${:.2} -> $0.00 ({} trades)",
            self.daily_pnl.total_pnl,
            self.daily_pnl.trade_count
        );

        self.daily_pnl = DailyPnlTracker {
            date: today.to_string(),
            realized_pnl: Decimal::ZERO,
            unrealized_pnl: Decimal::ZERO,
            total_pnl: Decimal::ZERO,
            trade_count: 0,
            arb_count: 0,
        };
    }

    #[inline]
    pub fn should_stop_trading(&self) -> bool {
        self.daily_pnl.total_pnl < -Decimal::from(self.config.risk.daily_loss_limit)
    }

    #[inline]
    pub fn get_risk_summary(&self) -> RiskSummary {
        RiskSummary {
            active_positions: self.positions.len(),
            active_arbitrages: self.active_arbs,
            total_exposure: self.calculate_current_inventory().total_exposure,
            net_delta: self.calculate_current_inventory().net_delta,
            daily_pnl: self.daily_pnl.total_pnl,
            daily_trades: self.daily_pnl.trade_count,
            market_exposure: self.market_exposure.clone(),
            event_exposure: self.event_exposure.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RiskSummary {
    pub active_positions: usize,
    pub active_arbitrages: usize,
    pub total_exposure: Decimal,
    pub net_delta: Decimal,
    pub daily_pnl: Decimal,
    pub daily_trades: u64,
    pub market_exposure: HashMap<String, Decimal>,
    pub event_exposure: HashMap<String, Decimal>,
}
