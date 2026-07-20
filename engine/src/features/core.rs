//! V2 feature row (Wave 5).
//!
//! Fixed feature order — MUST match the training pipeline's `FeatureRowV2`
//! and the ZMQ `feature_window` layout exactly:
//!   0: vol_regime    — GARCH-style volatility regime estimate (D1)
//!   1: vol_break      — structural-break / changepoint indicator (D1)
//!   2: funding_rate   — Binance perpetual funding rate
//!   3: basis_z        — spot-vs-perp basis Z-score
//!   4: llm_bull_prob  — OpenRouter hourly cached regime probability [0,1] (D4)
//!   5: ob_imbalance   — order-book imbalance from depth stream (D4/S1)
//!
//! D1/D4 internals are implemented in `volatility.rs` / `llm.rs`. Until those
//! are filled, the corresponding fields are produced as `0.0` by the assembler
//! so the engine still compiles and runs end-to-end.

use serde::{Deserialize, Serialize};

/// Number of model features in the V2 contract.
pub const FEATURE_DIM: usize = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureRowV2 {
    pub timestamp: i64,
    pub vol_regime: f64,
    pub vol_break: f64,
    pub funding_rate: f64,
    pub basis_z: f64,
    pub llm_bull_prob: f64,
    pub ob_imbalance: f64,
}

impl FeatureRowV2 {
    /// Assemble the fixed-order feature vector for ZMQ / TCN inference.
    pub fn to_array(&self) -> [f64; FEATURE_DIM] {
        [
            self.vol_regime,
            self.vol_break,
            self.funding_rate,
            self.basis_z,
            self.llm_bull_prob,
            self.ob_imbalance,
        ]
    }
}
