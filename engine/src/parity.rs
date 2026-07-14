//! Regression / Parity Harness — Feature 13.
//!
//! Compares the Rust feature + strategy pipeline against a "golden" reference
//! (typically exported from the Colab training notebook) over a fixed window
//! of historical candles, and writes a `parity_verified` marker that the
//! live-mode guard in [`crate::config`] checks before allowing `TRADING_MODE=live`.
//!
//! ## Layout
//!
//! 1. **Golden fixture** (`GoldenFixture`) — a JSON file holding:
//!    - `candles` — the input OHLCV + VWAP series.
//!    - `predictions` — recorded inference responses, one per candle.
//!    - `expected_features` — Colab-derived per-candle features.
//!    - `expected_signals` — Colab-derived per-candle position state.
//! 2. **`run_parity`** — re-runs the Rust pipeline offline and diffs against
//!    the golden fixture with a configurable tolerance.
//! 3. **`ParityReport`** — Pass/Fail summary with per-stage max absolute error.
//! 4. **`ParityMarker`** — small JSON file (`parity_verified.json`) written
//!    on success. Consumed by the live-mode guard.
//!
//! ## Why a recorded inference response?
//!
//! The bridge is decoupled from the parity check. The harness can use a
//! real ZMQ service (in CI) or a recorded prediction set (the default in
//! this fixture). Recording eliminates non-determinism from the model and
//! isolates the parity comparison to the *Rust implementation* of the
//! features + strategy math, which is what the spec asks for.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::Candle;
use crate::features::compute_features;
use crate::strategy::{compute_sma, next_position, Position, SignalInput, StrategyParams};

// ---------------------------------------------------------------------------
// Golden fixture
// ---------------------------------------------------------------------------

/// A single OHLCV + VWAP candle as it appears in the golden JSON fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenCandle {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub vwap: f64,
}

impl From<GoldenCandle> for Candle {
    fn from(g: GoldenCandle) -> Self {
        Candle {
            ts: g.ts,
            open: g.open,
            high: g.high,
            low: g.low,
            close: g.close,
            volume: g.volume,
            vwap: g.vwap,
        }
    }
}

/// A recorded inference response for one candle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenPrediction {
    pub candle_ts: i64,
    pub pred_1h: f64,
    pub pred_4h: f64,
    pub pred_24h: f64,
}

/// Colab-derived per-candle features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenFeature {
    pub candle_ts: i64,
    pub log_return: f64,
    pub atr_72: f64,
    pub vwap_dev: f64,
}

/// Colab-derived per-candle position state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenSignal {
    pub candle_ts: i64,
    /// Integer encoding: 0 = Flat, 1 = Long, -1 = Short.
    pub position: i64,
}

/// The full golden fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenFixture {
    /// Strategy parameters used to produce `expected_signals`.
    pub magnitude_threshold: f64,
    /// SMA window (in candles).
    pub sma_window: usize,
    /// `feature_window_size` — the count of normalized features that the
    /// inference service expects. Recorded for completeness.
    pub feature_window_size: usize,
    pub candles: Vec<GoldenCandle>,
    pub predictions: Vec<GoldenPrediction>,
    pub expected_features: Vec<GoldenFeature>,
    pub expected_signals: Vec<GoldenSignal>,
}

impl GoldenFixture {
    /// Load a golden fixture from `path`.
    pub fn load(path: &str) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading golden fixture: {path}"))?;
        let fixture: GoldenFixture = serde_json::from_str(&text)
            .with_context(|| format!("parsing golden fixture JSON: {path}"))?;
        Ok(fixture)
    }

    /// Stable SHA-256 over the serialized fixture. Used to bind the marker
    /// to a specific reference: a re-exported fixture invalidates old markers.
    pub fn sha256(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("serialize fixture");
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut out = String::with_capacity(64);
        for b in digest {
            out.push_str(&format!("{:02x}", b));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Parity report
// ---------------------------------------------------------------------------

/// Outcome of a single comparison stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageResult {
    pub name: String,
    pub compared: usize,
    pub max_abs_error: f64,
}

impl StageResult {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            compared: 0,
            max_abs_error: 0.0,
        }
    }

    fn observe(&mut self, diff: f64) {
        self.compared += 1;
        if diff > self.max_abs_error {
            self.max_abs_error = diff;
        }
    }
}

