use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

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
    #[serde(
        rename = "clobTokenIds",
        default,
        deserialize_with = "deserialize_token_ids"
    )]
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

// ============================================================================
// Short-Window (15m up/down) Market Detection
// ============================================================================
//
// These markets are short-duration binary price prediction markets (e.g., "Will BTC
// be above $X at 3:15 PM?"). They resolve quickly and are ideal for low-risk MM.
//
// Detection criteria:
// 1. end_date is within the configured short_window_minutes from now
// 2. Question/slug contains patterns like "up", "down", "above", "below", "price"
// 3. Must have order book enabled and meet minimum liquidity thresholds
// ============================================================================

/// Patterns that indicate a short-window up/down price prediction market
static UP_DOWN_PATTERNS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(up|down|above|below|higher|lower|price|btc|eth|sol|crypto|15.?min|30.?min|\d+:\d+)",
    )
    .expect("Invalid regex pattern")
});

/// Result of short-window market analysis
#[derive(Debug, Clone)]
pub struct ShortWindowInfo {
    /// Whether this qualifies as a short-window market
    pub is_short_window: bool,
    /// Minutes until market resolution (if end_date is set)
    pub minutes_to_expiry: Option<i64>,
    /// Whether the question/slug matches up/down patterns
    pub matches_pattern: bool,
}

impl Market {
    /// Analyze if this market qualifies as a short-window up/down market.
    /// These are binary price prediction markets resolving soon (e.g., 15-30 min).
    pub fn analyze_short_window(&self, config: &crate::utils::MarketsConfig) -> ShortWindowInfo {
        let now = Utc::now();

        // Check end_date proximity
        let minutes_to_expiry = self.end_date.as_ref().and_then(|end_date_str| {
            // Try parsing ISO 8601 format
            DateTime::parse_from_rfc3339(end_date_str)
                .ok()
                .or_else(|| {
                    // Try other common formats
                    DateTime::parse_from_str(end_date_str, "%Y-%m-%dT%H:%M:%S%.fZ").ok()
                })
                .or_else(|| DateTime::parse_from_str(end_date_str, "%Y-%m-%d %H:%M:%S").ok())
                .map(|dt| {
                    let expiry = dt.with_timezone(&Utc);
                    let duration = expiry.signed_duration_since(now);
                    duration.num_minutes()
                })
        });

        // Check if within short window and above minimum buffer
        let in_short_window = minutes_to_expiry.is_some_and(|mins| {
            mins > config.min_minutes_to_expiry as i64 && mins <= config.short_window_minutes as i64
        });

        // Check question/slug for up/down patterns
        let question_lower = self.question.to_lowercase();
        let slug_lower = self.slug.to_lowercase();
        let matches_pattern =
            UP_DOWN_PATTERNS.is_match(&question_lower) || UP_DOWN_PATTERNS.is_match(&slug_lower);

        // Must match both time window AND pattern to qualify
        let is_short_window =
            config.enable_short_window_markets && in_short_window && matches_pattern;

        ShortWindowInfo {
            is_short_window,
            minutes_to_expiry,
            matches_pattern,
        }
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
        let url = format!(
            "{}/markets?active=true&closed=false&limit=1000",
            self.base_url
        );

        debug!("Fetching markets from {}", url);

        let response = self
            .client
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
        // =====================================================================
        // BLACKLIST CHECK (always enforced, no exceptions)
        // =====================================================================
        if config
            .blacklisted_markets
            .iter()
            .any(|blacklist| market.market.contains(blacklist) || market.slug.contains(blacklist))
        {
            return false;
        }

        // =====================================================================
        // ORDER BOOK CHECK (safety: required for market making)
        // =====================================================================
        if config.enforce_enable_order_book && !market.enable_order_book {
            debug!("Skipping market without order book: {}", market.question);
            return false;
        }

        // =====================================================================
        // ACTIVE/CLOSED CHECK
        // =====================================================================
        if market.closed || !market.active {
            debug!("Skipping closed/inactive market: {}", market.question);
            return false;
        }

        // =====================================================================
        // OUTCOME VALIDATION
        // =====================================================================
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

        // =====================================================================
        // SHORT-WINDOW MARKET CHECK (dynamic 15m up/down discovery)
        // These markets bypass category filters but have their own volume threshold
        // =====================================================================
        let short_window_info = market.analyze_short_window(config);

        if short_window_info.is_short_window {
            // Short-window markets use a lower volume threshold
            if let Some(volume) = market.volume_24h {
                if volume < config.min_volume_24h_short as f64 {
                    debug!(
                        "Skipping low-volume short-window market: {} (${:.0} < ${})",
                        market.question, volume, config.min_volume_24h_short
                    );
                    return false;
                }
            }

            // Log discovery of short-window market
            info!(
                "🎯 Short-window market discovered: {} (expires in ~{} min)",
                market.question,
                short_window_info.minutes_to_expiry.unwrap_or(0)
            );
            return true;
        }

        // =====================================================================
        // CATEGORY FILTER (for non-short-window markets)
        // =====================================================================
        if !config.prioritize_categories.is_empty() {
            if let Some(ticker_tag) = &market.ticker_tag {
                let ticker_lower = ticker_tag.to_lowercase();

                let is_prioritized = config
                    .prioritize_categories
                    .iter()
                    .any(|category| ticker_lower.contains(&category.to_lowercase()));

                if !is_prioritized {
                    debug!(
                        "Skipping non-prioritized market: {} ({})",
                        market.question, ticker_tag
                    );
                    return false;
                }
            }
        }

        // =====================================================================
        // VOLUME CHECK (standard threshold for non-short-window markets)
        // =====================================================================
        if let Some(volume) = market.volume_24h {
            if volume < config.min_volume_24h as f64 {
                debug!(
                    "Skipping low-volume market: {} (${:.0} < ${})",
                    market.question, volume, config.min_volume_24h
                );
                return false;
            }
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
    pub fn filter_markets_by_category(&self, markets: &[Market], category: &str) -> Vec<Market> {
        markets
            .iter()
            .filter(|market| {
                market
                    .ticker_tag
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
                market
                    .volume_24h
                    .is_some_and(|vol| vol >= min_volume as f64)
            })
            .cloned()
            .collect()
    }
}
