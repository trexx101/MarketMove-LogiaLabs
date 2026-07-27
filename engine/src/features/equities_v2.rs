//! Wave B: equities feature pipeline (8 features).
//!
//! All features are reproducible from daily OHLCV + macro series stored in
//! `equity_candles`. The feature vector is designed for daily-horizon QQQ
//! prediction (1d / 5d / 21d) with LightGBM or a TCN.
//!
//! Feature order (MUST match training/inference contract):
//!   0: trend_slope     — 50d MA slope (log diff of SMA50 over 20d)
//!   1: trend_adx       — 14-period ADX (trend strength, 0-100)
//!   2: rsi_14          — 14-period RSI (0-100, momentum)
//!   3: vix_regime      — VIX level bucketed: <18 calm, 18-25 normal, >25 stress
//!   4: tlt_corr_20d    — 20-day rolling correlation QQQ vs TLT
//!   5: rvol_20d        — relative volume: today's volume / 20d avg volume
//!   6: gap_pct         — overnight gap: (open[t] - close[t-1]) / close[t-1]
//!   7: drawdown_from_50d_high — how far close is from the rolling 50d high (negative)

use serde::{Deserialize, Serialize};

use crate::db::EquityCandle;

/// Number of equities features.
pub const EQ_FEATURE_DIM: usize = 8;

/// Canonical feature ordering — MUST stay aligned with the trained
/// `norm_stats_qqq_v1.json` and `model_meta_qqq_v1.json` artifacts.
pub const EQ_FEATURE_NAMES: [&str; EQ_FEATURE_DIM] = [
    "trend_slope",
    "trend_adx",
    "rsi_14",
    "vix_regime",
    "tlt_corr_20d",
    "rvol_20d",
    "gap_pct",
    "drawdown_from_50d_high",
];

/// Feature row for the equities pipeline.
/// Fields map 1:1 to the `EQ_FEATURE_DIM` array ordering above.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquityFeatureRow {
    pub timestamp: i64,
    pub trend_slope: f64,
    pub trend_adx: f64,
    pub rsi_14: f64,
    pub vix_regime: f64,
    pub tlt_corr_20d: f64,
    pub rvol_20d: f64,
    pub gap_pct: f64,
    pub drawdown_from_50d_high: f64,
}

impl EquityFeatureRow {
    /// Fixed-order feature vector for inference / normalization.
    pub fn to_array(&self) -> [f64; EQ_FEATURE_DIM] {
        [
            self.trend_slope,
            self.trend_adx,
            self.rsi_14,
            self.vix_regime,
            self.tlt_corr_20d,
            self.rvol_20d,
            self.gap_pct,
            self.drawdown_from_50d_high,
        ]
    }
}