/// Overall parity verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
}

/// A full parity report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityReport {
    pub verdict: Verdict,
    pub tolerance: f64,
    pub candles_compared: usize,
    pub predictions_compared: usize,
    pub features: StageResult,
    pub predictions: StageResult,
    pub signals: StageResult,
    /// SHA-256 of the golden fixture that produced this report.
    pub fixture_sha256: String,
    /// Human-readable notes (e.g. "all stages within tolerance").
    pub notes: String,
}

impl ParityReport {
    /// The largest per-stage `max_abs_error` across features, predictions, and signals.
    pub fn worst_abs_error(&self) -> f64 {
        self.features
            .max_abs_error
            .max(self.predictions.max_abs_error)
            .max(self.signals.max_abs_error)
    }
}

// ---------------------------------------------------------------------------
// run_parity — the core harness
// ---------------------------------------------------------------------------

/// Run the parity harness offline.
///
/// - `candles` — the input candle series (typically loaded from the fixture).
/// - `predictions` — recorded inference responses, ordered by `candle_ts` ascending.
/// - `golden` — the reference (expected_features, expected_signals, params).
/// - `tolerance` — max absolute error allowed for each stage (e.g. `1e-6`).
pub fn run_parity(
    candles: &[Candle],
    predictions: &[GoldenPrediction],
    golden: &GoldenFixture,
    tolerance: f64,
) -> ParityReport {
    let params = StrategyParams {
        magnitude_threshold: golden.magnitude_threshold,
        sma_window: golden.sma_window,
    };

    // 1) Features --------------------------------------------------------
    let features = compute_features(candles);
    let mut feat_stage = StageResult::new("features");
    let min_len = features.len().min(golden.expected_features.len());
    for (i, (got, exp)) in features
        .iter()
        .zip(golden.expected_features.iter())
        .enumerate()
        .take(min_len)
    {
        debug_assert!(got_vs_fixture_ts(candles, golden, i, exp.candle_ts));
        feat_stage.observe((got.log_return - exp.log_return).abs());
        feat_stage.observe((got.atr_72 - exp.atr_72).abs());
        feat_stage.observe((got.vwap_dev - exp.vwap_dev).abs());
    }

    // 2) Predictions -----------------------------------------------------
    let mut pred_stage = StageResult::new("predictions");
    let pred_len = predictions.len().min(golden.predictions.len());
    for (got, exp) in predictions
        .iter()
        .zip(golden.predictions.iter())
        .take(pred_len)
    {
        if got.candle_ts != exp.candle_ts {
            pred_stage.observe(f64::INFINITY);
            continue;
        }
        pred_stage.observe((got.pred_1h - exp.pred_1h).abs());
        pred_stage.observe((got.pred_4h - exp.pred_4h).abs());
        pred_stage.observe((got.pred_24h - exp.pred_24h).abs());
    }

    // 3) Signals (regime-filtered hysteresis) ---------------------------
    let mut sig_stage = StageResult::new("signals");
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let sig_len = golden.expected_signals.len().min(candles.len()).min(predictions.len());
    let mut current = Position::Flat;
    for i in 0..sig_len {
        let c = &candles[i];
        let p = &predictions[i];
        let (sma, sma_valid) = compute_sma(&closes[..=i], params.sma_window);

        let input = SignalInput {
            pred_4h: p.pred_4h,
            pred_24h: p.pred_24h,
            current_close: c.close,
            sma,
            sma_valid,
        };
        let got = next_position(current, &input, &params);
        let exp = Position::from_i64(golden.expected_signals[i].position);
        // Position is an exact-match field — any divergence is an error.
        if got != exp {
            sig_stage.observe(f64::INFINITY);
        } else {
            sig_stage.observe(0.0);
        }
        current = got;
    }

    let verdict = if feat_stage.max_abs_error > tolerance
        || pred_stage.max_abs_error > tolerance
        || sig_stage.max_abs_error > tolerance
    {
        Verdict::Fail
    } else {
        Verdict::Pass
    };

    let notes = if verdict == Verdict::Pass {
        format!(
            "all stages within tolerance {} (worst max abs error: {:.3e})",
            tolerance,
            feat_stage
                .max_abs_error
                .max(pred_stage.max_abs_error)
                .max(sig_stage.max_abs_error)
        )
    } else {
        format!(
            "parity FAILED: features={:.3e} predictions={:.3e} signals={:.3e} (tolerance {})",
            feat_stage.max_abs_error,
            pred_stage.max_abs_error,
            sig_stage.max_abs_error,
            tolerance
        )
    };

    ParityReport {
        verdict,
        tolerance,
        candles_compared: min_len,
        predictions_compared: pred_len,
        features: feat_stage,
        predictions: pred_stage,
        signals: sig_stage,
        fixture_sha256: golden.sha256(),
        notes,
    }
}

