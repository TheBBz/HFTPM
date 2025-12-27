use super::types::{WsMessage, BookSnapshot};
use crate::utils::{Config, LatencyTracker, ScopedTimer};
use crate::orderbook::OrderBookManager;
use crate::arb_engine::ArbEngine;
use crate::risk::RiskManager;
use crate::executor::OrderExecutor;
use crate::monitoring::Monitor;
use crate::gamma_api::Market;

use tokio_tungstenite::tungstenite::protocol::Message;
use futures::{StreamExt, SinkExt};
use tokio::sync::mpsc;
use anyhow::{Result, Context};
use std::sync::Arc;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tracing::{info, warn, error, debug, instrument};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const RECONNECT_DELAY: Duration = Duration::from_millis(1000);
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

pub struct WebSocketClient {
    config: Arc<Config>,
    markets: Arc<Vec<Market>>,
    latency_tracker: LatencyTracker,
    subscribed_markets: HashSet<String>,
    simulation_executor: Option<Arc<crate::executor::SimulationExecutor>>,
}

impl WebSocketClient {
    pub async fn new(config: &Config, markets: &[Market]) -> Result<Self> {
        let simulation_executor = if config.trading.trading_mode == crate::utils::TradingMode::Simulation {
            Some(Arc::new(crate::executor::SimulationExecutor::new(config)))
        } else {
            None
        };

        Ok(Self {
            config: Arc::new(config.clone()),
            markets: Arc::new(markets.to_vec()),
            latency_tracker: LatencyTracker::new(),
            subscribed_markets: HashSet::new(),
            simulation_executor,
        })
    }

    pub async fn subscribe_all_markets(&mut self) -> Result<()> {
        // Just mark markets as needing subscription - actual subscription happens in connect_and_run
        for market in self.markets.iter().take(self.config.trading.max_order_books) {
            self.subscribed_markets.insert(market.id.clone());
        }
        info!("📡 Prepared {} markets for subscription", self.subscribed_markets.len());
        Ok(())
    }

    fn build_subscription_message(&self) -> String {
        let asset_ids: Vec<String> = self.markets
            .iter()
            .take(self.config.trading.max_order_books)
            .flat_map(|m| m.assets_ids.clone())
            .collect();

        serde_json::json!({
            "assets_ids": asset_ids,
            "type": "market"
        }).to_string()
    }