/// Compute equities features for every candle in `qqq` (daily OHLCV, oldest
/// first), optionally enriched with VIX and TLT close-aligned series.
///
/// `vix_close` and `tlt_close` must be aligned to `qqq` by timestamp (same
/// length, same ordering). If either is `None` or shorter than `qqq`, the
/// corresponding feature is filled with `0.0` for the missing entries.
///
/// Returns one `EquityFeatureRow` per input candle. Warmup periods that lack
/// sufficient lookback produce `0.0` for the affected features (not NaN).
pub fn compute_equity_features(
    qqq: &[EquityCandle],
    vix_close: Option<&[f64]>,
    tlt_close: Option<&[f64]>,
) -> Vec<EquityFeatureRow> {
    let n = qqq.len();
    if n == 0 {
        return Vec::new();
    }

    // --- pre-compute closes, volumes, etc. ---
    let closes: Vec<f64> = qqq.iter().map(|c| c.close).collect();
    let volumes: Vec<i64> = qqq.iter().map(|c| c.volume).collect();
    let opens: Vec<f64> = qqq.iter().map(|c| c.open).collect();

    // SMA50 for trend slope: slope = ln(sma50[t]) - ln(sma50[t-20])
    let sma50 = rolling_sma(&closes, 50);

    // ADX(14)
    let adx = adx_14(qqq);

    // RSI(14)
    let rsi = rsi_14(&closes);

    // VIX regime bucketing.
    let vix_feat: Vec<f64> = match vix_close {
        Some(v) if v.len() == n => v
            .iter()
            .map(|&vix| {
                if vix <= 0.0 {
                    0.0
                } else if vix < 18.0 {
                    0.0 // calm
                } else if vix < 25.0 {
                    1.0 // normal
                } else {
                    2.0 // stress
                }
            })
            .collect(),
        _ => vec![0.0; n],
    };

    // 20-day rolling correlation QQQ vs TLT.
    let tlt_corr: Vec<f64> = match tlt_close {
        Some(t) if t.len() == n => rolling_correlation(&closes, t, 20),
        _ => vec![0.0; n],
    };

    // Relative volume: today's volume / 20d avg.
    let rvol = rvol_20d(&volumes);

    // Overnight gap.
    let gaps = gap_pct(&opens, &closes);

    // Drawdown from 50d high.
    let dd = drawdown_from_high(&closes, 50);

    // Assemble rows.
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let trend_slope = if i >= 20 && sma50[i].is_finite() && sma50[i - 20].is_finite() && sma50[i - 20] > 0.0 {
            (sma50[i] / sma50[i - 20]).ln()
        } else {
            0.0
        };

        rows.push(EquityFeatureRow {
            timestamp: qqq[i].ts,
            trend_slope,
            trend_adx: adx[i],
            rsi_14: rsi[i],
            vix_regime: vix_feat[i],
            tlt_corr_20d: tlt_corr[i],
            rvol_20d: rvol[i],
            gap_pct: gaps[i],
            drawdown_from_50d_high: dd[i],
        });
    }
    rows
}

// ---- normalization (robust median/MAD) ----

/// Normalization statistics computed via median and MAD (median absolute
/// deviation). Robust to outliers, matching the V2 crypto scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityNormStats {
    pub median: [f64; EQ_FEATURE_DIM],
    pub mad: [f64; EQ_FEATURE_DIM],
}

impl EquityNormStats {
    /// Compute norm stats from a set of feature rows.
    pub fn from_rows(rows: &[EquityFeatureRow]) -> Self {
        let mut stats = EquityNormStats {
            median: [0.0; EQ_FEATURE_DIM],
            mad: [0.0; EQ_FEATURE_DIM],
        };
        for j in 0..EQ_FEATURE_DIM {
            let col: Vec<f64> = rows.iter().map(|r| r.to_array()[j]).collect();
            let med = median(&col);
            let mad_val = median(
                &col.iter()
                    .map(|v| (v - med).abs())
                    .collect::<Vec<_>>(),
            );
            stats.median[j] = med;
            stats.mad[j] = mad_val;
        }
        stats
    }

    /// Normalize a feature row to a robust z-score vector.
    /// Uses (x - median) / (1.4826 * MAD) where 1.4826 scales MAD to std.
    /// If MAD is ~0, returns 0.0 for that dimension.
    pub fn normalize(&self, row: &EquityFeatureRow) -> [f64; EQ_FEATURE_DIM] {
        let raw = row.to_array();
        let mut out = [0.0; EQ_FEATURE_DIM];
        for i in 0..EQ_FEATURE_DIM {
            let scale = 1.4826 * self.mad[i];
            out[i] = if scale < 1e-12 {
                0.0
            } else {
                (raw[i] - self.median[i]) / scale
            };
        }
        out
    }

    /// Serialize to a JSON file.
    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Load from a JSON file in the canonical positional-array format
    /// (`{"median":[...],"mad":[...]}`).
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Load from the name-keyed training format
    /// (`{"medians":{"trend_slope":...},"mads":{"trend_adx":...}}`).
    /// Missing entries fall back to 0.0 (median) / 1.0 (mad).
    pub fn load_named(path: &str) -> anyhow::Result<Self> {
        #[derive(serde::Deserialize)]
        struct Named {
            medians: std::collections::HashMap<String, f64>,
            mads: std::collections::HashMap<String, f64>,
        }
        let text = std::fs::read_to_string(path)?;
        let n: Named = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parse named norm-stats {path}: {e}"))?;
        let mut median = [0.0_f64; EQ_FEATURE_DIM];
        let mut mad = [1.0_f64; EQ_FEATURE_DIM];
        for (i, name) in EQ_FEATURE_NAMES.iter().enumerate() {
            if let Some(v) = n.medians.get(*name) {
                median[i] = *v;
            }
            if let Some(v) = n.mads.get(*name) {
                mad[i] = *v;
            }
        }
        Ok(Self { median, mad })
    }
}

