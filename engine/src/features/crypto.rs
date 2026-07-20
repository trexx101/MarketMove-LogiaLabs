//! V2 feature assembly for Binance (crypto) data.
//!
//! `BinanceFeatureSource` assembles a `FeatureRowV2` from the latest candles in
//! the DB plus the cached LLM regime probability. The D1 volatility features and
//! D4 LLM feature are sourced from `volatility` / `llm` modules (currently stubs).

use anyhow::Result;
use async_trait::async_trait;

use crate::db::{self, Candle, DbPool};
use crate::features::core::{FeatureRowV2, FEATURE_DIM};
use crate::features::llm;
use crate::features::volatility;

use super::FeatureSource;

/// Fetches the Binance symbol string used for REST/WS from a config symbol
/// like `BTC/USD` -> `BTCUSDT`.
pub fn to_binance_symbol(symbol: &str) -> String {
    symbol.replace('/', "").to_uppercase()
}

pub struct BinanceFeatureSource {
    pool: DbPool,
    /// How many trailing candles to read for warmup / vol estimates.
    window: usize,
}

impl BinanceFeatureSource {
    pub fn new(pool: DbPool, window: usize) -> Self {
        Self { pool, window }
    }
}

#[async_trait]
impl FeatureSource for BinanceFeatureSource {
    async fn fetch_latest(&self, _symbol: &str) -> Result<FeatureRowV2> {
        let candles = db::fetch_recent_candles(&self.pool, self.window).await?;
        Ok(assemble(&candles))
    }

    async fn backfill_window(&self, _symbol: &str, limit: usize) -> Result<Vec<FeatureRowV2>> {
        let candles = db::fetch_recent_candles(&self.pool, limit.max(self.window)).await?;
        if candles.len() < self.window {
            return Ok(Vec::new());
        }
        // Slide a window of `window` across the history, producing one row per tail.
        let mut out = Vec::new();
        for i in (self.window..=candles.len()).rev().take(limit) {
            out.push(assemble(&candles[i - self.window..i]));
        }
        out.reverse();
        Ok(out)
    }
}

/// Assemble a single `FeatureRowV2` from a window of candles.
///
/// Funding/basis/ob_imbalance are read from the most recent candle's auxiliary
/// fields once the Binance data client (S1) populates them. Until then they are
/// `0.0`. D1 volatility features come from `volatility`; D4 from the LLM cache.
fn assemble(candles: &[Candle]) -> FeatureRowV2 {
    let last = candles.last().expect("assemble called with non-empty window");
    FeatureRowV2 {
        timestamp: last.ts,
        vol_regime: volatility::vol_regime(candles),
        vol_break: volatility::vol_break(candles),
        funding_rate: last.funding_rate,
        basis_z: last.basis_z,
        llm_bull_prob: llm::read_cached_bull_prob(),
        ob_imbalance: last.ob_imbalance,
    }
}

/// Helper used by tests / the scheduler to assemble a window into the fixed
/// feature vector expected by the inference service.
pub fn assemble_window(candles: &[Candle]) -> Option<[f64; FEATURE_DIM]> {
    if candles.is_empty() {
        return None;
    }
    Some(assemble(candles).to_array())
}
