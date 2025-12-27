use reqwest::Client;
use serde::{Deserialize, Serialize};
use anyhow::{Result, Context};
use tracing::{info, debug};
use std::sync::Arc;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInfo {
    pub id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub id: String,
    pub question: String,
    pub slug: String,
    #[serde(rename = "conditionId")]
    pub market: String,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_outcomes")]
    pub outcomes: Vec<Outcome>,
    #[serde(rename = "clobTokenIds", default, deserialize_with = "deserialize_token_ids")]
    pub assets_ids: Vec<String>,
    #[serde(rename = "category")]
    pub ticker_tag: Option<String>,
    #[serde(rename = "endDate")]
    pub end_date: Option<String>,
    #[serde(rename = "volume24hr", default)]
    pub volume_24h: Option<f64>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub closed: bool,
    #[serde(rename = "enableOrderBook", default)]
    pub enable_order_book: bool,
    /// Events this market belongs to - markets with the same event_id are related
    #[serde(default)]
    pub events: Vec<EventInfo>,
}

impl Market {
    /// Get the primary event ID for this market (used for correlation grouping)
    pub fn event_id(&self) -> Option<&str> {
        self.events.first().map(|e| e.id.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub name: String,
    pub token_id: String,
}

// Deserialize outcomes from JSON string like "[\"Yes\", \"No\"]"
fn deserialize_outcomes<'de, D>(deserializer: D) -> Result<Vec<Outcome>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    
    let s: Option<String> = Option::deserialize(deserializer)?;
    if let Some(s) = s {
        let outcome_names: Vec<String> = serde_json::from_str(&s)
            .map_err(|e| Error::custom(format!("Failed to parse outcomes: {}", e)))?;
        
        Ok(outcome_names
            .into_iter()
            .enumerate()
            .map(|(i, name)| Outcome {
                id: i.to_string(),
                name,
                token_id: String::new(),
            })
            .collect())
    } else {
        Ok(Vec::new())
    }
}

// Deserialize token IDs from JSON string like "[\"123...\", \"456...\"]"
fn deserialize_token_ids<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    
    let s: Option<String> = Option::deserialize(deserializer)?;
    if let Some(s) = s {
        serde_json::from_str(&s)
            .map_err(|e| Error::custom(format!("Failed to parse token IDs: {}", e)))
    } else {
        Ok(Vec::new())
    }
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

        // Fetch active markets with CLOB trading enabled
        let url = format!("{}/markets?active=true&closed=false&limit=1000", self.base_url);

        debug!("Fetching markets from {}", url);

        let response = self.client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch markets from Gamma API")?;

        if !response.status().is_success() {
            anyhow::bail!("Gamma API returned status: {}", response.status());
        }

        let markets: Vec<Market> = response
            .json()
            .await
            .context("Failed to parse Gamma API response")?;

        // Note: We don't limit here - let the caller decide how many to use
        // The config has max_order_books in trading section for that
        let filtered_markets: Vec<Market> = markets
            .into_iter()
            .filter(|market| self.should_include_market(market, markets_config))
            .collect();

        info!("✅ Fetched {} active markets", filtered_markets.len());

        // Cache markets for quick lookup
        let mut cache = self.markets_cache.write().await;
        for market in &filtered_markets {
            cache.insert(market.market.clone(), market.clone());
        }
        drop(cache);

        Ok(filtered_markets)
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
            if volume < config.min_volume_24h as f64 {
                debug!(
                    "Skipping low-volume market: {} (${} < ${})",
                    market.question,
                    volume,
                    config.min_volume_24h
                );
                return false;
            }
        }

        // Skip markets without order book enabled
        if !market.enable_order_book {
            debug!("Skipping market without order book: {}", market.question);
            return false;
        }

        // Skip closed/inactive markets
        if market.closed || !market.active {
            debug!("Skipping closed/inactive market: {}", market.question);
            return false;
        }

        if market.outcomes.is_empty() {
            debug!("Skipping market with no outcomes: {}", market.question);
            return false;
        }

        if market.outcomes.len() < 2 {
            debug!("Skipping single-outcome market: {}", market.question);
            return false;
        }

        if market.assets_ids.is_empty() {
            debug!("Skipping market with no asset IDs: {}", market.question);
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
                    .map_or(false, |vol| vol >= min_volume as f64)
            })
            .cloned()
            .collect()
    }
}