// ================================================================
// Indicator implementations
// ================================================================

/// Simple rolling SMA. Returns Vec<f64> with same length as input.
/// Entries before the full window is available are NaN.
pub fn rolling_sma(values: &[f64], window: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![f64::NAN; n];
    if window == 0 || n < window {
        return out;
    }
    let mut sum: f64 = values[..window].iter().sum();
    out[window - 1] = sum / window as f64;
    for i in window..n {
        sum += values[i] - values[i - window];
        out[i] = sum / window as f64;
    }
    out
}

/// ADX(14) — Average Directional Index.
/// Standard Wilder's smoothing. Returns Vec<f64> same length as input.
/// Warmup entries (first ~28 bars) are 0.0.
fn adx_14(candles: &[EquityCandle]) -> Vec<f64> {
    let n = candles.len();
    let mut out = vec![0.0; n];
    if n < 28 {
        return out;
    }

    // +DM, -DM, TR
    let mut plus_dm = vec![0.0; n];
    let mut minus_dm = vec![0.0; n];
    let mut tr = vec![0.0; n];

    for i in 1..n {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_high = candles[i - 1].high;
        let prev_low = candles[i - 1].low;
        let prev_close = candles[i - 1].close;

        let up_move = high - prev_high;
        let down_move = prev_low - low;
        plus_dm[i] = if up_move > down_move && up_move > 0.0 { up_move } else { 0.0 };
        minus_dm[i] = if down_move > up_move && down_move > 0.0 { down_move } else { 0.0 };

        tr[i] = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
    }

    // Wilder's smoothing of TR, +DM, -DM over 14 periods.
    let period = 14usize;
    let mut atr = wilder_smooth(&tr, period);
    let mut smoothed_plus = wilder_smooth(&plus_dm, period);
    let mut smoothed_minus = wilder_smooth(&minus_dm, period);

    // +DI, -DI
    let mut plus_di = vec![0.0; n];
    let mut minus_di = vec![0.0; n];
    let mut dx = vec![0.0; n];

    for i in period..n {
        if atr[i] > 0.0 {
            plus_di[i] = 100.0 * smoothed_plus[i] / atr[i];
            minus_di[i] = 100.0 * smoothed_minus[i] / atr[i];
        }
        let sum = plus_di[i] + minus_di[i];
        if sum > 0.0 {
            dx[i] = 100.0 * (plus_di[i] - minus_di[i]).abs() / sum;
        }
    }

    // ADX = Wilder's smoothing of DX
    let adx_vals = wilder_smooth(&dx, period);
    for i in 0..n {
        out[i] = if adx_vals[i].is_finite() { adx_vals[i] } else { 0.0 };
    }
    out
}

/// Wilder's smoothing (equivalent to EMA with alpha = 1/period).
/// First value is the simple sum of the first `period` values.
fn wilder_smooth(values: &[f64], period: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![f64::NAN; n];
    if n < period || period == 0 {
        return out;
    }
    let mut sum: f64 = values[..period].iter().sum();
    out[period - 1] = sum / period as f64;
    for i in period..n {
        out[i] = (out[i - 1] * (period as f64 - 1.0) + values[i]) / period as f64;
        sum += values[i]; // keep sum for potential reuse
    }
    let _ = sum;
    out
}

