use crate::utils::ScopedTimer;
use crate::arb_engine::ArbitrageOpportunity;
use polymarket_client_sdk::clob::{
    Client,
    Config as ClobConfig,
    types::{
        OrderType,
        Side,
        SignedOrder as SdkSignedOrder,
        PostOrderResponse,
        CancelOrdersResponse,
        BalanceAllowanceResponse,
    },
};
use polymarket_client_sdk::auth::{state::Authenticated, Builder};
use rust_decimal::Decimal;
use std::sync::Arc;
use anyhow::{Result, Context};
use tracing::{info, error, warn};
use futures::future::join_all;
use std::time::Instant;
use alloy::signers::local::PrivateKeySigner;

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

pub struct OrderExecutor {
    config: Arc<crate::utils::Config>,
    clob_client: Client<Authenticated<Builder>>,
    signer: PrivateKeySigner,
}

impl OrderExecutor {
    pub async fn new(config: &crate::utils::Config) -> Result<Self> {
        info!("🔐 Initializing order executor...");

        let private_key = &config.credentials.private_key;

        // Parse the private key for signing
        let signer: PrivateKeySigner = private_key.parse()
            .context("Failed to parse private key")?;

        let clob_config = ClobConfig::default();

        // Create unauthenticated client first
        let unauth_client = Client::new(&config.server.rest_url, clob_config)?;

        // Authenticate the client with Builder credentials
        let clob_client: Client<Authenticated<Builder>> = unauth_client
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
            partial_fill,
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

            let signable_order = self.clob_client
                .limit_order()
                .token_id(&edge.asset_id)
                .size(size)
                .price(price)
                .side(Side::Buy)
                .order_type(OrderType::FOK)
                .build()
                .await
                .context("Failed to build order")?;

            let sdk_signed_order: SdkSignedOrder = self.clob_client
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

    async fn submit_orders_parallel(&self, signed_orders: &[SignedOrder]) -> Result<Vec<OrderResult>> {
        let futures: Vec<_> = signed_orders
            .iter()
            .map(|signed_order| self.submit_single_order(signed_order))
            .collect();

        let results = join_all(futures).await;

        Ok(results)
    }

    async fn submit_single_order(&self, signed_order: &SignedOrder) -> OrderResult {
        // Re-create and sign the order for submission
        let signable_order = match self.clob_client
            .limit_order()
            .token_id(&signed_order.asset_id)
            .size(signed_order.size)
            .price(signed_order.price)
            .side(Side::Buy)
            .order_type(OrderType::FOK)
            .build()
            .await
        {
            Ok(order) => order,
            Err(e) => {
                error!("❌ Failed to build order: {} - {}", signed_order.asset_id, e);
                return OrderResult {
                    asset_id: signed_order.asset_id.clone(),
                    success: false,
                    order_id: None,
                    error: Some(e.to_string()),
                };
            }
        };

        let sdk_signed: SdkSignedOrder = match self.clob_client
            .sign(&self.signer, signable_order)
            .await
        {
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

        let response: Result<Vec<PostOrderResponse>, _> = self.clob_client
            .post_order(sdk_signed)
            .await;

        match response {
            Ok(responses) => {
                info!("✅ Order submitted: {} - {:?}", signed_order.asset_id, responses);
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

        let response: CancelOrdersResponse = self.clob_client
            .cancel_all_orders()
            .await
            .context("Failed to cancel orders")?;

        let cancel_count = response.canceled.len();
        info!("✅ Cancelled {} orders", cancel_count);

        Ok(cancel_count)
    }

    pub async fn get_balance(&self) -> Result<Decimal> {
        let response: BalanceAllowanceResponse = self.clob_client
            .balance_allowance()
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
        use sha2::{Sha256, Digest};

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
