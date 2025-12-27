use crate::websocket::types::BookSnapshot;
use crate::utils::Config;
use anyhow::{Result, Context};
use dashmap::DashMap;
use std::collections::BTreeMap;
use rust_decimal::Decimal;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub market_id: String,
    pub asset_id: String,
    pub bids: BTreeMap<Decimal, Decimal>,
    pub asks: BTreeMap<Decimal, Decimal>,
    pub timestamp: i64,
    pub hash: String,
}

impl OrderBook {
    pub fn new(
        market_id: String,
        asset_id: String,
        timestamp: i64,
        hash: String,
    ) -> Self {
        Self {
            market_id,
            asset_id,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            timestamp,
            hash,
        }
    }

    #[inline]
    pub fn update_from_snapshot(&mut self, snapshot: &BookSnapshot) {
        self.bids = snapshot.bids.iter().cloned().collect();
        self.asks = snapshot.asks.iter().cloned().collect();
        self.timestamp = snapshot.timestamp;
        self.hash = snapshot.hash.clone();

        debug!(
            "📊 Updated order book for {}: {} bids, {} asks, timestamp: {}",
            self.market_id,
            self.bids.len(),
            self.asks.len(),
            self.timestamp
        );
    }

    #[inline]
    pub fn update_price(&mut self, price: Decimal, size: Decimal, side: &str) {
        match side {
            "BUY" | "buy" => {
                if size > Decimal::ZERO {
                    self.bids.insert(price, size);
                } else {
                    self.bids.remove(&price);
                }
            }
            "SELL" | "sell" => {
                if size > Decimal::ZERO {
                    self.asks.insert(price, size);
                } else {
                    self.asks.remove(&price);
                }
            }
            _ => {}
        }

        self.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
    }

    #[inline]
    pub fn best_bid(&self) -> Option<(Decimal, Decimal)> {
        self.bids
            .last_key_value()
            .map(|(price, size)| (*price, *size))
    }

    #[inline]
    pub fn best_ask(&self) -> Option<(Decimal, Decimal)> {
        self.asks
            .first_key_value()
            .map(|(price, size)| (*price, *size))
    }

    #[inline]
    pub fn spread(&self) -> Option<Decimal> {
        if let (Some((bid_price, _)), Some((ask_price, _))) = (self.best_bid(), self.best_ask()) {
            if ask_price > bid_price {
                return Some(ask_price - bid_price);
            }
        }
        None
    }

    #[inline]
    pub fn bid_depth_at(&self, price: Decimal) -> Decimal {
        self.bids
            .range(price..)
            .map(|(_, size)| *size)
            .sum()
    }

    #[inline]
    pub fn ask_depth_at(&self, price: Decimal) -> Decimal {
        self.asks
            .range(..=price)
            .map(|(_, size)| *size)
            .sum()
    }

    #[inline]
    pub fn total_bid_depth(&self) -> Decimal {
        self.bids.values().sum()
    }

    #[inline]
    pub fn total_ask_depth(&self) -> Decimal {
        self.asks.values().sum()
    }

    #[inline]
    pub fn is_stale(&self, max_age_ms: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        now - self.timestamp > max_age_ms as i64
    }
}

#[derive(Debug, Clone)]
pub struct MarketBooks {
    pub market_id: String,
    pub asset_id_yes: Option<String>,
    pub asset_id_no: Option<String>,
    pub books: Vec<OrderBook>,
}

impl MarketBooks {
    pub fn new(market_id: String) -> Self {
        Self {
            market_id,
            asset_id_yes: None,
            asset_id_no: None,
            books: Vec::new(),
        }
    }

    #[inline]
    pub fn is_binary(&self) -> bool {
        self.books.len() == 2 &&
            self.asset_id_yes.is_some() &&
            self.asset_id_no.is_some()
    }

    #[inline]
    pub fn get_binary_book_sum(&self) -> Option<Decimal> {
        if !self.is_binary() {
            return None;
        }

        let (yes_price, _) = self.books[0].best_ask()?;
        let (no_price, _) = self.books[1].best_ask()?;

        Some(yes_price + no_price)
    }

    #[inline]
    pub fn get_total_ask_sum(&self) -> Decimal {
        self.books
            .iter()
            .filter_map(|book| book.best_ask().map(|(price, _)| price))
            .sum()
    }

    #[inline]
    pub fn min_liquidity_at_best_asks(&self) -> Decimal {
        self.books
            .iter()
            .filter_map(|book| book.best_ask().map(|(_, size)| size))
            .min()
            .unwrap_or(Decimal::ZERO)
    }
}