/// Helper: assert that the candle at `i` and the golden feature at `i` share
/// the same timestamp. Kept as a separate function so the `debug_assert!`
/// above stays readable.
#[inline]
fn got_vs_fixture_ts(candles: &[Candle], golden: &GoldenFixture, i: usize, exp_ts: i64) -> bool {
    candles.get(i).map(|c| c.ts == exp_ts).unwrap_or(false)
        && golden
            .expected_features
            .get(i)
            .map(|f| f.candle_ts == exp_ts)
            .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Parity marker (file-based, consumed by the live-mode guard)
// ---------------------------------------------------------------------------

/// Marker file written on a clean parity run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityMarker {
    /// Unix timestamp of the successful run (UTC seconds).
    pub verified_at: i64,
    /// SHA-256 of the golden fixture that produced this marker.
    pub fixture_sha256: String,
    /// Number of candles that were compared.
    pub candles_compared: usize,
    /// Worst-case per-stage max absolute error observed.
    pub max_abs_error: f64,
    /// Tolerance used for the comparison.
    pub tolerance: f64,
    /// Human-readable verdict note.
    pub notes: String,
}

impl ParityMarker {
    /// Maximum age (in seconds) before a marker is considered stale. Default = 7 days.
    pub const DEFAULT_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

    /// True if `now - verified_at` is within `max_age_secs`.
    pub fn is_fresh(&self, now: i64, max_age_secs: i64) -> bool {
        (now - self.verified_at).abs() <= max_age_secs
    }

    /// Build a marker from a `ParityReport` and the time of verification.
    pub fn from_report(report: &ParityReport, verified_at: i64) -> Self {
        Self {
            verified_at,
            fixture_sha256: report.fixture_sha256.clone(),
            candles_compared: report.candles_compared,
            max_abs_error: report.worst_abs_error(),
            tolerance: report.tolerance,
            notes: report.notes.clone(),
        }
    }
}

/// Write a marker to `path`. Parent directories are created if needed.
pub fn write_marker(path: &Path, marker: &ParityMarker) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating marker parent dir: {:?}", parent))?;
        }
    }
    let json = serde_json::to_string_pretty(marker).context("serializing ParityMarker")?;
    std::fs::write(path, json).with_context(|| format!("writing marker to {path:?}"))?;
    Ok(())
}

