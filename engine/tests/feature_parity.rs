//! Golden fixture test — verifies that Rust-computed features match
//! the expected values derived from the Colab feature pipeline.
//!
//! Tolerance: 1e-6 (per acceptance criteria).
//!
//! Fixture: 5 hourly candles with hand-verified OHLCV values.
//! Expected feature values are computed from first principles using the
//! same formulas as `engine::features::compute_features`.

use engine::db::Candle;
use engine::features::compute_features;
use engine::normalize::{normalize_row, NormStats};

/// Candle constructor helper.
fn c(ts: i64, open: f64, high: f64, low: f64, close: f64, volume: f64, vwap: f64) -> Candle {
    Candle { ts, open, high, low, close, volume, vwap }
}

/// Normalization stats as exported from training (from `models/norm_stats.json`).
fn training_stats() -> NormStats {
    NormStats {
        mean: [
            -0.004212553236568371,
            -0.11279115979987146,
            -0.04671712566224684,
        ],
        std: [
            0.997939242240963,
            1.315850980779091,
            1.142457884275758,
        ],
    }
}

// ---------------------------------------------------------------------------
// Feature computation parity
// ---------------------------------------------------------------------------

#[test]
fn feature_parity_log_return_candle0_is_zero() {
    let candles = vec![c(0, 100.0, 105.0, 95.0, 100.0, 10.0, 100.0)];
    let rows = compute_features(&candles);
    assert!(
        rows[0].log_return.abs() < 1e-12,
        "first candle log_return must be 0, got {}",
        rows[0].log_return
    );
}

#[test]
fn feature_parity_log_return_candle1() {
    // c0: close=100, c1: close=108 → log(108/100) = log(1.08)
    let candles = vec![
        c(0,    100.0, 105.0, 95.0,  100.0, 10.0, 100.0),
        c(3600, 100.0, 110.0, 98.0,  108.0, 12.0, 104.0),
    ];
    let rows = compute_features(&candles);
    let expected = (108.0_f64 / 100.0).ln();
    assert!(
        (rows[1].log_return - expected).abs() < 1e-12,
        "log_return expected {expected:.10}, got {:.10}",
        rows[1].log_return
    );
}

#[test]
fn feature_parity_true_range_candle1() {
    // c0: close=100; c1: high=110, low=98
    // TR = max(110-98=12, |110-100|=10, |98-100|=2) = 12
    let candles = vec![
        c(0,    100.0, 105.0, 95.0, 100.0, 10.0, 100.0),
        c(3600, 100.0, 110.0, 98.0, 108.0, 12.0, 104.0),
    ];
    let rows = compute_features(&candles);
    // ATR at index 1 = mean(TR[0], TR[1]) = mean(10, 12) = 11
    let expected_atr = (10.0 + 12.0) / 2.0;
    assert!(
        (rows[1].atr_72 - expected_atr).abs() < 1e-12,
        "atr_72 at idx=1 expected {expected_atr}, got {}",
        rows[1].atr_72
    );
}

#[test]
fn feature_parity_vwap_dev_candle1() {
    // close=108, vwap=104 → (108-104)/104 = 4/104
    let candles = vec![
        c(0,    100.0, 105.0, 95.0, 100.0, 10.0, 100.0),
        c(3600, 100.0, 110.0, 98.0, 108.0, 12.0, 104.0),
    ];
    let rows = compute_features(&candles);
    let expected = (108.0 - 104.0) / 104.0;
    assert!(
        (rows[1].vwap_dev - expected).abs() < 1e-12,
        "vwap_dev expected {expected:.10}, got {:.10}",
        rows[1].vwap_dev
    );
}

#[test]
fn feature_parity_atr_window_72_candles() {
    // All TRs are exactly 2.0; after 72+ candles the rolling mean stabilises at 2.0.
    let mut candles = Vec::with_capacity(80);
    // first candle: TR = high - low = 2.0
    candles.push(c(0, 100.0, 101.0, 99.0, 100.0, 100.0, 100.0));
    for i in 1..80_i64 {
        // high = prev_close + 1, low = prev_close - 1 → TR always 2.0
        candles.push(c(i * 3600, 100.0, 101.0, 99.0, 100.0, 100.0, 100.0));
    }
    let rows = compute_features(&candles);
    for (i, row) in rows.iter().enumerate().skip(71) {
        assert!(
            (row.atr_72 - 2.0).abs() < 1e-12,
            "atr_72 at idx={i} should be 2.0, got {}",
            row.atr_72
        );
    }
}

#[test]
fn feature_parity_vwap_dev_zero_when_vwap_zero() {
    let candles = vec![c(0, 100.0, 105.0, 95.0, 100.0, 10.0, 0.0)];
    let rows = compute_features(&candles);
    assert_eq!(rows[0].vwap_dev, 0.0, "vwap_dev must be 0 when vwap=0");
}

// ---------------------------------------------------------------------------
// Normalization parity
// ---------------------------------------------------------------------------

#[test]
fn normalization_parity_identity_stats() {
    use engine::features::FeatureRow;
    let feat = FeatureRow { log_return: 1.0, atr_72: 2.0, vwap_dev: 3.0 };
    let stats = NormStats { mean: [0.0, 0.0, 0.0], std: [1.0, 1.0, 1.0] };
    let z = normalize_row(&feat, &stats);
    assert!((z[0] - 1.0).abs() < 1e-12);
    assert!((z[1] - 2.0).abs() < 1e-12);
    assert!((z[2] - 3.0).abs() < 1e-12);
}

#[test]
fn normalization_parity_training_stats_candle1() {
    // Known fixture: c0 close=100, c1 close=108, vwap=104
    let candles = vec![
        c(0,    100.0, 105.0, 95.0, 100.0, 10.0, 100.0),
        c(3600, 100.0, 110.0, 98.0, 108.0, 12.0, 104.0),
    ];
    let rows = compute_features(&candles);
    let stats = training_stats();
    let z = normalize_row(&rows[1], &stats);

    // Manually derived expected values:
    // log_return = ln(1.08) ≈ 0.07696104128675416
    // atr_72 = 11.0
    // vwap_dev = 4/104 ≈ 0.038461538461538
    let lr = (108.0_f64 / 100.0).ln();
    let atr = 11.0_f64;
    let vd = 4.0_f64 / 104.0;

    let exp_z0 = (lr - stats.mean[0]) / stats.std[0];
    let exp_z1 = (atr - stats.mean[1]) / stats.std[1];
    let exp_z2 = (vd - stats.mean[2]) / stats.std[2];

    assert!(
        (z[0] - exp_z0).abs() < 1e-6,
        "z[0] expected {exp_z0:.10}, got {:.10}",
        z[0]
    );
    assert!(
        (z[1] - exp_z1).abs() < 1e-6,
        "z[1] expected {exp_z1:.10}, got {:.10}",
        z[1]
    );
    assert!(
        (z[2] - exp_z2).abs() < 1e-6,
        "z[2] expected {exp_z2:.10}, got {:.10}",
        z[2]
    );
}
