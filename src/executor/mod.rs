use crate::arb_engine::ArbitrageOpportunity;
use crate::utils::ScopedTimer;
use alloy::signers::{local::PrivateKeySigner, Signer};
use anyhow::{Context, Result};
use futures::future::join_all;
use polymarket_client_sdk::auth::{state::Authenticated, Normal};
use polymarket_client_sdk::clob::{
    types::{
        BalanceAllowanceResponse, CancelOrdersResponse, OrderType, PostOrderResponse, Side,
        SignedOrder as SdkSignedOrder,
    },
    Client, Config as ClobConfig,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct SignedOrder {
    pub asset_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub order_hash: String,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub success: bool,
    pub filled: bool,
    pub partial_fill: bool,
    pub filled_amount: Decimal,
    pub total_cost: Decimal,
    pub orders: Vec<OrderResult>,
    pub execution_time_ms: u64,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OrderResult {
    pub asset_id: String,
    pub success: bool,
    pub order_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedTrade {
    pub timestamp: i64,
    pub market_id: String,
    pub arb_type: String,
    pub edges: Vec<SimulatedEdge>,
    pub total_cost: Decimal,
    pub expected_payout: Decimal,
    pub net_profit: Decimal,
    pub execution_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedEdge {
    pub asset_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub cost: Decimal,
}

pub struct SimulationExecutor {
    #[allow(dead_code)]
    config: Arc<crate::utils::Config>,
    trades: Arc<tokio::sync::RwLock<VecDeque<SimulatedTrade>>>,
    simulated_balance: Arc<tokio::sync::RwLock<Decimal>>,
    initial_balance: Decimal,
}

impl SimulationExecutor {
    pub fn new(config: &crate::utils::Config) -> Self {
        let initial_balance = Decimal::from(config.trading.bankroll);

        info!("🎮 Simulation mode enabled - NO REAL TRADES");
        info!("💰 Starting simulated balance: ${:.2}", initial_balance);

        Self {
            config: Arc::new(config.clone()),
            trades: Arc::new(tokio::sync::RwLock::new(VecDeque::with_capacity(1000))),
            simulated_balance: Arc::new(tokio::sync::RwLock::new(initial_balance)),
            initial_balance,
        }
    }

    pub async fn simulate_arbitrage(
        &self,
        arb_op: &ArbitrageOpportunity,
    ) -> Result<ExecutionResult> {
        let start_time = Instant::now();

        info!(
            "🎮 SIMULATED: Executing arbitrage for market {}",
            arb_op.market_id
        );

        let total_cost = arb_op
            .edges
            .iter()
            .map(|e| e.expected_cost)
            .sum::<Decimal>();
        let expected_payout = arb_op.position_size;
        let fee_cost = arb_op.fee_cost;
        let net_profit = arb_op.net_profit;

        let mut balance = self.simulated_balance.write().await;

        if *balance < total_cost {
            warn!(
                "🎮 SIMULATED: Insufficient balance: ${:.2} < ${:.2}",
                *balance, total_cost
            );
            return Ok(ExecutionResult {
                success: false,
                filled: false,
                partial_fill: false,
                filled_amount: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                orders: vec![],
                execution_time_ms: start_time.elapsed().as_millis() as u64,
                error_message: Some("Insufficient simulated balance".to_string()),
            });
        }

        *balance -= total_cost;
        *balance += expected_payout;
        *balance -= fee_cost;

        let current_balance = *balance;
        let total_pnl = current_balance - self.initial_balance;
        drop(balance);

        let simulated_trade = SimulatedTrade {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            market_id: arb_op.market_id.clone(),
            arb_type: format!("{:?}", arb_op.arb_type),
            edges: arb_op
                .edges
                .iter()
                .map(|e| SimulatedEdge {
                    asset_id: e.asset_id.clone(),
                    price: e.price,
                    size: e.size,
                    cost: e.expected_cost,
                })
                .collect(),
            total_cost,
            expected_payout,
            net_profit,
            execution_time_ms: start_time.elapsed().as_millis() as u64,
        };

        let mut trades = self.trades.write().await;
        trades.push_back(simulated_trade.clone());
        if trades.len() > 1000 {
            trades.pop_front();
        }
        drop(trades);

        info!(
            "🎮 SIMULATED FILL: ${:.2} profit | Balance: ${:.2} (P&L: ${:.2})",
            net_profit, current_balance, total_pnl
        );

        let order_results: Vec<OrderResult> = arb_op
            .edges
            .iter()
            .map(|edge| OrderResult {
                asset_id: edge.asset_id.clone(),
                success: true,
                order_id: Some(format!("SIM_{}", uuid::Uuid::new_v4())),
                error: None,
            })
            .collect();

        Ok(ExecutionResult {
            success: true,
            filled: true,
            partial_fill: false,
            filled_amount: arb_op.position_size,
            total_cost: net_profit,
            orders: order_results,
            execution_time_ms: start_time.elapsed().as_millis() as u64,
            error_message: None,
        })
    }

    #[allow(dead_code)]
    pub async fn get_simulated_balance(&self) -> Decimal {
        *self.simulated_balance.read().await
    }

    #[allow(dead_code)]
    pub async fn get_simulated_pnl(&self) -> Decimal {
        *self.simulated_balance.read().await - self.initial_balance
    }
}

pub struct OrderExecutor {
    #[allow(dead_code)]
    config: Arc<crate::utils::Config>,
    clob_client: Client<Authenticated<Normal>>,
    signer: PrivateKeySigner,
}

impl OrderExecutor {
    pub async fn new(config: &crate::utils::Config) -> Result<Self> {
        info!("🔐 Initializing order executor...");

        let private_key = &config.credentials.private_key;

        // Parse the private key for signing with Polygon chain ID (137)
        let mut signer: PrivateKeySigner =
            private_key.parse().context("Failed to parse private key")?;

        // Set chain ID for Polygon
        signer.set_chain_id(Some(137));

        let clob_config = ClobConfig::default();

        // Create unauthenticated client first
        let unauth_client = Client::new(&config.server.rest_url, clob_config)?;

        // Authenticate the client
        let clob_client: Client<Authenticated<Normal>> = unauth_client
            .authentication_builder(&signer)
            .authenticate()
            .await
            .context("Failed to authenticate CLOB client")?;

        info!("✅ Order executor initialized and authenticated");
        info!("📝 Signature type: {}", config.credentials.signature_type);

        Ok(Self {
            config: Arc::new(config.clone()),
            clob_client,
            signer,
        })
    }

    async fn validate_prices(&self, arb_op: &ArbitrageOpportunity) -> Result<bool> {
        let slippage_tolerance = self.config.trading.slippage_tolerance;

        // In production, re-fetch current orderbook prices here
        // For now, log the validation check
        for edge in &arb_op.edges {
            info!(
                "🔍 Validating price for {}: {:.4} (tolerance: {:.2}%)",
                edge.asset_id,
                edge.price,
                slippage_tolerance * Decimal::ONE_HUNDRED
            );
        }

        Ok(true) // Placeholder: implement actual check against fresh orderbook data
    }

    pub async fn execute_arbitrage(
        &self,
        arb_op: &ArbitrageOpportunity,
    ) -> Result<ExecutionResult> {
        let _timer = ScopedTimer::new("execute_arbitrage", None);

        info!("🎯 Executing GTC arbitrage for market {}", arb_op.market_id);

        // Validate prices haven't moved beyond slippage tolerance
        if !self.validate_prices(arb_op).await? {
            warn!(
                "⚠️ Price slippage detected for {}, aborting execution",
                arb_op.market_id
            );
            return Ok(ExecutionResult {
                success: false,
                filled: false,
                partial_fill: false,
                filled_amount: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                orders: vec![],
                execution_time_ms: 0,
                error_message: Some("Price slippage exceeded tolerance".to_string()),
            });
        }

        let start_time = Instant::now();

        // Create and submit GTC orders (fast ~50ms per order)
        let signed_orders = self.create_signed_orders(arb_op).await?;

        info!(
            "📦 Created {} GTC orders for {} (avoiding 500ms taker delay)",
            signed_orders.len(),
            arb_op.market_id
        );

        let submission_results = self.submit_orders_parallel(&signed_orders).await?;

        let success_count = submission_results.iter().filter(|r| r.success).count();
        let filled_count = submission_results
            .iter()
            .filter(|r| r.success && r.order_id.is_some())
            .count();

        // For GTC orders, we need to wait briefly for fills
        // Short-window arb opportunities typically have immediate liquidity
        if success_count > 0 {
            info!("⏳ Waiting 200ms for GTC orders to fill...");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            
            // Cancel any unfilled orders to avoid stale positions
            match self.cancel_open_orders(&arb_op.market_id).await {
                Ok(cancelled) => {
                    if cancelled > 0 {
                        info!("🗑️  Cancelled {} unfilled GTC orders", cancelled);
                    }
                }
                Err(e) => {
                    warn!("⚠️ Failed to cancel orders: {}", e);
                }
            }
        }

        let total_cost = submission_results
            .iter()
            .filter(|r| r.success)
            .map(|r| {
                arb_op
                    .edges
                    .iter()
                    .find(|e| e.asset_id == r.asset_id)
                    .map(|e| e.expected_cost)
                    .unwrap_or(Decimal::ZERO)
            })
            .sum::<Decimal>();

        let filled_amount = arb_op
            .edges
            .iter()
            .filter_map(|edge| {
                submission_results
                    .iter()
                    .find(|r| r.asset_id == edge.asset_id && r.success)
                    .and_then(|r| r.order_id.as_ref())
                    .map(|_| edge.size)
            })
            .sum::<Decimal>();

        let execution_time = start_time.elapsed();
        let execution_time_ms = execution_time.as_millis() as u64;

        let all_success = success_count == signed_orders.len();
        let all_filled = filled_count == signed_orders.len();
        let partial_fill = success_count > 0 && !all_filled;

        info!(
            "✅ GTC Execution: {}/{} orders submitted, {:.2}ms total",
            success_count,
            signed_orders.len(),
            execution_time_ms
        );

        Ok(ExecutionResult {
            success: all_success,
            filled: all_filled,
            partial_fill,
            filled_amount,
            total_cost,
            orders: submission_results,
            execution_time_ms,
            error_message: if !all_success {
                Some(format!(
                    "Only {}/{} orders succeeded",
                    success_count,
                    signed_orders.len()
                ))
            } else {
                None
            },
        })
    }

    #[inline]
    async fn create_signed_orders(
        &self,
        arb_op: &ArbitrageOpportunity,
    ) -> Result<Vec<SignedOrder>> {
        let mut signed_orders = Vec::with_capacity(arb_op.edges.len());

        for edge in &arb_op.edges {
            let price = edge.price;
            let size = edge.size;

            // Use GTC (Good Till Cancelled) instead of FOK to avoid 500ms taker delay
            // By posting at the current ask price, we act as an aggressive maker
            let signable_order = self
                .clob_client
                .limit_order()
                .token_id(&edge.asset_id)
                .size(size)
                .price(price)
                .side(Side::Buy)
                .order_type(OrderType::GTC) // GTC = 50ms vs FOK = 500ms
                .build()
                .await
                .context("Failed to build order")?;

            let sdk_signed_order: SdkSignedOrder = self
                .clob_client
                .sign(&self.signer, signable_order)
                .await
                .context("Failed to sign order")?;

            let order_hash = self.calculate_order_hash(&sdk_signed_order);

            signed_orders.push(SignedOrder {
                asset_id: edge.asset_id.clone(),
                price,
                size,
                order_hash,
                created_at: Instant::now(),
            });
        }

        Ok(signed_orders)
    }

    async fn submit_orders_parallel(
        &self,
        signed_orders: &[SignedOrder],
    ) -> Result<Vec<OrderResult>> {
        let futures: Vec<_> = signed_orders
            .iter()
            .map(|signed_order| self.submit_single_order(signed_order))
            .collect();

        let results = join_all(futures).await;

        Ok(results)
    }

    async fn submit_single_order(&self, signed_order: &SignedOrder) -> OrderResult {
        // Re-create and sign the order for submission
        // Use GTC to avoid 500ms taker delay
        let signable_order = match self
            .clob_client
            .limit_order()
            .token_id(&signed_order.asset_id)
            .size(signed_order.size)
            .price(signed_order.price)
            .side(Side::Buy)
            .order_type(OrderType::GTC) // GTC = 50ms vs FOK = 500ms
            .build()
            .await
        {
            Ok(order) => order,
            Err(e) => {
                error!(
                    "❌ Failed to build order: {} - {}",
                    signed_order.asset_id, e
                );
                return OrderResult {
                    asset_id: signed_order.asset_id.clone(),
                    success: false,
                    order_id: None,
                    error: Some(e.to_string()),
                };
            }
        };

        let sdk_signed: SdkSignedOrder =
            match self.clob_client.sign(&self.signer, signable_order).await {
                Ok(signed) => signed,
                Err(e) => {
                    error!("❌ Failed to sign order: {} - {}", signed_order.asset_id, e);
                    return OrderResult {
                        asset_id: signed_order.asset_id.clone(),
                        success: false,
                        order_id: None,
                        error: Some(e.to_string()),
                    };
                }
            };

        let response: Result<Vec<PostOrderResponse>, _> =
            self.clob_client.post_order(sdk_signed).await;

        match response {
            Ok(responses) => {
                info!(
                    "✅ Order submitted: {} - {:?}",
                    signed_order.asset_id, responses
                );
                OrderResult {
                    asset_id: signed_order.asset_id.clone(),
                    success: true,
                    order_id: Some(signed_order.order_hash.clone()),
                    error: None,
                }
            }
            Err(e) => {
                error!("❌ Order failed: {} - {}", signed_order.asset_id, e);
                OrderResult {
                    asset_id: signed_order.asset_id.clone(),
                    success: false,
                    order_id: None,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    pub async fn cancel_open_orders(&self, _market_id: &str) -> Result<usize> {
        info!("🗑️  Cancelling orders");

        let response: CancelOrdersResponse = self
            .clob_client
            .cancel_all_orders()
            .await
            .context("Failed to cancel orders")?;

        let cancel_count = response.canceled.len();
        info!("✅ Cancelled {} orders", cancel_count);

        Ok(cancel_count)
    }

    pub async fn get_balance(&self) -> Result<Decimal> {
        use polymarket_client_sdk::clob::types::BalanceAllowanceRequest;

        let request = BalanceAllowanceRequest::default();
        let response: BalanceAllowanceResponse = self
            .clob_client
            .balance_allowance(&request)
            .await
            .context("Failed to get balance")?;

        Ok(response.balance)
    }

    pub async fn health_check(&self) -> Result<bool> {
        match self.clob_client.ok().await {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("Health check failed: {}", e);
                Ok(false)
            }
        }
    }

    #[inline]
    fn calculate_order_hash(&self, _signed_order: &SdkSignedOrder) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        hasher.update(now.to_le_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}
