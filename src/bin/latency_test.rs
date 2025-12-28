//! Polymarket Order Latency Tester
//!
//! Tests the alleged 500ms taker delay by placing small real orders
//! and measuring end-to-end latency at each step.
//!
//! Usage:
//!   cargo run --release --bin latency_test
//!
//! WARNING: This places REAL orders with REAL money (small amounts)

use anyhow::{Context, Result};
use hfptm::{
    gamma_api::GammaClient,
    utils::Config,
};
use polymarket_client_sdk::auth::{state::Authenticated, Normal};
use polymarket_client_sdk::clob::{
    types::{OrderType, Side, SignedOrder as SdkSignedOrder, PostOrderResponse},
    Client, Config as ClobConfig,
};
use alloy::signers::{local::PrivateKeySigner, Signer};
use rust_decimal::Decimal;
use std::time::Instant;
use tracing::{info, warn, error};

/// Latency measurement results
#[derive(Debug, Clone)]
struct LatencyMeasurement {
    test_name: String,
    order_type: String,
    token_id: String,
    price: Decimal,
    size: Decimal,
    
    // Timing breakdown (microseconds)
    order_build_us: u64,
    order_sign_us: u64,
    api_submit_us: u64,
    total_us: u64,
    
    // Result
    success: bool,
    error_message: Option<String>,
}

