pub mod client;

pub use client::{WebSocketClient, WsMessage, WsMessageHandler};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    #[serde(rename = "event_type")]
    pub event_type: String,
    #[serde(rename = "asset_id")]
    pub asset_id: String,
    #[serde(rename = "market")]
    pub market: String,
    pub timestamp: Option<String>,
    pub hash: Option<String>,
    pub bids: Option<Vec<OrderSummary>>,
    pub asks: Option<Vec<OrderSummary>>,
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
    #[serde(rename = "best_bid")]
    pub best_bid: String,
    #[serde(rename = "best_ask")]
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
        self.event_type == "book"
    }

    pub fn is_price_change(&self) -> bool {
        self.event_type == "price_change"
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
