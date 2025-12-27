mod lib;

use std::env;
use anyhow::Result;
use tracing::{info, error};
use tokio::signal;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    println!("🚀 HFTPM - Ultra-Low-Latency Polymarket Arbitrage Bot");
    println!("📖 Version: {}", env!("CARGO_PKG_VERSION"));
    println!("🔗 GitHub: https://github.com/your-repo/HFTPM");
    println!();

    if let Err(e) = lib::run().await {
        error!("💥 Fatal error: {:?}", e);
        std::process::exit(1);
    }

    Ok(())
}