impl std::fmt::Display for LatencyMeasurement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} | {} | Build: {:.2}ms | Sign: {:.2}ms | API: {:.2}ms | Total: {:.2}ms | {}",
            self.test_name,
            self.order_type,
            self.order_build_us as f64 / 1000.0,
            self.order_sign_us as f64 / 1000.0,
            self.api_submit_us as f64 / 1000.0,
            self.total_us as f64 / 1000.0,
            if self.success { "✅" } else { "❌" }
        )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Banner
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  ⚡ POLYMARKET ORDER LATENCY TESTER                           ║");
    println!("║  Testing the 500ms Taker Delay Myth                           ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");
    println!();

    // Load config
    let config = Config::load()?;

    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter("info,hfptm=debug")
        .with_target(false)
        .compact()
        .init();

    // Verify we're NOT in simulation mode for this test
    if matches!(config.trading.trading_mode, hfptm::utils::TradingMode::Simulation) {
        warn!("⚠️  Config is in SIMULATION mode. Switching to LIVE for latency testing.");
        warn!("⚠️  This will place REAL orders with REAL money (small amounts: $1-2)");
    }

    println!();
    println!("⚠️  WARNING: This will place REAL orders!");
    println!("⚠️  Order size: $1-2 per test");
    println!("⚠️  Press Ctrl+C within 5 seconds to abort...");
    println!();
    
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Initialize CLOB client
    info!("🔐 Initializing authenticated CLOB client...");
    
    let private_key = &config.credentials.private_key;
    let mut signer: PrivateKeySigner = private_key
        .parse()
        .context("Failed to parse private key")?;
    signer.set_chain_id(Some(137)); // Polygon

    let clob_config = ClobConfig::default();
    let unauth_client = Client::new(&config.server.rest_url, clob_config)?;
    
    let clob_client: Client<Authenticated<Normal>> = unauth_client
        .authentication_builder(&signer)
        .authenticate()
        .await
        .context("Failed to authenticate CLOB client")?;

    info!("✅ CLOB client authenticated");

    // Check balance
    let balance = clob_client
        .balance_allowance(&polymarket_client_sdk::clob::types::BalanceAllowanceRequest::default())
        .await?;
    info!("💰 Current balance: ${:.2}", balance.balance);

    if balance.balance < Decimal::from(5) {
        warn!("⚠️  Balance appears low (${:.2}). This might be a proxy wallet issue.", balance.balance);
        warn!("⚠️  Proceeding anyway - orders may fail if truly insufficient.");
    }

    // Fetch a short-window market to test on
    info!("🔍 Finding a short-window market for testing...");
    let gamma_client = GammaClient::new(&config.server.gamma_url);
    let short_window_markets = gamma_client.fetch_short_window_markets(&config.markets).await?;
    
    if short_window_markets.is_empty() {
        error!("❌ No short-window markets found. Try again during market hours.");
        return Ok(());
    }

    let test_market = &short_window_markets[0];
    info!("📊 Test market: {}", test_market.question);
    info!("   Assets: {:?}", test_market.assets_ids);

    if test_market.assets_ids.len() < 2 {
        error!("❌ Market doesn't have YES/NO tokens");
        return Ok(());
    }

    let yes_token = &test_market.assets_ids[0];
    let no_token = &test_market.assets_ids[1];

    // Test parameters
    let test_size = Decimal::from(1); // $1 per test
    let test_price = Decimal::from_str_exact("0.50")?; // 50 cents (neutral)
    
    let mut measurements: Vec<LatencyMeasurement> = Vec::new();

    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  RUNNING LATENCY TESTS");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // Test 1: FOK (Fill or Kill) - Taker order
    info!("🧪 Test 1: FOK Order (Taker) on YES token");
    let measurement = test_order_latency(
        &clob_client,
        &signer,
        "FOK_YES",
        yes_token,
        test_price,
        test_size,
        OrderType::FOK,
    ).await;
    println!("   {}", measurement);
    measurements.push(measurement);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Test 2: FOK on NO token
    info!("🧪 Test 2: FOK Order (Taker) on NO token");
    let measurement = test_order_latency(
        &clob_client,
        &signer,
        "FOK_NO",
        no_token,
        test_price,
        test_size,
        OrderType::FOK,
    ).await;
    println!("   {}", measurement);
    measurements.push(measurement);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Test 3: GTC (Good Till Cancelled) - Maker order
    info!("🧪 Test 3: GTC Order (Maker) on YES token");
    let measurement = test_order_latency(
        &clob_client,
        &signer,
        "GTC_YES",
        yes_token,
        Decimal::from_str_exact("0.10")?, // Low price = likely won't fill (maker)
        test_size,
        OrderType::GTC,
    ).await;
    println!("   {}", measurement);
    measurements.push(measurement);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Test 4: GTC on NO token
    info!("🧪 Test 4: GTC Order (Maker) on NO token");
    let measurement = test_order_latency(
        &clob_client,
        &signer,
        "GTC_NO",
        no_token,
        Decimal::from_str_exact("0.10")?,
        test_size,
        OrderType::GTC,
    ).await;
    println!("   {}", measurement);
    measurements.push(measurement);

    // Cancel all GTC orders we placed
    info!("🗑️  Cancelling test orders...");
    let _ = clob_client.cancel_all_orders().await;

    // Run multiple FOK tests for statistical significance
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  RUNNING 10x FOK TESTS FOR STATISTICS");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    for i in 0..10 {
        let token = if i % 2 == 0 { yes_token } else { no_token };
        let test_name = format!("FOK_BATCH_{}", i + 1);
        
        let measurement = test_order_latency(
            &clob_client,
            &signer,
            &test_name,
            token,
            test_price,
            test_size,
            OrderType::FOK,
        ).await;
        
        println!("   {}", measurement);
        measurements.push(measurement);
        
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // Summary
    println!();
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  📊 LATENCY TEST RESULTS                                      ║");
    println!("╠═══════════════════════════════════════════════════════════════╣");
    
    let fok_measurements: Vec<&LatencyMeasurement> = measurements
        .iter()
        .filter(|m| m.order_type == "FOK")
        .collect();
    
    let gtc_measurements: Vec<&LatencyMeasurement> = measurements
        .iter()
        .filter(|m| m.order_type == "GTC")
        .collect();

    if !fok_measurements.is_empty() {
        let avg_fok_api = fok_measurements.iter().map(|m| m.api_submit_us).sum::<u64>() 
            / fok_measurements.len() as u64;
        let avg_fok_total = fok_measurements.iter().map(|m| m.total_us).sum::<u64>()
            / fok_measurements.len() as u64;
        let min_fok_api = fok_measurements.iter().map(|m| m.api_submit_us).min().unwrap_or(0);
        let max_fok_api = fok_measurements.iter().map(|m| m.api_submit_us).max().unwrap_or(0);
        
        println!("║  FOK (Taker) Orders:                                          ║");
        println!("║    API Submit Avg: {:.2}ms                                      ", avg_fok_api as f64 / 1000.0);
        println!("║    API Submit Min: {:.2}ms                                      ", min_fok_api as f64 / 1000.0);
        println!("║    API Submit Max: {:.2}ms                                      ", max_fok_api as f64 / 1000.0);
        println!("║    Total E2E Avg:  {:.2}ms                                      ", avg_fok_total as f64 / 1000.0);
    }

    if !gtc_measurements.is_empty() {
        let avg_gtc_api = gtc_measurements.iter().map(|m| m.api_submit_us).sum::<u64>()
            / gtc_measurements.len() as u64;
        let avg_gtc_total = gtc_measurements.iter().map(|m| m.total_us).sum::<u64>()
            / gtc_measurements.len() as u64;
        
        println!("║  GTC (Maker) Orders:                                          ║");
        println!("║    API Submit Avg: {:.2}ms                                      ", avg_gtc_api as f64 / 1000.0);
        println!("║    Total E2E Avg:  {:.2}ms                                      ", avg_gtc_total as f64 / 1000.0);
    }

    // Verdict on 500ms myth
    let avg_fok_api = fok_measurements.iter().map(|m| m.api_submit_us).sum::<u64>()
        / fok_measurements.len().max(1) as u64;
    
    println!("╠═══════════════════════════════════════════════════════════════╣");
    if avg_fok_api > 400_000 {
        println!("║  🚨 VERDICT: 500ms taker delay CONFIRMED!                     ║");
        println!("║     Average FOK API time: {:.0}ms                              ", avg_fok_api as f64 / 1000.0);
    } else if avg_fok_api > 200_000 {
        println!("║  ⚠️  VERDICT: Significant taker delay detected                ║");
        println!("║     Average FOK API time: {:.0}ms                              ", avg_fok_api as f64 / 1000.0);
    } else {
        println!("║  ✅ VERDICT: No significant taker delay detected              ║");
        println!("║     Average FOK API time: {:.0}ms                              ", avg_fok_api as f64 / 1000.0);
    }
    println!("╚═══════════════════════════════════════════════════════════════╝");

    // Final balance
    let final_balance = clob_client
        .balance_allowance(&polymarket_client_sdk::clob::types::BalanceAllowanceRequest::default())
        .await?;
    info!("💰 Final balance: ${:.2} (cost: ${:.2})", 
        final_balance.balance, 
        balance.balance - final_balance.balance);

    Ok(())
}

