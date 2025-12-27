use crate::utils::{Config, ScopedTimer};
use crate::arb_engine::ArbitrageOpportunity;
use polymarket_client_sdk::clob::{
    Client,
    types::{
        OrderType,
        Side,
        SignatureType,
        SignedOrder as SdkSignedOrder,
    },
};
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use rust_decimal::Decimal;
use std::sync::Arc;
use anyhow::{Result, Context};
use tracing::{info, error, warn};
use futures::future::join_all;
use std::time::Instant;

#[derive(Debug)]
pub struct SignedOrder {
    pub order: SdkSignedOrder,
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

pub struct OrderExecutor {
    config: Arc<Config>,
    clob_client: Arc<Client<Authenticated<Normal>>>,
}

impl OrderExecutor {
    pub async fn new(config: &Config) -> Result<Self> {
        info!("🔐 Initializing order executor...");

        let private_key = config.credentials.private_key.clone();

        let clob_config = polymarket_client_sdk::clob::Config::default();

        let signature_type = match config.credentials.signature_type {
            0 => SignatureType::Eoa,
            1 => SignatureType::Proxy,
            2 => SignatureType::GnosisSafe,
            _ => return Err(anyhow::anyhow!("Invalid signature type")),
        };

        let clob_client = Client::new(&config.server.rest_url, clob_config)?;

        info!("✅ Order executor initialized");
        info!("📝 Signature type: {:?}", signature_type);

        Ok(Self {
            config: Arc::new(config.clone()),
            clob_client: Arc::new(clob_client),
        })
    }

    pub async fn execute_arbitrage(&self, arb_op: &ArbitrageOpportunity) -> Result<ExecutionResult> {
        let _timer = ScopedTimer::new("execute_arbitrage", None);

        info!(
            "🎯 Executing arbitrage for market {}",
            arb_op.market_id
        );

        let start_time = Instant::now();

        let signed_orders = self.create_signed_orders(arb_op).await?;

        info!(
            "📦 Created {} signed orders for {}",
            signed_orders.len(),
            arb_op.market_id
        );

        let submission_results = self.submit_orders_parallel(&signed_orders).await?;

        let success_count = submission_results.iter().filter(|r| r.success).count();
        let filled_count = submission_results.iter().filter(|r| r.success && r.order_id.is_some()).count();

        let total_cost = submission_results
            .iter()
            .filter(|r| r.success)
            .map(|r| arb_op.edges.iter()
                .find(|e| e.asset_id == r.asset_id)
                .map(|e| e.expected_cost)
                .unwrap_or(Decimal::ZERO))
            .sum::<Decimal>();

        let filled_amount = arb_op.edges.iter()
            .filter_map(|edge| submission_results
                .iter()
                .find(|r| r.asset_id == edge.asset_id && r.success)
                .and_then(|r| r.order_id.as_ref())
                .map(|_| edge.size))
            .sum::<Decimal>();

        let execution_time = start_time.elapsed();
        let execution_time_ms = execution_time.as_millis() as u64;

        let all_success = success_count == signed_orders.len();
        let all_filled = filled_count == signed_orders.len();
        let partial_fill = success_count > 0 && !all_filled;

        info!(
            "✅ Execution results: {}/{} orders filled, ${:.2}/${:.2} filled, {:.2}ms",
            filled_count,
            signed_orders.len(),
            filled_amount,
            arb_op.position_size,
            execution_time_ms
        );

        Ok(ExecutionResult {
            success: all_success,
            filled: all_filled,
            partial_fill: partial_fill,
            filled_amount,
            total_cost,
            orders: submission_results,
            execution_time_ms,
            error_message: if !all_success {
                Some(format!("Only {}/{} orders succeeded", success_count, signed_orders.len()))
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

            let order_builder = self.clob_client.limit_order()
                .token_id(&edge.asset_id)
                .size(size)
                .price(price)
                .side(Side::Buy)
                .order_type(OrderType::FOK);

            let signable_order: polymarket_client_sdk::clob::types::SignableOrder = order_builder.build().await
                .context("Failed to create order")?;

            let signed_order: SdkSignedOrder = self.clob_client.sign(&signable_order).await
                .context("Failed to sign order")?;

            let order_hash = self.calculate_order_hash(&signed_order);

            signed_orders.push(SignedOrder {
                order: signed_order,
                order_hash,
                created_at: Instant::now(),
            });
        }

        Ok(signed_orders)
    }

    async fn submit_orders_parallel(&self, signed_orders: &[SignedOrder]) -> Result<Vec<OrderResult>> {
        let futures: Vec<_> = signed_orders
            .iter()
            .map(|signed_order| self.submit_single_order(signed_order))
            .collect();

        let results = join_all(futures).await;

        Ok(results)
    }

    async fn submit_single_order(&self, signed_order: &SignedOrder) -> OrderResult {
        let order_id = signed_order.order.order.tokenId.to_string();
        let asset_id = signed_order.order.order.tokenId.to_string();

        match self.clob_client.post_order(signed_order.order.clone()).await {
            Ok(response) => {
                info!("✅ Order submitted: {} - {:?}", order_id, response);
                OrderResult {
                    asset_id,
                    success: true,
                    order_id: Some(order_id),
                    error: None,
                }
            }
            Err(e) => {
                error!("❌ Order failed: {} - {}", order_id, e);
                OrderResult {
                    asset_id,
                    success: false,
                    order_id: None,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    pub async fn cancel_open_orders(&self, _market_id: &str) -> Result<usize> {
        info!("🗑️  Cancelling orders");

        let response: polymarket_client_sdk::clob::types::CancelOrdersResponse = self.clob_client.cancel_all_orders().await
            .context("Failed to cancel orders")?;

        let cancel_count = response.canceled_orders.len();
        info!("✅ Cancelled {} orders", cancel_count);

        Ok(cancel_count)
    }

    pub async fn get_balance(&self) -> Result<Decimal> {
        use polymarket_client_sdk::clob::types::BalanceAllowanceRequest;

        let request = BalanceAllowanceRequest::default();
        let response = self.clob_client.balance_allowance(&request).await
            .context("Failed to get balance")?;

        Ok(Decimal::from(response.balance))
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
    fn calculate_order_hash(&self, signed_order: &SdkSignedOrder) -> String {
        use sha2::{Sha256, Digest};
        use hex;

        let mut hasher = Sha256::new();

        let order = &signed_order.order;

        hasher.update(order.tokenId.as_le_bytes().as_ref());
        hasher.update(order.makerAmount.as_le_bytes().as_ref());
        hasher.update(order.takerAmount.as_le_bytes().as_ref());
        hasher.update(order.expiration.as_le_bytes().as_ref());
        hasher.update(order.nonce.as_le_bytes().as_ref());

        let result = hasher.finalize();
        hex::encode(result)
    }
}