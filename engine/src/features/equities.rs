//! Equities `FeatureSource` stub (extensibility placeholder).
//!
//! The core temporal model is asset-agnostic; swapping crypto for equities means
//! implementing this source (replace funding/on-chain with earnings surprise,
//! intraday volatility, etc.) without touching the model. Intentionally
//! unimplemented until the equities expansion wave.

use anyhow::Result;
use async_trait::async_trait;

use crate::features::core::FeatureRowV2;

use super::FeatureSource;

pub struct EquitiesFeatureSourceStub;

#[async_trait]
impl FeatureSource for EquitiesFeatureSourceStub {
    async fn fetch_latest(&self, _symbol: &str) -> Result<FeatureRowV2> {
        unimplemented!("equities FeatureSource is a placeholder for a future wave");
    }

    async fn backfill_window(&self, _symbol: &str, _limit: usize) -> Result<Vec<FeatureRowV2>> {
        unimplemented!("equities FeatureSource is a placeholder for a future wave");
    }
}
