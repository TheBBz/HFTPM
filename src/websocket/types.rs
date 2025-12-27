use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    #[serde(rename = "event_type", default)]
    pub event_type: String,
    #[serde(rename = "asset_id", default)]
    pub asset_id: String,
    #[serde(rename = "market")]
    pub market: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub bids: Option<Vec<OrderSummary>>,
    #[serde(default)]
    pub asks: Option<Vec<OrderSummary>>,
    #[serde(default)]
    pub price_changes: Option<Vec<PriceChange>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSummary {
    pub price: String,
    pub size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceChange {
    #[serde(rename = "asset_id")]
    pub asset_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub hash: String,
    #[serde(rename = "best_bid", default)]
    pub best_bid: String,
    #[serde(rename = "best_ask", default)]
    pub best_ask: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub asset_id: String,
    pub market: String,
    pub bids: Vec<(rust_decimal::Decimal, rust_decimal::Decimal)>,
    pub asks: Vec<(rust_decimal::Decimal, rust_decimal::Decimal)>,
    pub timestamp: i64,
    pub hash: String,
}

impl WsMessage {
    pub fn is_book_snapshot(&self) -> bool {
        self.event_type == "book" || (self.bids.is_some() || self.asks.is_some())
    }

    pub fn is_price_change(&self) -> bool {
        self.event_type == "price_change" || self.price_changes.is_some()
    }

    pub fn parse_timestamp(&self) -> i64 {
        self.timestamp
            .as_ref()
            .and_then(|ts| ts.parse().ok())
            .unwrap_or_else(|| {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64
            })
    }
}

impl BookSnapshot {
    #[inline]
    pub fn is_stale(&self, max_age_ms: u64) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        now - self.timestamp > max_age_ms as i64
    }
}
