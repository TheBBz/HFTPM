//! Volume Farmer Module - RN1-style "Trash Farming" for airdrop qualification
//!
//! This module implements the volume farming strategy:
//! 1. Find contracts priced at $0.01-$0.05 (almost certain to lose)
//! 2. Buy them to generate massive notional volume cheaply
//! 3. Qualify for POLY airdrop and platform rewards based on volume
//!
//! Example: Buying $10 of contracts at $0.01 = $1,000 notional volume
//! The $10 loss is worth it if airdrop allocation > $10

use crate::gamma_api::Market;
use crate::orderbook::OrderBookManager;
use crate::utils::Config;
use anyhow::Result;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// A trash farming opportunity
#[derive(Debug, Clone)]
pub struct TrashOpportunity {
    pub market_id: String,
    pub asset_id: String,
    pub outcome_name: String,
    pub price: Decimal,
    pub available_size: Decimal,
    pub cost_for_volume: Decimal,   // How much we pay
    pub notional_volume: Decimal,   // Volume credit we get
    pub volume_multiplier: Decimal, // notional_volume / cost
}

/// Record of a trash trade
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashTrade {
    pub timestamp: i64,
    pub market_id: String,
    pub asset_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub cost: Decimal,
    pub notional_volume: Decimal,
}

/// Volume farming statistics
#[derive(Debug, Clone, Default)]
pub struct VFStats {
    pub trades_executed: u64,
    pub total_cost: Decimal,
    pub total_notional_volume: Decimal,
    pub avg_volume_multiplier: Decimal,
    pub daily_budget_used: Decimal,
    pub estimated_airdrop_value: Decimal,
}

/// The Volume Farmer engine
pub struct VolumeFarmer {
    config: Arc<Config>,
    trades: Vec<TrashTrade>,
    daily_spend: Decimal,
    total_volume: Decimal,
    last_reset: Instant,
    simulated_balance: Decimal,
    initial_balance: Decimal,
}

impl VolumeFarmer {
    pub fn new(config: &Config) -> Self {
        let initial_balance = Decimal::from(config.trading.bankroll);

        info!("🗑️  Volume Farmer initialized");
        info!("   Max price: ${:.2}", config.trading.vf_max_price);
        info!(
            "   Min volume/trade: ${}",
            config.trading.vf_min_volume_per_trade
        );
        info!("   Daily budget: ${}", config.trading.vf_daily_budget);

        Self {
            config: Arc::new(config.clone()),
            trades: Vec::new(),
            daily_spend: Decimal::ZERO,
            total_volume: Decimal::ZERO,
            last_reset: Instant::now(),
            simulated_balance: initial_balance,
            initial_balance,
        }
    }

