#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_config_loading() {
        let result = Config::load();
        assert!(result.is_ok() || result.err().unwrap().to_string().contains("private_key"));
    }

    #[tokio::test]
    async fn test_orderbook_manager() {
        let config = create_test_config();
        let manager = OrderBookManager::new(&config).unwrap();

        let market_id = "test_market";
        let asset_id = "test_asset";
        let timestamp = 1234567890000i64;

        let bids = vec![(rust_decimal::dec!(0.48), rust_decimal::dec!(100))];
        let asks = vec![(rust_decimal::dec!(0.52), rust_decimal::dec!(100))];

        let snapshot = BookSnapshot {
            market_id: market_id.to_string(),
            asset_id: asset_id.to_string(),
            bids,
            asks,
            timestamp,
            hash: "test_hash".to_string(),
        };

        let result = manager.update_book(market_id, asset_id, &snapshot);
        assert!(result.is_ok());

        let best_ask = manager.get_best_asks_for_market(market_id);
        assert!(best_ask.is_some());
    }

    #[tokio::test]
    async fn test_arbitrage_detection() {
        let config = create_test_config();
        let mut arb_engine = ArbEngine::new(&config);
        let mut orderbook_manager = OrderBookManager::new(&config).unwrap();
        let risk_manager = RiskManager::new(&config);

        let market_id = "test_market";
        let asset_yes = "asset_yes";
        let asset_no = "asset_no";

        let yes_price = rust_decimal::dec!(0.47);
        let no_price = rust_decimal::dec!(0.48);
        let size = rust_decimal::dec!(200);

        let yes_snapshot = BookSnapshot {
            market_id: market_id.to_string(),
            asset_id: asset_yes.to_string(),
            bids: vec![(yes_price, size)],
            asks: vec![(yes_price, size)],
            timestamp: 1234567890000i64,
            hash: "hash1".to_string(),
        };

        let no_snapshot = BookSnapshot {
            market_id: market_id.to_string(),
            asset_id: asset_no.to_string(),
            bids: vec![(no_price, size)],
            asks: vec![(no_price, size)],
            timestamp: 1234567890000i64,
            hash: "hash2".to_string(),
        };

        orderbook_manager.update_book(market_id, asset_yes, &yes_snapshot).unwrap();
        orderbook_manager.update_book(market_id, asset_no, &no_snapshot).unwrap();

        let arb_op = arb_engine.detect_arbitrage(&orderbook_manager, market_id, &risk_manager).await;
        assert!(arb_op.is_ok());

        let opportunity = arb_op.unwrap();
        assert!(opportunity.is_some());

        let Some(arb) = opportunity else {
            panic!("Should detect arbitrage");
        };

        assert_eq!(arb.arb_type, ArbType::Binary);
        assert!(arb.total_edge > rust_decimal::ZERO);
    }

    #[tokio::test]
    async fn test_risk_manager() {
        let config = create_test_config();
        let mut risk_manager = RiskManager::new(&config);

        let market_id = "test_market";
        let asset_id = "test_asset";
        let outcome = "YES";
        let position_type = PositionType::Long;
        let size = rust_decimal::dec!(100);
        let price = rust_decimal::dec!(0.50);
        let cost = rust_decimal::dec!(50);

        risk_manager.add_position(
            market_id.to_string(),
            asset_id.to_string(),
            outcome.to_string(),
            position_type.clone(),
            size,
            price,
            cost,
        ).unwrap();

        let position = risk_manager.get_position(asset_id);
        assert!(position.is_some());

        let pos = position.unwrap();
        assert_eq!(pos.size, size);
        assert_eq!(pos.avg_price, price);
    }

    #[tokio::test]
    async fn test_latency_tracker() {
        let mut tracker = LatencyTracker::new();

        tracker.record(50_000_000);
        tracker.record(150_000_000);

        assert_eq!(tracker.count(), 2);
        assert_eq!(tracker.avg_latency_ns(), 100_000_000);
        assert_eq!(tracker.avg_latency_ms(), 100.0);
    }

    fn create_test_config() -> Config {
        Config {
            server: ServerConfig {
                wss_url: "wss://test.polymarket.com/ws/market".to_string(),
                rest_url: "https://test.polymarket.com".to_string(),
                gamma_url: "https://test.polymarket.com".to_string(),
                polygon_rpc_url: "https://test.polygon.com".to_string(),
            },
            credentials: CredentialsConfig {
                private_key: "0x1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                api_key: "test_key".to_string(),
                api_secret: "test_secret".to_string(),
                api_passphrase: "test_pass".to_string(),
                funder_address: "0x1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                signature_type: 2u8,
            },
            trading: TradingConfig {
                bankroll: 1000,
                max_arb_size: 100,
                min_edge: rust_decimal::dec!(0.025),
                min_liquidity: 100,
                max_order_books: 100,
                tick_size: "0.01".to_string(),
                order_type: "FOK".to_string(),
                slippage_tolerance: rust_decimal::dec!(0.01),
            },
            risk: RiskConfig {
                max_exposure_per_market: 200,
                max_exposure_per_event: 500,
                max_concurrent_arbs: 5,
                daily_loss_limit: 50,
                max_gas_gwei: 100,
                position_timeout_seconds: 86400,
                inventory_drift_threshold: rust_decimal::dec!(0.05),
            },
            markets: MarketsConfig {
                prioritize_categories: vec!["sports".to_string()],
                blacklisted_markets: vec![],
                min_volume_24h: 1000,
                min_traders_24h: 10,
                min_order_book_depth: 5,
            },
            execution: ExecutionConfig {
                max_latency_ms: 150,
                websocket_ping_interval_secs: 10,
                websocket_reconnect_delay_ms: 1000,
                max_retries: 5,
                retry_backoff_ms: 100,
                http_timeout_secs: 5,
                connection_pool_size: 10,
            },
            monitoring: MonitoringConfig {
                log_level: "info".to_string(),
                enable_dashboard: true,
                dashboard_port: 3000,
                enable_tracing: true,
                log_file: "logs/test.log".to_string(),
                metrics_retention_hours: 24,
            },
            alerts: AlertsConfig {
                enable_telegram: false,
                telegram_bot_token: "".to_string(),
                telegram_chat_id: "".to_string(),
                alert_on_trade_usd: 25,
                alert_on_error: true,
                alert_on_latency_spike: true,
                latency_spike_threshold_ms: 200,
                alert_on_pnl_drawdown: true,
                pnl_drawdown_threshold_usd: 100,
            },
            latency: LatencyConfig {
                enable_cpu_pinning: false,
                target_cpu_core: 0,
                use_jemalloc: true,
                max_orderbook_updates_per_sec: 10000,
                enable_zero_copy: true,
            },
        }
    }
}