    #[instrument(skip(self, orderbook_manager, arb_engine, risk_manager, executor, monitor))]
    pub async fn run(
        &mut self,
        orderbook_manager: &mut OrderBookManager,
        arb_engine: &mut ArbEngine,
        risk_manager: &mut RiskManager,
        executor: &OrderExecutor,
        monitor: &mut Monitor,
    ) -> Result<()> {
        info!("🚀 Starting WebSocket connection to {}", self.config.server.wss_url);

        loop {
            match self.connect_and_run(
                orderbook_manager,
                arb_engine,
                risk_manager,
                executor,
                monitor,
            ).await {
                Ok(_) => {
                    warn!("WebSocket closed unexpectedly, reconnecting...");
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
                Err(e) => {
                    error!("WebSocket error: {:?}, reconnecting in {:?}...", e, RECONNECT_DELAY);
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
            }
        }
    }

    async fn connect_and_run(
        &mut self,
        orderbook_manager: &mut OrderBookManager,
        arb_engine: &mut ArbEngine,
        risk_manager: &mut RiskManager,
        executor: &OrderExecutor,
        monitor: &mut Monitor,
    ) -> Result<()> {
        let url = &self.config.server.wss_url;
        let (ws_stream, _) = tokio_tungstenite::connect_async(url).await
            .context("Failed to connect to WebSocket")?;

        info!("✅ WebSocket connected to {}", url);

        let (mut write, mut read) = ws_stream.split();

        // Send subscription message immediately after connecting
        let subscribe_msg = self.build_subscription_message();
        info!("📡 Sending subscription for {} asset IDs...", 
            self.markets.iter().take(self.config.trading.max_order_books).flat_map(|m| m.assets_ids.clone()).count());
        write.send(Message::Text(subscribe_msg)).await
            .context("Failed to send subscription message")?;
        info!("✅ Subscription message sent");

        // Channel for sending messages to the write half
        let (tx, mut rx) = mpsc::channel::<Message>(100);

        // Clone tx for ping task
        let ping_tx = tx.clone();

        // Spawn ping task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PING_INTERVAL);
            loop {
                interval.tick().await;
                if ping_tx.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        });

        // Spawn write task
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write.send(msg).await {
                    error!("Failed to send WebSocket message: {:?}", e);
                    break;
                }
            }
        });

        let mut start_time = Instant::now();
        let mut message_count = 0u64;
        let mut last_stats = Instant::now();

        while let Some(message) = read.next().await {
            let message = message.context("Failed to read WebSocket message")?;

            match message {
                Message::Text(text) => {
                    let _timer = ScopedTimer::new("ws_message_processing", None);

                    let text_bytes = text.as_bytes();

                    if text_bytes.len() > MAX_MESSAGE_SIZE {
                        warn!("Message too large: {} bytes", text_bytes.len());
                        continue;
                    }

                    // Try to parse as array first, then fall back to single message
                    let messages: Vec<WsMessage> = if text.trim_start().starts_with('[') {
                        match serde_json::from_str::<Vec<WsMessage>>(&text) {
                            Ok(msgs) => msgs,
                            Err(e) => {
                                warn!("Failed to parse message array: {} | Sample: {}", e, &text[..text.len().min(300)]);
                                continue;
                            }
                        }
                    } else {
                        match serde_json::from_str::<WsMessage>(&text) {
                            Ok(msg) => vec![msg],
                            Err(e) => {
                                warn!("Failed to parse message: {} | Sample: {}", e, &text[..text.len().min(300)]);
                                continue;
                            }
                        }
                    };

                    for ws_msg in messages {
                        message_count += 1;

                        if ws_msg.is_book_snapshot() {
                            debug!("📖 Book snapshot for market: {}", ws_msg.market);
                            self.handle_book_snapshot(
                                &ws_msg,
                                orderbook_manager,
                                arb_engine,
                                risk_manager,
                                executor,
                                monitor,
                            ).await?;
                        } else if ws_msg.is_price_change() {
                            debug!("💹 Price change for market: {}", ws_msg.market);
                            self.handle_price_change(
                                &ws_msg,
                                orderbook_manager,
                                arb_engine,
                                risk_manager,
                                executor,
                                monitor,
                            ).await?;
                        } else {
                            debug!("❓ Unknown message type for market: {}", ws_msg.market);
                        }
                    }

                    let elapsed = start_time.elapsed();
                    if elapsed.as_secs() >= 1 {
                        let msgs_per_sec = message_count as f64 / elapsed.as_secs_f64();
                        if msgs_per_sec > self.config.latency.max_orderbook_updates_per_sec as f64 {
                            warn!(
                                "⚠️  High message rate: {:.2} msg/s (limit: {})",
                                msgs_per_sec,
                                self.config.latency.max_orderbook_updates_per_sec
                            );
                        }

                        if last_stats.elapsed().as_secs() >= 60 {
                            info!(
                                "📊 WebSocket stats: {:.2} msg/s, avg latency: {:.2}ms",
                                msgs_per_sec,
                                self.latency_tracker.avg_latency_ms()
                            );
                            last_stats = Instant::now();
                            message_count = 0;
                            start_time = Instant::now();
                        }
                    }
                }
                Message::Ping(data) => {
                    let _ = tx.send(Message::Pong(data)).await;
                }
                Message::Pong(_) => {
                    debug!("Received pong");
                }
                Message::Close(frame) => {
                    info!("WebSocket closed: {:?}", frame);
                    return Ok(());
                }
                _ => {}
            }
        }

        Ok(())
    }

    #[inline]
    #[instrument(skip(self, orderbook_manager, arb_engine, risk_manager, executor, monitor))]
    async fn handle_book_snapshot(
        &self,
        ws_msg: &WsMessage,
        orderbook_manager: &mut OrderBookManager,
        arb_engine: &mut ArbEngine,
        risk_manager: &mut RiskManager,
        executor: &OrderExecutor,
        monitor: &mut Monitor,
    ) -> Result<()> {
        let _timer = ScopedTimer::new("book_snapshot", None);

        let market_id = ws_msg.market.clone();
        let asset_id = ws_msg.asset_id.clone();
        let timestamp = ws_msg.parse_timestamp();

        let bids = ws_msg.bids
            .as_ref()
            .map(|bids| bids.iter().filter_map(|o| {
                o.price.parse::<rust_decimal::Decimal>().ok()
                    .map(|p| (p, o.size.parse::<rust_decimal::Decimal>().unwrap_or(rust_decimal::Decimal::ZERO)))
            }).collect())
            .unwrap_or_default();

        let asks = ws_msg.asks
            .as_ref()
            .map(|asks| asks.iter().filter_map(|o| {
                o.price.parse::<rust_decimal::Decimal>().ok()
                    .map(|p| (p, o.size.parse::<rust_decimal::Decimal>().unwrap_or(rust_decimal::Decimal::ZERO)))
            }).collect())
            .unwrap_or_default();

        let book = BookSnapshot {
            asset_id: asset_id.clone(),
            market: market_id.clone(),
            bids,
            asks,
            timestamp,
            hash: ws_msg.hash.clone().unwrap_or_default(),
        };

        // Try to update book, skip if market not found
        match orderbook_manager.update_book(&market_id, &asset_id, &book) {
            Ok(_) => {},
            Err(e) => {
                debug!("⏭️  Skipping book update for unknown market {}: {}", market_id, e);
                return Ok(());
            }
        }

        if let Some(arb_op) = arb_engine.detect_arbitrage(
            orderbook_manager,
            &market_id,
            risk_manager,
        )? {
            monitor.record_arbitrage_detected(&arb_op).await;

            // Check quality threshold before executing
            if arb_engine.should_execute_opportunity(&arb_op) {
                self.execute_arbitrage(
                    &arb_op,
                    risk_manager,
                    executor,
                    monitor,
                ).await?;
            } else {
                debug!("⏭️  Skipping low-quality arbitrage");
            }
        }

        Ok(())
    }

    #[inline]
    #[instrument(skip(self, orderbook_manager, arb_engine, risk_manager, executor, monitor))]
    async fn handle_price_change(
        &self,
        ws_msg: &WsMessage,
        orderbook_manager: &mut OrderBookManager,
        arb_engine: &mut ArbEngine,
        risk_manager: &mut RiskManager,
        executor: &OrderExecutor,
        monitor: &mut Monitor,
    ) -> Result<()> {
        let _timer = ScopedTimer::new("price_change", None);

        let market_id = ws_msg.market.clone();

        if let Some(price_changes) = &ws_msg.price_changes {
            for change in price_changes {
                let price = change.price.parse::<rust_decimal::Decimal>()
                    .context("Failed to parse price")?;

                let size = change.size.parse::<rust_decimal::Decimal>()
                    .context("Failed to parse size")?;

                // Try to update price, skip if market not found
                match orderbook_manager.update_price(
                    &market_id,
                    &change.asset_id,
                    price,
                    size,
                    change.side.as_str(),
                ) {
                    Ok(_) => {},
                    Err(e) => {
                        debug!("⏭️  Skipping price update for unknown market {}: {}", market_id, e);
                        continue;
                    }
                }
            }
        }

        if let Some(arb_op) = arb_engine.detect_arbitrage(
            orderbook_manager,
            &market_id,
            risk_manager,
        )? {
            monitor.record_arbitrage_detected(&arb_op).await;

            // Check quality threshold before executing
            if arb_engine.should_execute_opportunity(&arb_op) {
                self.execute_arbitrage(
                    &arb_op,
                    risk_manager,
                    executor,
                    monitor,
                ).await?;
            } else {
                debug!("⏭️  Skipping low-quality arbitrage");
            }
        }

        Ok(())
    }

    #[inline]
    async fn execute_arbitrage(
        &self,
        arb_op: &crate::arb_engine::ArbitrageOpportunity,
        risk_manager: &mut RiskManager,
        executor: &OrderExecutor,
        monitor: &mut Monitor,
    ) -> Result<()> {
        let _timer = ScopedTimer::new("arb_execution", None);

        if !risk_manager.can_execute_arbitrage(arb_op)? {
            debug!("⚠️  Risk manager rejected arbitrage: {:?}", arb_op);
            return Ok(());
        }

        let execution_start = Instant::now();

        // Execute based on trading mode
        let result = if self.config.trading.trading_mode == crate::utils::TradingMode::Simulation {
            self.simulation_executor.as_ref().unwrap().simulate_arbitrage(arb_op).await
        } else {
            executor.execute_arbitrage(arb_op).await
        };

        match result {
            Ok(exec_result) => {
                let execution_time = execution_start.elapsed();

                risk_manager.record_arbitrage_execution(arb_op, &exec_result)?;

                monitor.record_arbitrage_executed(arb_op, &exec_result, execution_time).await;

                if execution_time.as_millis() as u64 > self.config.execution.max_latency_ms {
                    monitor.alert_latency_spike(
                        execution_time.as_millis() as u64,
                        self.config.alerts.latency_spike_threshold_ms,
                    ).await;
                }

                let mode_indicator = if self.config.trading.trading_mode == crate::utils::TradingMode::Simulation {
                    "[SIM]"
                } else {
                    "[LIVE]"
                };

                info!(
                    "✅ {} Arbitrage executed in {:.2}ms: {}",
                    mode_indicator,
                    execution_time.as_secs_f64() * 1000.0,
                    arb_op
                );
            }
            Err(e) => {
                error!("❌ Arbitrage execution failed: {:?}", e);
                monitor.alert_error(&format!("Arbitrage execution failed: {:?}", e)).await;
            }
        }

        Ok(())
    }
}
