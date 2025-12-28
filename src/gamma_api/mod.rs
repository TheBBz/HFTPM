use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInfo {
    pub id: String,
    pub title: Option<String>,
}

// =============================================================================
// Event/Series API Response Types (for short-window market discovery)
// =============================================================================

/// Series metadata from events API (e.g., "BTC Up or Down 15m" series)
#[derive(Debug, Clone, Deserialize)]
pub struct EventSeries {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub recurrence: Option<String>, // "15m", "30m", etc.
}

/// Nested market within an event response
#[derive(Debug, Clone, Deserialize)]
pub struct EventMarket {
    pub id: String,
    pub question: String,
    pub slug: String,
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub outcomes: Option<String>, // JSON string: "[\"Up\", \"Down\"]"
    #[serde(rename = "clobTokenIds", default)]
    pub clob_token_ids: Option<String>, // JSON string with token IDs
    #[serde(rename = "endDate")]
    pub end_date: Option<String>,
    #[serde(rename = "volume24hr", default)]
    pub volume_24hr: Option<f64>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub closed: bool,
    #[serde(rename = "enableOrderBook", default)]
    pub enable_order_book: bool,
    #[serde(rename = "acceptingOrders", default)]
    pub accepting_orders: bool,
}

impl EventMarket {
    /// Convert EventMarket to the standard Market struct
    pub fn to_market(&self, event_id: &str, category: Option<String>) -> Option<Market> {
        // Parse outcomes from JSON string
        let outcomes = self.outcomes.as_ref().and_then(|s| {
            serde_json::from_str::<Vec<String>>(s).ok()
        }).unwrap_or_default();
        
        if outcomes.is_empty() {
            return None;
        }
        
        // Parse token IDs from JSON string
        let token_ids = self.clob_token_ids.as_ref().and_then(|s| {
            serde_json::from_str::<Vec<String>>(s).ok()
        }).unwrap_or_default();
        
        if token_ids.len() != outcomes.len() {
            return None;
        }
        
        let outcome_structs: Vec<Outcome> = outcomes
            .into_iter()
            .enumerate()
            .map(|(i, name)| Outcome {
                id: i.to_string(),
                name,
                token_id: token_ids.get(i).cloned().unwrap_or_default(),
            })
            .collect();
        
        Some(Market {
            id: self.id.clone(),
            question: self.question.clone(),
            slug: self.slug.clone(),
            market: self.condition_id.clone(),
            description: self.description.clone(),
            outcomes: outcome_structs,
            assets_ids: token_ids,
            ticker_tag: category,
            end_date: self.end_date.clone(),
            volume_24h: self.volume_24hr,
            active: self.active,
            closed: self.closed,
            enable_order_book: self.enable_order_book,
            events: vec![EventInfo {
                id: event_id.to_string(),
                title: Some(self.question.clone()),
            }],
        })
    }
}