    /// Find trash farming opportunities (cheap contracts to buy for volume)
    pub fn find_opportunities(
        &self,
        orderbook_manager: &OrderBookManager,
        markets: &[Market],
    ) -> Vec<TrashOpportunity> {
        let mut opportunities = Vec::new();
        let max_price = self.config.trading.vf_max_price;
        let min_volume = Decimal::from(self.config.trading.vf_min_volume_per_trade);

        for market in markets {
            for (i, asset_id) in market.assets_ids.iter().enumerate() {
                // Get best ask for this asset (cheapest we can buy)
                if let Some(book) = orderbook_manager.get_book(&market.market, asset_id) {
                    // Find asks at or below our max price
                    for (price, size) in book.asks.iter() {
                        if *price <= max_price && *price > Decimal::ZERO {
                            // Calculate volume multiplier
                            // If price = $0.01, then $1 buys 100 contracts = $100 notional
                            let contracts_per_dollar = Decimal::ONE / *price;
                            let volume_multiplier = contracts_per_dollar;

                            // Calculate cost needed for minimum volume
                            let cost_for_min_volume = min_volume / volume_multiplier;
                            let notional_volume = cost_for_min_volume * volume_multiplier;

                            // Only include if we have enough size available
                            if *size >= cost_for_min_volume / *price {
                                let outcome_name = market
                                    .outcomes
                                    .get(i)
                                    .map(|o| o.name.clone())
                                    .unwrap_or_else(|| format!("Outcome_{}", i));

                                opportunities.push(TrashOpportunity {
                                    market_id: market.market.clone(),
                                    asset_id: asset_id.clone(),
                                    outcome_name,
                                    price: *price,
                                    available_size: *size,
                                    cost_for_volume: cost_for_min_volume,
                                    notional_volume,
                                    volume_multiplier,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Sort by volume multiplier (highest first = best deals)
        opportunities.sort_by(|a, b| b.volume_multiplier.cmp(&a.volume_multiplier));

        opportunities
    }

    /// Simulate executing a trash trade
    pub async fn simulate_trash_trade(
        &mut self,
        opportunity: &TrashOpportunity,
    ) -> Result<Option<TrashTrade>> {
        // Check daily budget
        if self.daily_spend >= Decimal::from(self.config.trading.vf_daily_budget) {
            debug!(
                "Daily budget exhausted: ${:.2} / ${}",
                self.daily_spend, self.config.trading.vf_daily_budget
            );
            return Ok(None);
        }

        // Check if we have enough balance
        if self.simulated_balance < opportunity.cost_for_volume {
            warn!(
                "Insufficient balance for trash trade: ${:.2} < ${:.2}",
                self.simulated_balance, opportunity.cost_for_volume
            );
            return Ok(None);
        }

        // Calculate how much we can actually spend
        let remaining_budget =
            Decimal::from(self.config.trading.vf_daily_budget) - self.daily_spend;
        let actual_cost = opportunity.cost_for_volume.min(remaining_budget);
        let actual_volume = actual_cost * opportunity.volume_multiplier;

        // Execute the trade
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let trade = TrashTrade {
            timestamp,
            market_id: opportunity.market_id.clone(),
            asset_id: opportunity.asset_id.clone(),
            price: opportunity.price,
            size: actual_cost / opportunity.price,
            cost: actual_cost,
            notional_volume: actual_volume,
        };

        // Update state
        self.simulated_balance -= actual_cost;
        self.daily_spend += actual_cost;
        self.total_volume += actual_volume;
        self.trades.push(trade.clone());

        info!(
            "🗑️  [SIM] TRASH TRADE: {} @ ${:.4} | Cost: ${:.2} | Volume: ${:.0} ({}x) | Balance: ${:.2}",
            opportunity.outcome_name,
            opportunity.price,
            actual_cost,
            actual_volume,
            opportunity.volume_multiplier,
            self.simulated_balance
        );

        Ok(Some(trade))
    }

    /// Reset daily budget (call at midnight UTC)
    pub fn reset_daily_budget(&mut self) {
        self.daily_spend = Decimal::ZERO;
        self.last_reset = Instant::now();
        info!("🗑️  Daily budget reset");
    }

    /// Check if we should reset (24h passed)
    pub fn should_reset_budget(&self) -> bool {
        self.last_reset.elapsed().as_secs() >= 86400
    }

    /// Get current statistics
    pub fn get_stats(&self) -> VFStats {
        let avg_multiplier = if !self.trades.is_empty() {
            self.total_volume / self.trades.iter().map(|t| t.cost).sum::<Decimal>()
        } else {
            Decimal::ZERO
        };

        // Rough estimate: assume 1% of volume translates to airdrop value
        // This is speculative - actual airdrop allocation is unknown
        let estimated_airdrop = self.total_volume * dec!(0.001);

        VFStats {
            trades_executed: self.trades.len() as u64,
            total_cost: self.trades.iter().map(|t| t.cost).sum(),
            total_notional_volume: self.total_volume,
            avg_volume_multiplier: avg_multiplier,
            daily_budget_used: self.daily_spend,
            estimated_airdrop_value: estimated_airdrop,
        }
    }

    /// Get remaining daily budget
    pub fn remaining_budget(&self) -> Decimal {
        let budget = Decimal::from(self.config.trading.vf_daily_budget);
        (budget - self.daily_spend).max(Decimal::ZERO)
    }

    /// Get simulated balance
    pub fn get_balance(&self) -> Decimal {
        self.simulated_balance
    }

    /// Get P&L
    pub fn get_pnl(&self) -> Decimal {
        self.simulated_balance - self.initial_balance
    }
}

impl std::fmt::Display for VFStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Volume Farm: {} trades | Cost: ${:.2} | Volume: ${:.0} | Avg {}x | Est. Airdrop: ${:.2}",
            self.trades_executed,
            self.total_cost,
            self.total_notional_volume,
            self.avg_volume_multiplier,
            self.estimated_airdrop_value
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_multiplier() {
        // At $0.01, $1 buys 100 contracts = $100 notional volume
        let price = dec!(0.01);
        let cost = dec!(1.0);
        let contracts = cost / price; // 100
        let notional = contracts * Decimal::ONE; // $100

        assert_eq!(contracts, dec!(100));
        assert_eq!(notional, dec!(100));
    }

    #[test]
    fn test_volume_at_different_prices() {
        // $10 at $0.01 = $1000 volume (100x)
        // $10 at $0.05 = $200 volume (20x)
        // $10 at $0.10 = $100 volume (10x)

        let cost = dec!(10.0);

        let vol_001 = cost / dec!(0.01);
        let vol_005 = cost / dec!(0.05);
        let vol_010 = cost / dec!(0.10);

        assert_eq!(vol_001, dec!(1000));
        assert_eq!(vol_005, dec!(200));
        assert_eq!(vol_010, dec!(100));
    }
}