async fn test_order_latency(
    client: &Client<Authenticated<Normal>>,
    signer: &PrivateKeySigner,
    test_name: &str,
    token_id: &str,
    price: Decimal,
    size: Decimal,
    order_type: OrderType,
) -> LatencyMeasurement {
    let total_start = Instant::now();
    
    // Step 1: Build order
    let build_start = Instant::now();
    let signable_order = match client
        .limit_order()
        .token_id(token_id)
        .size(size)
        .price(price)
        .side(Side::Buy)
        .order_type(order_type.clone())
        .build()
        .await
    {
        Ok(order) => order,
        Err(e) => {
            return LatencyMeasurement {
                test_name: test_name.to_string(),
                order_type: format!("{:?}", order_type),
                token_id: token_id.to_string(),
                price,
                size,
                order_build_us: build_start.elapsed().as_micros() as u64,
                order_sign_us: 0,
                api_submit_us: 0,
                total_us: total_start.elapsed().as_micros() as u64,
                success: false,
                error_message: Some(format!("Build failed: {}", e)),
            };
        }
    };
    let build_us = build_start.elapsed().as_micros() as u64;

    // Step 2: Sign order
    let sign_start = Instant::now();
    let signed_order: SdkSignedOrder = match client.sign(signer, signable_order).await {
        Ok(signed) => signed,
        Err(e) => {
            return LatencyMeasurement {
                test_name: test_name.to_string(),
                order_type: format!("{:?}", order_type),
                token_id: token_id.to_string(),
                price,
                size,
                order_build_us: build_us,
                order_sign_us: sign_start.elapsed().as_micros() as u64,
                api_submit_us: 0,
                total_us: total_start.elapsed().as_micros() as u64,
                success: false,
                error_message: Some(format!("Sign failed: {}", e)),
            };
        }
    };
    let sign_us = sign_start.elapsed().as_micros() as u64;

    // Step 3: Submit to API (THIS IS WHERE 500ms DELAY WOULD SHOW)
    let submit_start = Instant::now();
    let response: Result<Vec<PostOrderResponse>, _> = client.post_order(signed_order).await;
    let submit_us = submit_start.elapsed().as_micros() as u64;

    let total_us = total_start.elapsed().as_micros() as u64;

    match response {
        Ok(_responses) => {
            LatencyMeasurement {
                test_name: test_name.to_string(),
                order_type: format!("{:?}", order_type),
                token_id: token_id.to_string(),
                price,
                size,
                order_build_us: build_us,
                order_sign_us: sign_us,
                api_submit_us: submit_us,
                total_us,
                success: true,
                error_message: None,
            }
        }
        Err(e) => {
            // Even if order was rejected, we still measure the API response time
            LatencyMeasurement {
                test_name: test_name.to_string(),
                order_type: format!("{:?}", order_type),
                token_id: token_id.to_string(),
                price,
                size,
                order_build_us: build_us,
                order_sign_us: sign_us,
                api_submit_us: submit_us,
                total_us,
                success: false,
                error_message: Some(e.to_string()),
            }
        }
    }
}
