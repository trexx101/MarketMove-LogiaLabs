//! GARCH(1,1) volatility regime and CUSUM-based structural break detectors.
//! Features scaled to [0,2] and [0,1] respectively for neural network compatibility.
//! Optimized for O(n) per-bar computation with numerical stability.
//!
//! D1: produced by DeepSeek-R1, reviewed + corrected by Hermes agent:
//! - Removed `..Default::default()` (Candle doesn't derive Default).
//! - Removed `rand::random()` test dependency (not in Cargo.toml).
//! - Fixed CUSUM sign direction (was always 0 due to `0.0.max(...)` on positive side).

use crate::db::Candle;

/// GARCH(1,1) volatility regime indicator (scaled 0-2).
/// - ω=0.05, α=0.1, β=0.85 provide stable long-run vol ~1.0.
/// - Scaled via: 2 * (current_vol / (current_vol + long_run_vol))
///   where long_run_vol = sqrt(ω/(1-α-β)) = 1.0.
pub fn vol_regime(candles: &[Candle]) -> f64 {
    if candles.len() < 2 {
        return 1.0; // Neutral during initialization
    }

    const OMEGA: f64 = 0.05;
    const ALPHA: f64 = 0.1;
    const BETA: f64 = 0.85;
    const LONG_RUN_VOL: f64 = 1.0; // sqrt(OMEGA / (1.0 - ALPHA - BETA))

    // Compute log returns, filtering invalid values.
    let mut sigma2 = OMEGA / (1.0 - ALPHA - BETA);
    let mut count = 0usize;
    for w in candles.windows(2) {
        if w[0].close > 0.0 && w[1].close > 0.0 {
            let r = (w[1].close / w[0].close).ln();
            if r.is_finite() {
                sigma2 = OMEGA + ALPHA * r.powi(2) + BETA * sigma2;
                count += 1;
            }
        }
    }

    if count == 0 {
        return 1.0; // No valid returns
    }

    let current_vol = sigma2.sqrt();
    2.0 * current_vol / (current_vol + LONG_RUN_VOL)
}

/// Structural break detector using CUSUM of absolute returns (scaled 0-1).
/// - Detects mean shifts in |returns| with k=0.5σ sensitivity.
/// - Output: 1 - exp(-CUSUM/σ) capped at 1.0.
pub fn vol_break(candles: &[Candle]) -> f64 {
    if candles.len() < 10 {
        return 0.0;
    }

    // Compute absolute log returns.
    let abs_returns: Vec<f64> = candles
        .windows(2)
        .filter_map(|w| {
            if w[0].close > 0.0 && w[1].close > 0.0 {
                let r = (w[1].close / w[0].close).ln().abs();
                if r.is_finite() { Some(r) } else { None }
            } else {
                None
            }
        })
        .collect();

    if abs_returns.len() < 5 {
        return 0.0;
    }

    let mean = abs_returns.iter().sum::<f64>() / abs_returns.len() as f64;
    let variance = abs_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / abs_returns.len() as f64;
    let std_dev = variance.sqrt().max(1e-8);

    let k = 0.5 * std_dev;
    let mut cusum = 0.0_f64;

    for r in &abs_returns {
        // Upper-sided CUSUM: detects volatility increase.
        cusum = 0.0_f64.max(cusum + r - mean - k);
    }

    (1.0 - (-cusum / std_dev).exp()).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Candle;

    fn candle(ts: i64, close: f64) -> Candle {
        Candle {
            ts,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            vwap: close,
            funding_rate: 0.0,
            basis_z: 0.0,
            ob_imbalance: 0.0,
        }
    }

    fn candles_from_prices(prices: &[f64]) -> Vec<Candle> {
        prices
            .iter()
            .enumerate()
            .map(|(i, &p)| candle(i as i64, p))
            .collect()
    }

    #[test]
    fn vol_regime_constant_price_is_at_floor() {
        // Constant price => zero returns, GARCH converges to σ² = ω/(1-β) = 0.333.
        // Scaled: 2*sqrt(0.333)/(sqrt(0.333)+1) ≈ 0.732.
        let candles = candles_from_prices(&[100.0; 100]);
        let vol = vol_regime(&candles);
        assert!((vol - 0.732).abs() < 0.02, "constant price floor should be ~0.732, got {vol}");
    }

    #[test]
    fn vol_regime_neutral_on_short_input() {
        let candles = candles_from_prices(&[100.0]);
        let vol = vol_regime(&candles);
        assert!((vol - 1.0).abs() < 1e-9, "single candle should return neutral 1.0");
    }

    #[test]
    fn vol_regime_oscillating_is_near_long_run() {
        let mut prices = vec![100.0];
        for i in 1..200 {
            prices.push(prices[i - 1] * if i % 2 == 0 { 1.01 } else { 0.99 });
        }
        let candles = candles_from_prices(&prices);
        let vol = vol_regime(&candles);
        assert!(vol > 0.5 && vol < 1.5, "oscillating vol should be near long-run, got {vol}");
    }

    #[test]
    fn vol_break_stable_is_low() {
        let mut prices = vec![100.0];
        for _ in 0..49 {
            prices.push(*prices.last().unwrap() * 1.001);
        }
        let candles = candles_from_prices(&prices);
        let br = vol_break(&candles);
        assert!(br < 0.3, "stable trend should give low break, got {br}");
    }

    #[test]
    fn vol_break_spike_is_high() {
        let mut prices = vec![100.0];
        for _ in 0..20 {
            prices.push(*prices.last().unwrap() * 1.0001);
        }
        // Simulate volatility spike with deterministic alternating pattern.
        for i in 0..30 {
            let factor = if i % 2 == 0 { 1.02 } else { 0.98 };
            prices.push(*prices.last().unwrap() * factor);
        }
        let candles = candles_from_prices(&prices);
        let br = vol_break(&candles);
        assert!(br > 0.5, "volatility spike should trigger break, got {br}");
    }
}