/// RSI(14) using Wilder's smoothing.
fn rsi_14(closes: &[f64]) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![50.0; n]; // neutral RSI during warmup
    if n < 15 {
        return out;
    }

    let mut gains = vec![0.0; n];
    let mut losses = vec![0.0; n];
    for i in 1..n {
        let diff = closes[i] - closes[i - 1];
        if diff >= 0.0 {
            gains[i] = diff;
        } else {
            losses[i] = -diff;
        }
    }

    let period = 14usize;
    // Initial average gain/loss = simple mean of first 14.
    let mut avg_gain: f64 = gains[1..=period].iter().sum::<f64>() / period as f64;
    let mut avg_loss: f64 = losses[1..=period].iter().sum::<f64>() / period as f64;

    out[period] = rsi_from_gl(avg_gain, avg_loss);

    for i in (period + 1)..n {
        avg_gain = (avg_gain * (period as f64 - 1.0) + gains[i]) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + losses[i]) / period as f64;
        out[i] = rsi_from_gl(avg_gain, avg_loss);
    }
    out
}

fn rsi_from_gl(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss < 1e-12 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

/// 20-day rolling relative volume: volume[t] / mean(volume[t-20..t]).
fn rvol_20d(volumes: &[i64]) -> Vec<f64> {
    let n = volumes.len();
    let mut out = vec![0.0; n];
    if n < 21 {
        return out;
    }
    for i in 20..n {
        let avg: f64 = volumes[i - 20..i].iter().map(|&v| v as f64).sum::<f64>() / 20.0;
        out[i] = if avg > 0.0 { volumes[i] as f64 / avg } else { 0.0 };
    }
    out
}

/// Overnight gap: (open[t] - close[t-1]) / close[t-1].
fn gap_pct(opens: &[f64], closes: &[f64]) -> Vec<f64> {
    let n = opens.len();
    let mut out = vec![0.0; n];
    for i in 1..n {
        if closes[i - 1] > 0.0 {
            out[i] = (opens[i] - closes[i - 1]) / closes[i - 1];
        }
    }
    out
}

/// Drawdown from rolling N-day high: (close[t] - max(high[t-N..t])) / max(...).
/// Returns negative or zero values.
fn drawdown_from_high(closes: &[f64], window: usize) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![0.0; n];
    if n < window + 1 {
        return out;
    }
    for i in window..n {
        let high = closes[i - window..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        if high > 0.0 {
            out[i] = (closes[i] - high) / high;
        }
    }
    out
}

/// 20-day rolling Pearson correlation between two series.
fn rolling_correlation(a: &[f64], b: &[f64], window: usize) -> Vec<f64> {
    let n = a.len();
    let mut out = vec![0.0; n];
    if n < window + 1 || window == 0 {
        return out;
    }
    for i in window..n {
        let slice_a = &a[i - window..i];
        let slice_b = &b[i - window..i];
        out[i] = pearson_r(slice_a, slice_b);
    }
    out
}

fn pearson_r(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let mean_x: f64 = x.iter().sum::<f64>() / n;
    let mean_y: f64 = y.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for i in 0..x.len() {
        let dx = x[i] - mean_x;
        let dy = y[i] - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }
    let denom = (var_x * var_y).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        cov / denom
    }
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

// ================================================================
// Tests
// ================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn eq_candle(ts: i64, o: f64, h: f64, l: f64, c: f64, v: i64) -> EquityCandle {
        EquityCandle {
            symbol: "QQQ".to_string(),
            ts,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            source: "yahoo".to_string(),
        }
    }

    fn synthetic_qqq(n: usize) -> Vec<EquityCandle> {
        let mut candles = Vec::with_capacity(n);
        let mut price = 100.0_f64;
        for i in 0..n {
            // Slow uptrend with small noise.
            let change = if i % 3 == 0 { -0.5 } else { 1.0 };
            price += change;
            let o = price - 0.3;
            let h = price + 1.5;
            let l = price - 1.5;
            let c = price;
            let v = 1_000_000 + (i as i64 * 5000);
            candles.push(eq_candle(i as i64 * 86400, o, h, l, c, v));
        }
        candles
    }

    #[test]
    fn feature_dim_matches() {
        assert_eq!(EQ_FEATURE_DIM, 8);
    }

    #[test]
    fn compute_returns_correct_length() {
        let candles = synthetic_qqq(100);
        let rows = compute_equity_features(&candles, None, None);
        assert_eq!(rows.len(), 100);
    }

    #[test]
    fn warmup_features_are_zero() {
        // With only 5 candles, all features requiring lookback should be 0.
        let candles = synthetic_qqq(5);
        let rows = compute_equity_features(&candles, None, None);
        for r in &rows {
            assert_eq!(r.trend_slope, 0.0, "trend_slope should be 0 during warmup");
            assert_eq!(r.trend_adx, 0.0, "adx should be 0 during warmup");
            assert_eq!(r.rvol_20d, 0.0, "rvol should be 0 during warmup");
        }
    }

    #[test]
    fn trend_slope_nonzero_after_warmup() {
        let candles = synthetic_qqq(100);
        let rows = compute_equity_features(&candles, None, None);
        // Row 80 should have a positive slope (uptrend).
        assert!(
            rows[80].trend_slope != 0.0,
            "trend_slope should be non-zero after 80 bars"
        );
    }

    #[test]
    fn rsi_in_range() {
        let candles = synthetic_qqq(100);
        let rows = compute_equity_features(&candles, None, None);
        for r in &rows {
            assert!(r.rsi_14 >= 0.0 && r.rsi_14 <= 100.0, "RSI out of range: {}", r.rsi_14);
        }
    }

    #[test]
    fn adx_in_range() {
        let candles = synthetic_qqq(100);
        let rows = compute_equity_features(&candles, None, None);
        for r in &rows {
            assert!(r.trend_adx >= 0.0 && r.trend_adx <= 100.0, "ADX out of range: {}", r.trend_adx);
        }
    }

    #[test]
    fn vix_regime_buckets() {
        let candles = synthetic_qqq(60);
        // VIX = 15 → calm → 0.0
        let vix = vec![15.0; 60];
        let rows = compute_equity_features(&candles, Some(&vix), None);
        assert_eq!(rows[59].vix_regime, 0.0);

        // VIX = 20 → normal → 1.0
        let vix2 = vec![20.0; 60];
        let rows2 = compute_equity_features(&candles, Some(&vix2), None);
        assert_eq!(rows2[59].vix_regime, 1.0);

        // VIX = 30 → stress → 2.0
        let vix3 = vec![30.0; 60];
        let rows3 = compute_equity_features(&candles, Some(&vix3), None);
        assert_eq!(rows3[59].vix_regime, 2.0);
    }

    #[test]
    fn tlt_correlation_in_range() {
        let candles = synthetic_qqq(60);
        let tlt = (0..60).map(|i| 50.0 + (i as f64 * 0.1)).collect::<Vec<_>>();
        let rows = compute_equity_features(&candles, None, Some(&tlt));
        for r in &rows {
            assert!(r.tlt_corr_20d >= -1.0 && r.tlt_corr_20d <= 1.0, "corr out of range");
        }
    }

    #[test]
    fn rvol_positive_after_warmup() {
        let candles = synthetic_qqq(50);
        let rows = compute_equity_features(&candles, None, None);
        assert!(rows[40].rvol_20d > 0.0, "rvol should be positive after 20 bars");
    }

    #[test]
    fn gap_pct_is_zero_first_bar() {
        let candles = synthetic_qqq(30);
        let rows = compute_equity_features(&candles, None, None);
        assert_eq!(rows[0].gap_pct, 0.0, "first bar gap should be 0");
    }

    #[test]
    fn drawdown_nonpositive() {
        let candles = synthetic_qqq(100);
        let rows = compute_equity_features(&candles, None, None);
        for r in &rows {
            assert!(r.drawdown_from_50d_high <= 0.0, "drawdown should be <= 0");
        }
    }

    #[test]
    fn norm_stats_normalize() {
        let candles = synthetic_qqq(100);
        let rows = compute_equity_features(&candles, None, None);
        let stats = EquityNormStats::from_rows(&rows);
        let normalized = stats.normalize(&rows[80]);
        assert_eq!(normalized.len(), EQ_FEATURE_DIM);
        // Check no NaN.
        for v in &normalized {
            assert!(v.is_finite(), "normalized value is NaN");
        }
    }

    #[test]
    fn norm_stats_save_load_roundtrip() {
        let candles = synthetic_qqq(50);
        let rows = compute_equity_features(&candles, None, None);
        let stats = EquityNormStats::from_rows(&rows);
        let path = std::env::temp_dir().join("test_eq_norm_stats.json");
        stats.save(path.to_str().unwrap()).unwrap();
        let loaded = EquityNormStats::load(path.to_str().unwrap()).unwrap();
        for i in 0..EQ_FEATURE_DIM {
            assert!((stats.median[i] - loaded.median[i]).abs() < 1e-9);
            assert!((stats.mad[i] - loaded.mad[i]).abs() < 1e-9);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn norm_stats_load_named_matches_trained_artifact() {
        // Load the actual trained QQQ norm-stats JSON (string-keyed shape).
        let trained_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models")
            .join("norm_stats_qqq_v1.json");
        if !trained_path.exists() {
            eprintln!("skipping: trained artifact not at {trained_path:?}");
            return;
        }
        let stats = EquityNormStats::load_named(trained_path.to_str().unwrap()).unwrap();

        // Property-based checks (not hardcoded to specific training-run values,
        // which change when the model is retrained).
        //
        // Feature order: [trend_slope, trend_adx, rsi_14, vix_regime,
        //                  tlt_corr_20d, rvol_20d, gap_pct, drawdown_from_50d_high]
        //
        // trend_slope (idx 0): small value around 0
        assert!(stats.median[0].abs() < 0.1, "trend_slope median: {}", stats.median[0]);
        assert!(stats.mad[0] > 1e-4, "trend_slope MAD too small: {}", stats.mad[0]);

        // trend_adx (idx 1): real ADX, 0-100 scale, median should be 10-50
        assert!(stats.median[1] > 5.0, "trend_adx median too low: {} (ATR proxy?)", stats.median[1]);
        assert!(stats.median[1] < 80.0, "trend_adx median too high: {}", stats.median[1]);
        assert!(stats.mad[1] >= 1.0, "trend_adx MAD too small: {}", stats.mad[1]);

        // rsi_14 (idx 2): 0-100 scale, median should be 35-65, MAD >= 5
        assert!(stats.median[2] > 20.0, "rsi_14 median too low: {} (clipped?)", stats.median[2]);
        assert!(stats.median[2] < 80.0, "rsi_14 median too high: {}", stats.median[2]);
        assert!(stats.mad[2] >= 5.0, "rsi_14 MAD too small: {} (near-zero = explosion bug)", stats.mad[2]);

        // Round-trip through normalize() must not produce NaN or extreme values.
        let candles = synthetic_qqq(60);
        let rows = compute_equity_features(&candles, None, None);
        let out = stats.normalize(&rows[50]);
        for v in &out {
            assert!(v.is_finite(), "normalized value is NaN/Inf: {v}");
            assert!(v.abs() < 1e6, "normalized value exploded: {v} (norm_stats bug?)");
        }
    }

    #[test]
    fn to_array_order_matches_struct() {
        let row = EquityFeatureRow {
            timestamp: 0,
            trend_slope: 1.0,
            trend_adx: 2.0,
            rsi_14: 3.0,
            vix_regime: 4.0,
            tlt_corr_20d: 5.0,
            rvol_20d: 6.0,
            gap_pct: 7.0,
            drawdown_from_50d_high: 8.0,
        };
        let arr = row.to_array();
        assert_eq!(arr[0], 1.0); // trend_slope
        assert_eq!(arr[1], 2.0); // trend_adx
        assert_eq!(arr[2], 3.0); // rsi_14
        assert_eq!(arr[3], 4.0); // vix_regime
        assert_eq!(arr[4], 5.0); // tlt_corr_20d
        assert_eq!(arr[5], 6.0); // rvol_20d
        assert_eq!(arr[6], 7.0); // gap_pct
        assert_eq!(arr[7], 8.0); // drawdown_from_50d_high
    }
}
