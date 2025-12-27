use reqwest::Client;
use serde::{Deserialize, Serialize};
use anyhow::{Result, Context};
use tracing::{info, debug};
use std::sync::Arc;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub id: String,
    pub question: String,
    pub slug: String,
    pub market: String,
    pub description: Option<String>,
    pub outcomes: Vec<Outcome>,
    pub assets_ids: Vec<String>,
    pub ticker_tag: Option<String>,
    pub end_date: Option<String>,
    pub volume_24h: Option<u64>,
    pub traders_24h: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub name: String,
    pub token_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaResponse {
    pub data: Vec<Market>,
    pub next_cursor: Option<String>,
}

pub struct GammaClient {
    client: Arc<Client>,
    base_url: String,
    markets_cache: Arc<tokio::sync::RwLock<HashMap<String, Market>>>,
}

impl GammaClient {
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client: Arc::new(client),
            base_url: base_url.to_string(),
            markets_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    pub async fn fetch_markets(
        &self,
        markets_config: &crate::utils::MarketsConfig,
    ) -> Result<Vec<Market>> {
        info!("📊 Fetching markets from Gamma API...");

        let mut all_markets = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            let url = if let Some(c) = &cursor {
                format!("{}/markets?cursor={}&limit=100", self.base_url, c)
            } else {
                format!("{}/markets?limit=100", self.base_url)
            };

            debug!("Fetching markets page {} from {}", page, url);

            let response = self.client
                .get(&url)
                .send()
                .await
                .context("Failed to fetch markets from Gamma API")?;

            if !response.status().is_success() {
                anyhow::bail!("Gamma API returned status: {}", response.status());
            }

            let gamma_response: GammaResponse = response
                .json()
                .await
                .context("Failed to parse Gamma API response")?;

            let filtered_markets: Vec<Market> = gamma_response.data
                .into_iter()
                .filter(|market| self.should_include_market(market, markets_config))
                .collect();

            let count = filtered_markets.len();
            let is_empty = filtered_markets.is_empty();
            all_markets.extend(filtered_markets);

            debug!("Fetched {} markets on page {} (total: {})", count, page, all_markets.len());

            cursor = gamma_response.next_cursor.clone();

            if cursor.is_none() || is_empty {
                break;
            }

            page += 1;

            if all_markets.len() >= 5000 {
                info!("Reached maximum market limit (5000)");
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        info!("✅ Fetched {} markets from Gamma API", all_markets.len());

        let mut cache = self.markets_cache.write().await;
        for market in &all_markets {
            cache.insert(market.market.clone(), market.clone());
        }
        drop(cache);

        Ok(all_markets)
    }

    #[inline]
    fn should_include_market(&self, market: &Market, config: &crate::utils::MarketsConfig) -> bool {
        if config.blacklisted_markets.iter().any(|blacklist| {
            market.market.contains(blacklist) || market.slug.contains(blacklist)
        }) {
            return false;
        }

        if !config.prioritize_categories.is_empty() {
            if let Some(ticker_tag) = &market.ticker_tag {
                let ticker_lower = ticker_tag.to_lowercase();

                let is_prioritized = config.prioritize_categories.iter().any(|category| {
                    ticker_lower.contains(&category.to_lowercase())
                });

                if !is_prioritized {
                    debug!("Skipping non-prioritized market: {} ({})", market.question, ticker_tag);
                    return false;
                }
            }
        }

        if let Some(volume) = market.volume_24h {
            if volume < config.min_volume_24h {
                debug!(
                    "Skipping low-volume market: {} (${} < ${})",
                    market.question,
                    volume,
                    config.min_volume_24h
                );
                return false;
            }
        }

        if let Some(traders) = market.traders_24h {
            if traders < config.min_traders_24h {
                debug!(
                    "Skipping low-trader market: {} ({} traders < {})",
                    market.question,
                    traders,
                    config.min_traders_24h
                );
                return false;
            }
        }

        if market.outcomes.is_empty() {
            debug!("Skipping market with no outcomes: {}", market.question);
            return false;
        }

        if market.outcomes.len() < 2 {
            debug!("Skipping single-outcome market: {}", market.question);
            return false;
        }

        if market.assets_ids.len() != market.outcomes.len() {
            debug!(
                "Skipping market with mismatched assets/outcomes: {} ({} assets, {} outcomes)",
                market.question,
                market.assets_ids.len(),
                market.outcomes.len()
            );
            return false;
        }

        true
    }

    #[inline]
    pub async fn get_market(&self, market_id: &str) -> Option<Market> {
        let cache = self.markets_cache.read().await;
        cache.get(market_id).cloned()
    }

    #[inline]
    pub async fn invalidate_cache(&self) {
        let mut cache = self.markets_cache.write().await;
        cache.clear();
        info!("🗑️  Gamma market cache invalidated");
    }

    #[inline]
    pub fn filter_markets_by_category(
        &self,
        markets: &[Market],
        category: &str,
    ) -> Vec<Market> {
        markets
            .iter()
            .filter(|market| {
                market.ticker_tag
                    .as_ref()
                    .map(|tag| tag.to_lowercase().contains(&category.to_lowercase()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    #[inline]
    pub fn get_binary_markets(&self, markets: &[Market]) -> Vec<Market> {
        markets
            .iter()
            .filter(|market| market.outcomes.len() == 2)
            .cloned()
            .collect()
    }

    #[inline]
    pub fn get_multi_outcome_markets(&self, markets: &[Market]) -> Vec<Market> {
        markets
            .iter()
            .filter(|market| market.outcomes.len() > 2)
            .cloned()
            .collect()
    }

    #[inline]
    pub fn get_markets_by_volume(&self, markets: &[Market], min_volume: u64) -> Vec<Market> {
        markets
            .iter()
            .filter(|market| {
                market.volume_24h
                    .map_or(false, |vol| vol >= min_volume)
            })
            .cloned()
            .collect()
    }
}
