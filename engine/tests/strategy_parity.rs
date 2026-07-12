//! Strategy parity golden fixture test.
//!
//! Validates the regime-filtered hysteresis state machine against the
//! Colab backtester logic described in `Training_model_Design.md`
//! (cells `b_0ifN-sOviQ` + `p7gjxTQjQjWa`).
//!
//! All expected outputs are derived from first principles by manually
//! applying the 4-step algorithm:
//!   1. raw_signal    — align 4H+24H against threshold
//!   2. regime        — close vs SMA200
//!   3. filtered      — asymmetric regime gate
//!   4. hysteresis    — ffill (hold when filtered==0)

use engine::strategy::{compute_sma, next_position, Position, SignalInput, StrategyParams};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn params(threshold: f64) -> StrategyParams {
    StrategyParams {
        magnitude_threshold: threshold,
        sma_window: 200,
    }
}

fn input(pred_4h: f64, pred_24h: f64, close: f64, sma: f64, valid: bool) -> SignalInput {
    SignalInput {
        pred_4h,
        pred_24h,
        current_close: close,
        sma,
        sma_valid: valid,
    }
}

// ---------------------------------------------------------------------------
// SMA computation
// ---------------------------------------------------------------------------

#[test]
fn sma_of_200_identical_closes_equals_that_value() {
    let closes = vec![50_000.0_f64; 200];
    let (sma, valid) = compute_sma(&closes, 200);
    assert!(valid, "200 closes should produce a valid SMA");
    assert!((sma - 50_000.0).abs() < 1e-9, "SMA of identical values must equal that value");
}

#[test]
fn sma_with_fewer_closes_than_window_is_invalid() {
    let closes = vec![50_000.0_f64; 150];
    let (sma, valid) = compute_sma(&closes, 200);
    assert!(!valid, "150 closes against window=200 must be invalid");
    assert!((sma - 50_000.0).abs() < 1e-9, "partial SMA value still correct");
}