/// Full event response from /events endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct EventResponse {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub markets: Vec<EventMarket>,
    #[serde(default)]
    pub series: Vec<EventSeries>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub closed: bool,
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

    // =========================================================================
    // Short-Window Market Discovery (15m/30m Up/Down markets from /events API)
    // =========================================================================
    
    /// Known series slugs for short-window markets (15m/30m/1h up/down)
    /// Discovered via Gamma API research - these markets only exist in /events endpoint
    const SHORT_WINDOW_SERIES: &'static [&'static str] = &[
        // 15-minute series (highest frequency, best for short-window arb)
        "btc-up-or-down-15m",
        "eth-up-or-down-15m",
        "sol-up-or-down-15m",
        "link-up-or-down-15m",
        "doge-up-or-down-15m",
        "xrp-up-or-down-15m",
        "sui-up-or-down-15m",
        "pepe-up-or-down-15m",
        "avax-up-or-down-15m",
        "ada-up-or-down-15m",
        "bnb-up-or-down-15m",
        "pol-up-or-down-15m",
        "near-up-or-down-15m",
        "apt-up-or-down-15m",
        "hype-up-or-down-15m",
        // 30-minute series
        "btc-up-or-down-30m",
        "eth-up-or-down-30m",
        "sol-up-or-down-30m",
        // 1-hour series
        "btc-up-or-down-1h",
        "eth-up-or-down-1h",
        "sol-up-or-down-1h",
        "link-up-or-down-1h",
        "doge-up-or-down-1h",
        "xrp-up-or-down-1h",
        "sui-up-or-down-1h",
        "pepe-up-or-down-1h",
    ];

    /// Fetch short-window markets from the events API
    /// The API's series_slug filter is broken, so we generate event slugs dynamically
    /// based on current time (e.g., btc-updown-15m-{timestamp})
    pub async fn fetch_short_window_markets(
        &self,
        markets_config: &crate::utils::MarketsConfig,
    ) -> Result<Vec<Market>> {
        if !markets_config.enable_short_window_markets {
            return Ok(Vec::new());
        }

        info!("🔍 Fetching short-window markets from Events API...");
        
        let mut short_window_markets = Vec::new();
        let now = chrono::Utc::now();
        
        // Generate timestamps for the next few 15-minute intervals
        // Round down to nearest 15 minutes, then get current + next few windows
        let current_minute = now.minute();
        let interval_start = (current_minute / 15) * 15;
        let base_time = now
            .with_minute(interval_start).unwrap_or(now)
            .with_second(0).unwrap_or(now)
            .with_nanosecond(0).unwrap_or(now);
        
        // Crypto tickers that have 15m markets
        let tickers_15m = ["btc", "eth", "sol", "link", "doge", "xrp", "sui", "pepe", "avax", "ada", "bnb", "pol", "near", "apt", "hype"];
        let tickers_1h = ["btc", "eth", "sol", "link", "doge", "xrp", "sui", "pepe"];
        
        // Fetch 15m events (current and next 3 intervals)
        for offset_mins in [0i64, 15, 30, 45] {
            let event_time = base_time + chrono::Duration::minutes(offset_mins);
            let timestamp = event_time.timestamp();
            
            for ticker in &tickers_15m {
                let event_slug = format!("{}-updown-15m-{}", ticker, timestamp);
                if let Some(market) = self.fetch_event_by_slug(&event_slug, markets_config).await {
                    short_window_markets.push(market);
                }
            }
        }
        
        // Fetch 1h events (current and next hour)
        let hour_start = now
            .with_minute(0).unwrap_or(now)
            .with_second(0).unwrap_or(now)
            .with_nanosecond(0).unwrap_or(now);
        
        for offset_hours in [0i64, 1] {
            let event_time = hour_start + chrono::Duration::hours(offset_hours);
            let timestamp = event_time.timestamp();
            
            for ticker in &tickers_1h {
                let event_slug = format!("{}-updown-1h-{}", ticker, timestamp);
                if let Some(market) = self.fetch_event_by_slug(&event_slug, markets_config).await {
                    short_window_markets.push(market);
                }
            }
        }

        info!("✅ Found {} short-window markets from Events API", short_window_markets.len());

        // Add to cache
        let mut cache = self.markets_cache.write().await;
        for market in &short_window_markets {
            cache.insert(market.market.clone(), market.clone());
        }

        Ok(short_window_markets)
    }
    
    /// Fetch a single event by its slug and extract the market
    async fn fetch_event_by_slug(
        &self,
        event_slug: &str,
        markets_config: &crate::utils::MarketsConfig,
    ) -> Option<Market> {
        let url = format!("{}/events/{}", self.base_url, event_slug);
        
        match self.client.get(&url).send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    return None; // Event doesn't exist for this timestamp
                }
                
                match response.json::<EventResponse>().await {
                    Ok(event) => {
                        // Get the first active market from the event
                        for event_market in &event.markets {
                            if !event_market.active || event_market.closed || !event_market.enable_order_book {
                                continue;
                            }
                            
                            if let Some(market) = event_market.to_market(&event.id, Some("crypto".to_string())) {
                                let short_info = market.analyze_short_window(markets_config);
                                if short_info.is_short_window {
                                    info!(
                                        "🎯 Short-window market found: {} (expires in ~{} min)",
                                        market.question,
                                        short_info.minutes_to_expiry.unwrap_or(0)
                                    );
                                    return Some(market);
                                }
                            }
                        }
                        None
                    }
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    }
}