/// Read a marker from `path`. Returns `Ok(None)` if the file does not exist.
pub fn read_marker(path: &Path) -> Result<Option<ParityMarker>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    let marker: ParityMarker = serde_json::from_str(&text)
        .with_context(|| format!("parsing ParityMarker from {path:?}"))?;
    Ok(Some(marker))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn synth_candle(ts: i64, close: f64) -> GoldenCandle {
        GoldenCandle {
            ts,
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 100.0,
            vwap: close,
        }
    }

    fn build_fixture(n: usize, threshold: f64, sma_window: usize) -> GoldenFixture {
        // Synthetic linear-up walk: close = 100 + 0.1 * i
        let candles: Vec<GoldenCandle> = (0..n)
            .map(|i| {
                let close = 100.0 + 0.1 * (i as f64);
                synth_candle(i as i64 * 3600, close)
            })
            .collect();

        // Run the Rust pipeline to get a self-consistent golden fixture.
        let rust_candles: Vec<Candle> = candles.iter().cloned().map(Into::into).collect();
        let features = compute_features(&rust_candles);
        let closes: Vec<f64> = rust_candles.iter().map(|c| c.close).collect();
        let expected_features: Vec<GoldenFeature> = features
            .iter()
            .zip(candles.iter())
            .map(|(f, c)| GoldenFeature {
                candle_ts: c.ts,
                log_return: f.log_return,
                atr_72: f.atr_72,
                vwap_dev: f.vwap_dev,
            })
            .collect();

        // Synthetic predictions: bullish drift above threshold on every step.
        let predictions: Vec<GoldenPrediction> = candles
            .iter()
            .map(|c| GoldenPrediction {
                candle_ts: c.ts,
                pred_1h: 0.001,
                pred_4h: threshold + 0.001,
                pred_24h: threshold + 0.002,
            })
            .collect();

        // Run the strategy to derive signals.
        let params = StrategyParams {
            magnitude_threshold: threshold,
            sma_window,
        };
        let mut current = Position::Flat;
        let mut expected_signals: Vec<GoldenSignal> = Vec::with_capacity(n);
        for (i, c) in candles.iter().enumerate() {
            let p = &predictions[i];
            let (sma, sma_valid) = compute_sma(&closes[..=i], sma_window);
            let input = SignalInput {
                pred_4h: p.pred_4h,
                pred_24h: p.pred_24h,
                current_close: c.close,
                sma,
                sma_valid,
            };
            let pos = next_position(current, &input, &params);
            expected_signals.push(GoldenSignal {
                candle_ts: c.ts,
                position: pos.as_i64(),
            });
            current = pos;
        }

        GoldenFixture {
            magnitude_threshold: threshold,
            sma_window,
            feature_window_size: 72,
            candles,
            predictions,
            expected_features,
            expected_signals,
        }
    }

    #[test]
    fn golden_fixture_round_trips_json() {
        let fixture = build_fixture(20, 0.005, 10);
        let json = serde_json::to_string(&fixture).unwrap();
        let back: GoldenFixture = serde_json::from_str(&json).unwrap();
        assert_eq!(back.candles.len(), 20);
        assert_eq!(back.predictions.len(), 20);
        assert_eq!(back.expected_features.len(), 20);
        assert_eq!(back.expected_signals.len(), 20);
        assert!((back.magnitude_threshold - 0.005).abs() < 1e-12);
        assert_eq!(back.sma_window, 10);
    }

    #[test]
    fn golden_fixture_sha256_is_stable() {
        let fixture = build_fixture(20, 0.005, 10);
        let h1 = fixture.sha256();
        let h2 = fixture.sha256();
        assert_eq!(h1, h2, "sha256 must be deterministic");
        assert_eq!(h1.len(), 64, "sha256 hex must be 64 chars");
    }

    #[test]
    fn run_parity_passes_on_self_consistent_fixture() {
        let fixture = build_fixture(20, 0.005, 10);
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-9);
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.candles_compared, 20);
        assert_eq!(report.predictions_compared, 20);
        assert!(report.features.max_abs_error < 1e-9);
        assert!(report.predictions.max_abs_error < 1e-9);
        assert!(report.signals.max_abs_error < 1e-9);
    }

    #[test]
    fn run_parity_fails_when_features_diverge() {
        let mut fixture = build_fixture(20, 0.005, 10);
        // Corrupt one expected feature value by an amount well above tolerance.
        fixture.expected_features[5].log_return += 1.5;
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-9);
        assert_eq!(report.verdict, Verdict::Fail);
        assert!(report.features.max_abs_error > 1.0);
    }

    #[test]
    fn run_parity_fails_when_signal_diverges() {
        let mut fixture = build_fixture(20, 0.005, 10);
        // Flip a signal entry where the Rust pipeline produces Long.
        // With sma_window=10 and threshold=0.005, the first valid entry is at
        // index 9. We flip the *expected* to 0 (Flat); the Rust pipeline
        // produces Long → mismatch.
        fixture.expected_signals[9].position = 0;
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-9);
        assert_eq!(report.verdict, Verdict::Fail);
        assert!(report.signals.max_abs_error.is_infinite());
    }

    #[test]
    fn run_parity_fails_on_timestamp_misalignment() {
        // Build a self-consistent fixture, then create a SEPARATE prediction
        // vector with one timestamp shifted. The harness must reject this.
        let fixture = build_fixture(20, 0.005, 10);
        let mut mismatched_preds = fixture.predictions.clone();
        mismatched_preds[3].candle_ts += 1;
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &mismatched_preds, &fixture, 1e-9);
        assert_eq!(report.verdict, Verdict::Fail);
        assert!(report.predictions.max_abs_error.is_infinite());
    }

    #[test]
    fn marker_from_report_carries_fields() {
        let fixture = build_fixture(20, 0.005, 10);
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-9);
        let marker = ParityMarker::from_report(&report, 1_700_000_000);
        assert_eq!(marker.verified_at, 1_700_000_000);
        assert_eq!(marker.fixture_sha256, fixture.sha256());
        assert_eq!(marker.candles_compared, 20);
        assert!((marker.tolerance - 1e-9).abs() < 1e-18);
        assert!(marker.max_abs_error < 1e-9);
    }

    #[test]
    fn marker_is_fresh_within_window() {
        let fixture = build_fixture(20, 0.005, 10);
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-9);
        let marker = ParityMarker::from_report(&report, 1_000_000);
        // 0 seconds elapsed: fresh
        assert!(marker.is_fresh(1_000_000, 60));
        // 1 hour elapsed: still fresh
        assert!(marker.is_fresh(1_000_000 + 3600, 24 * 3600));
        // 30 days elapsed, 7-day window: stale
        assert!(!marker.is_fresh(1_000_000 + 30 * 24 * 3600, 7 * 24 * 3600));
    }

    #[test]
    fn write_and_read_marker_round_trip() {
        let fixture = build_fixture(20, 0.005, 10);
        let candles: Vec<Candle> = fixture.candles.iter().cloned().map(Into::into).collect();
        let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-9);
        let marker = ParityMarker::from_report(&report, 1_700_000_000);

        let path = env::temp_dir().join("parity_marker_test.json");
        write_marker(&path, &marker).expect("write marker");
        let loaded = read_marker(&path).expect("read marker").expect("marker exists");
        assert_eq!(loaded.verified_at, marker.verified_at);
        assert_eq!(loaded.fixture_sha256, marker.fixture_sha256);
        assert_eq!(loaded.candles_compared, marker.candles_compared);
        assert_eq!(loaded.notes, marker.notes);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_marker_returns_none_for_missing_file() {
        let path = env::temp_dir().join("parity_marker_does_not_exist_xyz.json");
        let _ = std::fs::remove_file(&path); // best-effort
        let result = read_marker(&path).expect("read_marker ok");
        assert!(result.is_none());
    }
}