#[test]
fn sma_uses_only_last_window_elements() {
    // First 10 = 0.0, last 5 = 100.0 — window=5 should give mean=100
    let mut closes = vec![0.0_f64; 10];
    closes.extend(vec![100.0_f64; 5]);
    let (sma, valid) = compute_sma(&closes, 5);
    assert!(valid);
    assert!((sma - 100.0).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Single-step state-machine parity
// ---------------------------------------------------------------------------

#[test]
fn entry_long_requires_both_predictions_above_threshold() {
    // Only 4H above threshold — no entry
    let p = params(0.005);
    let no_entry = next_position(
        Position::Flat,
        &input(0.01, 0.002, 51_000.0, 50_000.0, true),
        &p,
    );
    assert_eq!(no_entry, Position::Flat, "4H only should not trigger long");

    // Both above threshold in bullish regime — enter Long
    let entry = next_position(
        Position::Flat,
        &input(0.01, 0.01, 51_000.0, 50_000.0, true),
        &p,
    );
    assert_eq!(entry, Position::Long);
}

#[test]
fn entry_short_requires_both_predictions_below_neg_threshold() {
    let p = params(0.005);
    // Only 24H below — no entry
    let no_entry = next_position(
        Position::Flat,
        &input(-0.002, -0.01, 49_000.0, 50_000.0, true),
        &p,
    );
    assert_eq!(no_entry, Position::Flat);

    // Both below in bearish regime — enter Short
    let entry = next_position(
        Position::Flat,
        &input(-0.01, -0.01, 49_000.0, 50_000.0, true),
        &p,
    );
    assert_eq!(entry, Position::Short);
}

#[test]
fn regime_blocks_long_in_bearish_market() {
    // Strong long signal but close < SMA → blocked
    let result = next_position(
        Position::Flat,
        &input(0.05, 0.05, 49_000.0, 50_000.0, true),
        &params(0.005),
    );
    assert_eq!(result, Position::Flat, "long signal must be blocked in bearish regime");
}

#[test]
fn regime_blocks_short_in_bullish_market() {
    // Strong short signal but close > SMA → blocked
    let result = next_position(
        Position::Flat,
        &input(-0.05, -0.05, 51_000.0, 50_000.0, true),
        &params(0.005),
    );
    assert_eq!(result, Position::Flat, "short signal must be blocked in bullish regime");
}

#[test]
fn hysteresis_holds_long_when_no_new_signal() {
    let p = params(0.005);
    // Neutral signal (below threshold) — position held
    let held = next_position(
        Position::Long,
        &input(0.002, 0.003, 51_000.0, 50_000.0, true),
        &p,
    );
    assert_eq!(held, Position::Long, "long must be held on neutral signal (ffill)");
}

#[test]
fn hysteresis_holds_short_when_no_new_signal() {
    let p = params(0.005);
    let held = next_position(
        Position::Short,
        &input(-0.003, -0.002, 49_000.0, 50_000.0, true),
        &p,
    );
    assert_eq!(held, Position::Short, "short must be held on neutral signal (ffill)");
}

#[test]
fn hysteresis_holds_long_when_short_signal_blocked_by_regime() {
    // Currently long, close > SMA (bullish), short signal fires but is regime-blocked
    let result = next_position(
        Position::Long,
        &input(-0.01, -0.01, 51_000.0, 50_000.0, true),
        &params(0.005),
    );
    assert_eq!(
        result,
        Position::Long,
        "long held when short signal is blocked by bullish regime (ffill)"
    );
}

#[test]
fn position_flips_long_to_short_when_regime_changes() {
    // Was long, regime turned bearish, short signal fires → immediate flip
    let result = next_position(
        Position::Long,
        &input(-0.01, -0.01, 49_000.0, 50_000.0, true),
        &params(0.005),
    );
    assert_eq!(result, Position::Short, "must flip long→short when regime is bearish and short preds align");
}

#[test]
fn position_flips_short_to_long_when_regime_changes() {
    let result = next_position(
        Position::Short,
        &input(0.01, 0.01, 51_000.0, 50_000.0, true),
        &params(0.005),
    );
    assert_eq!(result, Position::Long, "must flip short→long when regime is bullish and long preds align");
}

#[test]
fn invalid_sma_blocks_all_new_entries_and_holds_current() {
    let p = params(0.005);
    // Strong long signal but SMA not yet valid
    let stays_flat = next_position(
        Position::Flat,
        &input(0.1, 0.1, 60_000.0, 50_000.0, false),
        &p,
    );
    assert_eq!(stays_flat, Position::Flat);

    // Already long, strong short signal, SMA invalid → hold Long
    let stays_long = next_position(
        Position::Long,
        &input(-0.1, -0.1, 40_000.0, 50_000.0, false),
        &p,
    );
    assert_eq!(stays_long, Position::Long);
}

// ---------------------------------------------------------------------------
// Multi-step sequence — mirrors Colab regime-filtered hysteresis backtester
// ---------------------------------------------------------------------------

/// Simulate `n` steps through the state machine given parallel slices of
/// predictions, closes, and an SMA time-series. Returns the final position
/// and a vec of per-step positions.
fn run_sequence(
    pred_4h: &[f64],
    pred_24h: &[f64],
    closes: &[f64],
    smas: &[(f64, bool)], // (sma, valid) per step
    threshold: f64,
    sma_window: usize,
) -> Vec<Position> {
    let p = StrategyParams { magnitude_threshold: threshold, sma_window };
    let mut pos = Position::Flat;
    let mut history = Vec::with_capacity(pred_4h.len());

    for i in 0..pred_4h.len() {
        let inp = SignalInput {
            pred_4h: pred_4h[i],
            pred_24h: pred_24h[i],
            current_close: closes[i],
            sma: smas[i].0,
            sma_valid: smas[i].1,
        };
        pos = next_position(pos, &inp, &p);
        history.push(pos);
    }
    history
}

#[test]
fn sequence_matches_colab_regime_filtered_hysteresis() {
    // 10-step fixture. SMA = 50_000, threshold = 0.005.
    // Manually verified against the 4-step algorithm.
    //
    // Step | pred_4h | pred_24h | close  | regime | filtered | expected
    // -----+---------+----------+--------+--------+----------+----------
    //  0   |  0.01   |  0.01    | 51_000 |  bull  |    1     | Long
    //  1   |  0.002  |  0.002   | 51_000 |  bull  |    0     | Long  (ffill)
    //  2   | -0.01   | -0.01    | 51_000 |  bull  |    0     | Long  (short blocked by bull regime)
    //  3   | -0.01   | -0.01    | 49_000 |  bear  |   -1     | Short (flip; now bearish)
    //  4   |  0.01   |  0.01    | 49_000 |  bear  |    0     | Short (long blocked by bear regime)
    //  5   |  0.002  | -0.002   | 49_000 |  bear  |    0     | Short (ffill; no aligned signal)
    //  6   | -0.01   | -0.01    | 49_000 |  bear  |   -1     | Short (same direction; no change)
    //  7   |  0.01   |  0.01    | 51_000 |  bull  |    1     | Long  (flip; back to bullish)
    //  8   |  0.0    |  0.0     | 51_000 |  bull  |    0     | Long  (ffill; strictly neutral)
    //  9   | -0.01   | -0.01    | 49_000 |  bear  |   -1     | Short (flip again)

    let pred_4h  = [  0.01,  0.002, -0.01,  -0.01,   0.01,  0.002, -0.01,   0.01,  0.0,  -0.01];
    let pred_24h = [  0.01,  0.002, -0.01,  -0.01,   0.01, -0.002, -0.01,   0.01,  0.0,  -0.01];
    // Steps 0-2: bullish (close=51_000); steps 3-6: bearish (close=49_000);
    // steps 7-8: bullish again; step 9: bearish.
    let closes: Vec<f64> = vec![
        51_000.0, 51_000.0, 51_000.0, // 0-2
        49_000.0, 49_000.0, 49_000.0, 49_000.0, // 3-6
        51_000.0, 51_000.0, // 7-8
        49_000.0, // 9
    ];

    let sma = 50_000.0_f64;
    let smas: Vec<(f64, bool)> = closes.iter().map(|_| (sma, true)).collect();

    let history = run_sequence(&pred_4h, &pred_24h, &closes, &smas, 0.005, 200);

    let expected = [
        Position::Long,   // 0
        Position::Long,   // 1 ffill
        Position::Long,   // 2 short blocked by bull
        Position::Short,  // 3 flip
        Position::Short,  // 4 long blocked by bear
        Position::Short,  // 5 ffill
        Position::Short,  // 6 same direction
        Position::Long,   // 7 flip
        Position::Long,   // 8 ffill
        Position::Short,  // 9 flip
    ];

    for (i, (got, exp)) in history.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, exp,
            "step {i}: expected {exp}, got {got}"
        );
    }
}

#[test]
fn sequence_no_trades_below_threshold() {
    // All predictions are below threshold → always Flat
    let pred = vec![0.003; 10];
    let closes = vec![51_000.0; 10];
    let smas: Vec<(f64, bool)> = vec![(50_000.0, true); 10];

    let history = run_sequence(&pred, &pred, &closes, &smas, 0.005, 200);
    assert!(
        history.iter().all(|p| *p == Position::Flat),
        "all positions should be Flat when predictions are below threshold"
    );
}

#[test]
fn sequence_no_trades_when_sma_invalid() {
    // Valid signals but SMA invalid throughout → always Flat
    let pred = vec![0.01; 5];
    let closes = vec![51_000.0; 5];
    let smas: Vec<(f64, bool)> = vec![(50_000.0, false); 5]; // sma_valid = false

    let history = run_sequence(&pred, &pred, &closes, &smas, 0.005, 200);
    assert!(
        history.iter().all(|p| *p == Position::Flat),
        "entries must be blocked when SMA is invalid"
    );
}
