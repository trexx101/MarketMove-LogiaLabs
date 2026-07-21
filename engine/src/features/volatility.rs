//! Adaptive realized-vol percentile + CUSUM structural break.
//! vol_regime: recent 20-bar realized vol vs historical percentile [0,2].
//! vol_break: CUSUM break detector [0,1].
//! Matches training/labels.py for train==serve parity.

use crate::db::Candle;

/// Adaptive volatility regime: percentile rank of recent 20-bar realized vol
/// within the last 500 bars. Scaled [0, 2]. Replaces dead GARCH.
pub fn vol_regime(candles: &[Candle]) -> f64 {
    if candles.len() < 20 {
        return 1.0;
    }

    // Collect log returns.
    let mut returns: Vec<f64> = candles
        .windows(2)
        .filter_map(|w| {
            if w[0].close > 0.0 && w[1].close > 0.0 {
                let r = (w[1].close / w[0].close).ln();
                if r.is_finite() { Some(r) } else { None }
            } else {
                None
            }
        })
        .collect();

    if returns.len() < 10 {
        return 1.0;
    }

    // Check if all returns are essentially zero (constant price).
    // Prevents division by zero in realized vol.
    let returns_std = {
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
        var.sqrt()
    };
    if returns_std < 1e-12 {
        return 1.0; // Flat market → neutral vol regime
    }

    // Recent 20-bar realized vol (annualized approximation).
    let recent_n = 20usize.min(returns.len());
    let recent = &returns[returns.len() - recent_n..];
    let recent_mean = recent.iter().sum::<f64>() / recent_n as f64;
    let recent_var = recent.iter().map(|r| (r - recent_mean).powi(2)).sum::<f64>() / recent_n as f64;
    let recent_vol = recent_var.sqrt() * 93.6; // sqrt(8760) ≈ 93.6 for hourly→annual

    // Historical percentile (lookback up to 500 bars).
    let hist_n = 500usize.min(returns.len());
    let hist_returns = &returns[..hist_n];
    let mut count_below = 0usize;
    let mut count_total = 0usize;

    for i in recent_n..hist_n {
        let window = &hist_returns[i - recent_n..i];
        let w_mean = window.iter().sum::<f64>() / recent_n as f64;
        let w_var = window.iter().map(|r| (r - w_mean).powi(2)).sum::<f64>() / recent_n as f64;
        let w_vol = w_var.sqrt() * 93.6;
        if w_vol <= recent_vol {
            count_below += 1;
        }
        count_total += 1;
    }

    if count_total < 5 {
        return 1.0;
    }

    let percentile = count_below as f64 / count_total as f64;
    2.0 * percentile
}

/// CUSUM structural break detector (scaled 0-1).
pub fn vol_break(candles: &[Candle]) -> f64 {
    if candles.len() < 10 {
        return 0.0;
    }

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
        cusum = 0.0_f64.max(cusum + r - mean - k);
    }

    (1.0 - (-cusum / std_dev).exp()).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Candle;

    fn candle(ts: i64, close: f64) -> Candle {
        Candle { ts, open: close, high: close, low: close, close, volume: 1.0, vwap: close,
                 funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }
    }

    fn candles_from_prices(prices: &[f64]) -> Vec<Candle> {
        prices.iter().enumerate().map(|(i, &p)| candle(i as i64, p)).collect()
    }

    #[test]
    fn vol_regime_neutral_on_short_input() {
        let c = candles_from_prices(&[100.0]);
        assert!((vol_regime(&c) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn vol_regime_constant_is_at_mid() {
        // Constant price → all returns ~0 → std ≈ 0 → returns 1.0 (neutral).
        let c = candles_from_prices(&[100.0; 100]);
        assert!((vol_regime(&c) - 1.0).abs() < 0.01, "constant price should be neutral ~1.0, got {}", vol_regime(&c));
    }

    #[test]
    fn vol_regime_oscillating_responses() {
        // Oscillating returns should produce some variance but still in range.
        let mut prices = vec![100.0];
        for i in 1..200 {
            prices.push(prices[i - 1] * if i % 2 == 0 { 1.01 } else { 0.99 });
        }
        let c = candles_from_prices(&prices);
        let vol = vol_regime(&c);
        assert!(vol >= 0.0 && vol <= 2.0, "vol_regime should be in [0,2], got {vol}");
    }

    #[test]
    fn vol_break_stable_is_low() {
        let mut prices = vec![100.0];
        for _ in 0..49 { prices.push(*prices.last().unwrap() * 1.001); }
        let c = candles_from_prices(&prices);
        assert!(vol_break(&c) < 0.3, "stable trend should give low break, got {}", vol_break(&c));
    }

    #[test]
    fn vol_break_spike_is_high() {
        let mut prices = vec![100.0];
        for _ in 0..20 { prices.push(*prices.last().unwrap() * 1.0001); }
        for i in 0..30 { prices.push(*prices.last().unwrap() * if i % 2 == 0 { 1.02 } else { 0.98 }); }
        let c = candles_from_prices(&prices);
        assert!(vol_break(&c) > 0.5, "vol spike should trigger break, got {}", vol_break(&c));
    }
}
