use crate::utils::{Config, ScopedTimer};
use crate::arb_engine::ArbitrageOpportunity;
use polymarket_client_sdk::clob::{
    Client,
    Config as ClobConfig,
    types::{
        Amount,
        OrderType,
        Side,
        SignatureType,
        Order,
        OrderArgs,
    },
};
use alloy_primitives::{Address, address};
use alloy_signers::Signer as _;
use alloy_signer_local::LocalSigner;
use rust_decimal::Decimal;
use std::sync::Arc;
use anyhow::{Result, Context};
use tracing::{info, error, warn, debug};
use futures::future::join_all;
use std::time::Instant;
use chrono::Utc;

#[derive(Debug, Clone)]
pub struct SignedOrder {
    pub order: Order,
    pub order_args: OrderArgs,
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
    clob_client: Arc<Client>,
    signer: LocalSigner<alloy_signers::coins::Coins>,
}

impl OrderExecutor {
    pub async fn new(config: &Config) -> Result<Self> {
        info!("🔐 Initializing order executor...");

        let private_key = config.credentials.private_key.clone();
        let signer = LocalSigner::from_str(&private_key)
            .context("Failed to parse private key")?
            .with_chain_id(Some(137));

        let clob_config = ClobConfig::default();

        let signature_type = match config.credentials.signature_type {
            0 => SignatureType::Eoa,
            1 => SignatureType::Proxy,
            2 => SignatureType::GnosisSafe,
            _ => return Err(anyhow::anyhow!("Invalid signature type")),
        };

        let funder_address: Address = config.credentials.funder_address.parse()
            .context("Failed to parse funder address")?;

        let mut clob_client = Client::new(
            &config.server.rest_url,
            clob_config,
        )?
        .authentication_builder(&signer)
        .funder(funder_address)
        .signature_type(signature_type)
        .authenticate()
        .await
        .context("Failed to authenticate CLOB client")?;

        info!("✅ Order executor initialized");
        info!("📝 Signature type: {:?}", signature_type);
        info!("💰 Funder address: {}", funder_address);

        Ok(Self {
            config: Arc::new(config.clone()),
            clob_client: Arc::new(clob_client),
            signer,
        })
    }

    #[inline]
    pub async fn execute_arbitrage(
        &self,
        arb_op: ArbitrageOpportunity,
    ) -> Result<ExecutionResult> {
        let _timer = ScopedTimer::new("execution", None);

        let start_time = Instant::now();

        let signed_orders = self.create_signed_orders(&arb_op).await?;

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

            let order_args = OrderArgs {
                price: price.to_string().parse::<f64>()
                    .context("Failed to parse price to f64")?,
                size: size.to_string().parse::<f64>()
                    .context("Failed to parse size to f64")?,
                side: Side::Buy,
                token_id: edge.asset_id.clone(),
            };

            let order = self.clob_client
                .create_order(order_args.clone())
                .await
                .context("Failed to create order")?;

            let signed_order = self.clob_client
                .sign(&self.signer, order.clone())
                .await
                .context("Failed to sign order")?;

            let order_hash = self.calculate_order_hash(&order);

            signed_orders.push(SignedOrder {
                order,
                order_args,
                order_hash,
                created_at: Instant::now(),
            });
        }

        Ok(signed_orders)
    }

    #[inline]
    async fn submit_orders_parallel(
        &self,
        signed_orders: &[SignedOrder],
    ) -> Result<Vec<OrderResult>> {
        let client = Arc::clone(&self.clob_client);

        let results = join_all(
            signed_orders.iter().map(|signed_order| {
                let client = Arc::clone(&client);
                let signed_order = signed_order.clone();

                async move {
                    Self::submit_single_order(client, signed_order).await
                }
            })
        ).await;

        let order_results: Result<Vec<_>> = results.into_iter().collect();
        order_results.context("Some order submissions failed")
    }

    #[inline]
    async fn submit_single_order(
        client: Arc<Client>,
        signed_order: SignedOrder,
    ) -> Result<OrderResult> {
        let asset_id = signed_order.order_args.token_id.clone();
        let order = signed_order.order.clone();

        let order_type = match client.clob_config.order_type.as_str() {
            "FOK" => OrderType::Fok,
            "FAK" => OrderType::Fak,
            "GTC" => OrderType::Gtc,
            "GTD" => OrderType::Gtd,
            _ => OrderType::Gtc,
        };

        let result = tokio::time::timeout(
            Duration::from_secs(client.clob_config.http_timeout_secs),
            client.post_order(signed_order.order.clone(), order_type.clone()),
        ).await;

        match result {
            Ok(Ok(response)) => {
                debug!("✅ Order submitted: {:?}, response: {:?}", asset_id, response);

                Ok(OrderResult {
                    asset_id,
                    success: true,
                    order_id: Some(response.order_id),
                    error: None,
                })
            }
            Ok(Err(e)) => {
                error!("❌ Order submission failed for {}: {:?}", asset_id, e);
                Ok(OrderResult {
                    asset_id,
                    success: false,
                    order_id: None,
                    error: Some(e.to_string()),
                })
            }
            Err(_) => {
                warn!("⏱️  Order submission timed out for {}", asset_id);
                Ok(OrderResult {
                    asset_id,
                    success: false,
                    order_id: None,
                    error: Some("Timeout".to_string()),
                })
            }
        }
    }

    #[inline]
    fn calculate_order_hash(&self, order: &Order) -> String {
        use sha2::Digest;
        use sha2::Sha256;

        let mut hasher = Sha256::new();

        hasher.update(order.token_id.as_bytes());
        hasher.update(order.maker_amount.as_bytes());
        hasher.update(order.taker_amount.as_bytes());
        hasher.update(order.expiration.as_bytes());
        hasher.update(order.nonce.as_bytes());

        let result = hasher.finalize();

        hex::encode(result)
    }

    #[inline]
    pub async fn cancel_all_orders(&self) -> Result<()> {
        info!("🗑️  Canceling all open orders...");

        let open_orders = self.clob_client
            .get_open_orders()
            .await
            .context("Failed to fetch open orders")?;

        if open_orders.is_empty() {
            info!("No open orders to cancel");
            return Ok(());
        }

        let client = Arc::clone(&self.clob_client);

        let results = join_all(
            open_orders.iter().map(|order| {
                let client = Arc::clone(&client);
                let order_id = order.id.clone();

                async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(5),
                        client.cancel_order(order_id)
                    ).await;

                    match result {
                        Ok(Ok(_)) => {
                            info!("✅ Cancelled order: {}", order_id);
                            Ok(true)
                        }
                        Ok(Err(e)) => {
                            error!("❌ Failed to cancel order {}: {:?}", order_id, e);
                            Ok(false)
                        }
                        Err(_) => {
                            warn!("⏱️  Cancel order timed out: {}", order_id);
                            Ok(false)
                        }
                    }
                }
            })
        ).await;

        let cancel_count = results.into_iter()
            .filter_map(|r| r.ok())
            .filter(|&success| success)
            .count();

        info!("✅ Cancelled {}/{} orders", cancel_count, open_orders.len());

        Ok(())
    }

    #[inline]
    pub async fn get_balances(&self) -> Result<Vec<(String, Decimal)>> {
        let balances = self.clob_client
            .get_balances()
            .await
            .context("Failed to fetch balances")?;

        Ok(balances.into_iter()
            .filter_map(|balance| {
                Some((
                    balance.token_id.clone(),
                    balance.amount.parse::<Decimal>().ok()?,
                ))
            })
            .collect()
        )
    }
}
