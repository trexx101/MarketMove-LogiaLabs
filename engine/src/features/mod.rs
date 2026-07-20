//! V2 feature pipeline (Wave 5).
//!
//! The legacy V1 pipeline (3 OHLCV features) lives in `legacy` and is re-exported
//! so the running scheduler/inference keep working with the existing model. The
//! V2 pipeline (6-dim: vol_regime, vol_break, funding_rate, basis_z, llm_bull_prob,
//! ob_imbalance) is activated only after the new model clears the walk-forward
//! OOS IC gate.

pub mod core;
pub mod crypto;
pub mod equities;
pub mod llm;
pub mod legacy;
pub mod volatility;

// Re-export V1 types so existing callers (`crate::features::compute_features`,
// `crate::features::FeatureRow`) keep resolving during the transition.
pub use legacy::{compute_features, FeatureRow};

use anyhow::Result;

use crate::features::core::FeatureRowV2;

/// Pluggable feature source. `BinanceFeatureSource` is the active impl; the
/// `EquitiesFeatureSourceStub` is a placeholder for a future expansion wave
/// (the core model is asset-agnostic).
#[async_trait::async_trait]
pub trait FeatureSource: Send + Sync {
    /// Fetch the latest feature row for live inference.
    async fn fetch_latest(&self, symbol: &str) -> Result<FeatureRowV2>;
    /// Backfill a window of feature rows for rolling normalization / warmup.
    async fn backfill_window(&self, symbol: &str, limit: usize) -> Result<Vec<FeatureRowV2>>;
}
