//! D1 (DEFERRED to R1 reasoning model): volatility-regime + structural-break.
//!
//! This module defines the interface the assembler depends on. The actual GARCH
//! estimate and changepoint detection are filled in by the quant reasoning pass
//! (see `.omo/plans/mmn-edge-overhaul-scaffold.md` D1). For now both return `0.0`
//! so the engine compiles and runs end-to-end; they MUST be replaced before any
//! deploy decision.

use crate::db::Candle;

/// Volatility-regime estimate for a window of candles.
///
/// TODO(D1): implement a GARCH(1,1)-style rolling forecast, scaled to ~[0,2].
pub fn vol_regime(candles: &[Candle]) -> f64 {
    let _ = candles;
    0.0
}

/// Structural-break / changepoint indicator for a window of candles.
///
/// TODO(D1): implement a Bayesian/PELT-style changepoint probability in [0,1].
pub fn vol_break(candles: &[Candle]) -> f64 {
    let _ = candles;
    0.0
}