pub struct OrderBookManager {
    #[allow(dead_code)]
    config: Arc<Config>,
    market_books: DashMap<String, MarketBooks>,
}

impl OrderBookManager {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            config: Arc::new(config.clone()),
            market_books: DashMap::new(),
        })
    }

    #[inline]
    pub fn update_book(&self, market_id: &str, asset_id: &str, snapshot: &BookSnapshot) -> Result<()> {
        if snapshot.is_stale(500) {
            warn!("Stale book for {} (timestamp: {})", market_id, snapshot.timestamp);
            return Ok(());
        }

        let mut market_books = self.market_books
            .entry(market_id.to_string())
            .or_insert_with(|| MarketBooks::new(market_id.to_string()));

        let mut new_book = OrderBook::new(
            market_id.to_string(),
            asset_id.to_string(),
            snapshot.timestamp,
            snapshot.hash.clone(),
        );

        new_book.update_from_snapshot(snapshot);

        // First, update the books vector
        if let Some(idx) = market_books.books.iter().position(|b| b.asset_id == asset_id) {
            market_books.books[idx] = new_book;
        } else {
            market_books.books.push(new_book);
        }

        // Then, if we have exactly 2 books, determine yes/no asset IDs
        if market_books.books.len() == 2 {
            let asset_id_0 = market_books.books[0].asset_id.clone();
            let asset_id_1 = market_books.books[1].asset_id.clone();
            let bids_0_len = market_books.books[0].bids.keys().len();
            let asks_0_len = market_books.books[0].asks.keys().len();

            if bids_0_len > asks_0_len {
                market_books.asset_id_yes = Some(asset_id_0);
                market_books.asset_id_no = Some(asset_id_1);
            } else {
                market_books.asset_id_yes = Some(asset_id_1);
                market_books.asset_id_no = Some(asset_id_0);
            }
        }

        let updated = true;

        if updated {
            debug!(
                "✅ Updated order book for market {} asset {}",
                market_id,
                asset_id
            );
        }

        Ok(())
    }

    #[inline]
    pub fn update_price(
        &self,
        market_id: &str,
        asset_id: &str,
        price: Decimal,
        size: Decimal,
        side: &str,
    ) -> Result<()> {
        let mut market_books = self.market_books
            .get_mut(market_id)
            .context("Market not found")?;

        for book in &mut market_books.books {
            if book.asset_id == asset_id {
                book.update_price(price, size, side);
                break;
            }
        }

        Ok(())
    }

    #[inline]
    pub fn get_market_books(&self, market_id: &str) -> Option<MarketBooks> {
        self.market_books.get(market_id).map(|books| books.clone())
    }

    /// Get a specific order book by market_id and asset_id
    #[inline]
    pub fn get_book(&self, market_id: &str, asset_id: &str) -> Option<OrderBook> {
        let market_books = self.market_books.get(market_id)?;
        market_books.books.iter()
            .find(|book| book.asset_id == asset_id)
            .cloned()
    }

    #[inline]
    pub fn get_best_asks_for_market(&self, market_id: &str) -> Option<Vec<(String, Decimal, Decimal)>> {
        let market_books = self.get_market_books(market_id)?;

        Some(
            market_books.books
                .iter()
                .filter_map(|book| {
                    book.best_ask().map(|(price, size)| {
                        (book.asset_id.clone(), price, size)
                    })
                })
                .collect()
        )
    }

    #[inline]
    pub fn get_bid_ask_sum(&self, market_id: &str) -> Option<Decimal> {
        let market_books = self.get_market_books(market_id)?;

        if market_books.is_binary() {
            market_books.get_binary_book_sum()
        } else {
            Some(market_books.get_total_ask_sum())
        }
    }

    #[inline]
    pub fn get_min_liquidity_at_best_asks(&self, market_id: &str) -> Option<Decimal> {
        let market_books = self.get_market_books(market_id)?;
        Some(market_books.min_liquidity_at_best_asks())
    }

    #[inline]
    pub fn get_all_market_ids(&self) -> Vec<String> {
        self.market_books.iter().map(|entry| entry.key().clone()).collect()
    }

    #[inline]
    pub fn cleanup_stale_books(&self, max_age_ms: u64) {
        let mut stale_markets = Vec::new();

        for entry in self.market_books.iter() {
            let market_id = entry.key();
            let market_books = entry.value();

            for book in &market_books.books {
                if book.is_stale(max_age_ms) {
                    stale_markets.push(market_id.clone());
                    break;
                }
            }
        }

        for market_id in stale_markets {
            self.market_books.remove(&market_id);
            debug!("🗑️  Cleaned up stale market book: {}", market_id);
        }
    }
}
