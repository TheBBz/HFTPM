use anyhow::Result;
use std::env;
use tracing::error;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    println!("🚀 HFTPM - Ultra-Low-Latency Polymarket Arbitrage Bot");
    println!("📖 Version: {}", env!("CARGO_PKG_VERSION"));
    println!("🔗 GitHub: https://github.com/your-repo/HFTPM");
    println!();

    if let Err(e) = hfptm::run().await {
        error!("💥 Fatal error: {:?}", e);
        std::process::exit(1);
    }

    Ok(())
}
