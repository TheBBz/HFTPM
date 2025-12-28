//! Proxy Wallet Debug Tool
//!
//! Diagnoses issues with Polymarket proxy wallet authentication

use anyhow::{Context, Result};
use hfptm::utils::Config;
use polymarket_client_sdk::auth::{state::Authenticated, Normal};
use polymarket_client_sdk::clob::{Client, Config as ClobConfig};
use alloy::signers::{local::PrivateKeySigner, Signer};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  🔍 PROXY WALLET DEBUG TOOL                                   ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");
    println!();

    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .compact()
        .init();

    // Load config
    let config = Config::load()?;

    // Parse private key and get EOA address
    let private_key = &config.credentials.private_key;
    let mut signer: PrivateKeySigner = private_key
        .parse()
        .context("Failed to parse private key")?;
    signer.set_chain_id(Some(137)); // Polygon

    let eoa_address = signer.address();
    
    println!("📋 Configuration Check:");
    println!("   EOA Address (derived from private key): {}", eoa_address);
    println!("   Funder Address (from config):           {}", config.credentials.funder_address);
    println!("   Signature Type: {} (0=EOA, 1=Proxy, 2=Safe)", config.credentials.signature_type);
    println!();

    // Check if addresses match
    let funder_lower = config.credentials.funder_address.to_lowercase();
    let eoa_lower = format!("{:?}", eoa_address).to_lowercase();
    
    if funder_lower == eoa_lower {
        println!("✅ EOA and Funder addresses MATCH");
    } else {
        println!("⚠️  EOA and Funder addresses DO NOT MATCH");
        println!("   This is expected if using a proxy wallet.");
        println!("   The funder_address should be your Polymarket proxy wallet.");
    }
    println!();

    // Test authentication
    println!("🔐 Testing CLOB Authentication...");
    let clob_config = ClobConfig::default();
    let unauth_client = Client::new(&config.server.rest_url, clob_config)?;
    
    let clob_client: Client<Authenticated<Normal>> = unauth_client
        .authentication_builder(&signer)
        .authenticate()
        .await
        .context("Failed to authenticate CLOB client")?;

    println!("✅ Authentication successful");
    println!();

    // Test balance endpoint
    println!("💰 Testing Balance Endpoint...");
    match clob_client
        .balance_allowance(&polymarket_client_sdk::clob::types::BalanceAllowanceRequest::default())
        .await
    {
        Ok(balance) => {
            println!("   Balance: ${:.6}", balance.balance);
            println!("   Allowances: {:?}", balance.allowances);
        }
        Err(e) => {
            println!("❌ Balance query failed: {}", e);
        }
    }
    println!();

    // Test different signature types
    println!("🧪 Testing Signature Type Variations...");
    
    // The polymarket-client-sdk might need the proxy address set differently
    // Let's check what the authenticated client reports
    
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  DIAGNOSIS");
    println!("═══════════════════════════════════════════════════════════════");
    println!();
    
    if funder_lower != eoa_lower {
        println!("🔎 The private key generates address: {}", eoa_address);
        println!("🔎 But funder_address is set to:      {}", config.credentials.funder_address);
        println!();
        println!("   This could mean:");
        println!("   1. The private key is for a DIFFERENT wallet than your Polymarket proxy");
        println!("   2. You need to use signature_type = 0 (EOA) if this is your main wallet");
        println!("   3. Or export the correct private key from Polymarket settings");
        println!();
        println!("💡 FIX: In Polymarket website:");
        println!("   Settings → Export Private Key → Use THAT key in config.toml");
    }

    Ok(())
}
