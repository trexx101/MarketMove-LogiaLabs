//! End-to-end parity harness integration tests (Feature 13).
//!
//! Loads the 7-day (168-candle) golden fixture committed at
//! `tests/fixtures/parity_golden_168h.json`, runs the Rust pipeline
//! offline, asserts per-stage parity within tolerance, writes a
//! `parity_verified.json` marker, and verifies the marker is round-tripable.
//!
//! This test also covers acceptance criteria 1–4 from
//! `plans/market-markov-net/features/13 - regression parity harness.md`:
//!   1. Feature parity within tolerance across the 7-day window.
//!   2. Prediction parity within tolerance.
//!   3. Entry/exit timestamps + directions match the golden exactly.
//!   4. Parity report generated; `parity_verified` marker written on success.
//!
//! Note: the committed fixture is a Rust-generated placeholder. The real
//! Colab export replaces it before the live-mode guard is trusted.

use std::env;
use std::path::PathBuf;

use chrono::Utc;
use engine::parity::{
    read_marker, run_parity, write_marker, GoldenFixture, ParityMarker, ParityReport, Verdict,
};

/// Path to the 7-day golden fixture, relative to the workspace root.
fn fixture_path() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is the engine crate root; fixture lives at the
    // workspace root.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .expect("workspace root")
        .join("tests/fixtures/parity_golden_168h.json")
}

#[test]
fn golden_fixture_loads_and_has_168_candles() {
    let path = fixture_path();
    let fixture = GoldenFixture::load(path.to_str().unwrap()).expect("load golden fixture");
    assert_eq!(fixture.candles.len(), 168, "7-day window must have 168 hourly candles");
    assert_eq!(fixture.predictions.len(), 168);
    assert_eq!(fixture.expected_features.len(), 168);
    assert_eq!(fixture.expected_signals.len(), 168);
    assert_eq!(fixture.sma_window, 200);
    assert!((fixture.magnitude_threshold - 0.005).abs() < 1e-12);
    assert_eq!(fixture.feature_window_size, 72);
}

#[test]
fn seven_day_parity_check_passes_within_tolerance() {
    let path = fixture_path();
    let fixture = GoldenFixture::load(path.to_str().unwrap()).expect("load golden fixture");

    // Convert GoldenCandle → engine::db::Candle for the harness.
    let candles: Vec<engine::db::Candle> = fixture
        .candles
        .iter()
        .map(|c| engine::db::Candle {
            ts: c.ts,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            vwap: c.vwap,
        })
        .collect();

    // Acceptance criterion 1 — feature parity within tolerance.
    // Acceptance criterion 2 — prediction parity within tolerance.
    // Acceptance criterion 3 — signal parity (exact match).
    // Acceptance criterion 4 — report generated (ParityReport returned).
    let report: ParityReport = run_parity(&candles, &fixture.predictions, &fixture, 1e-6);

    // Emit a human-readable report to stdout (cargo test -- --nocapture).
    println!("\n=== Parity Report ===");
    println!("verdict:     {:?}", report.verdict);
    println!("tolerance:   {}", report.tolerance);
    println!("candles:     {}", report.candles_compared);
    println!("predictions: {}", report.predictions_compared);
    println!("features:    compared={} max_abs_error={:.3e}", report.features.compared, report.features.max_abs_error);
    println!("predictions: compared={} max_abs_error={:.3e}", report.predictions.compared, report.predictions.max_abs_error);
    println!("signals:     compared={} max_abs_error={:.3e}", report.signals.compared, report.signals.max_abs_error);
    println!("worst:       {:.3e}", report.worst_abs_error());
    println!("notes:       {}", report.notes);
    println!("fixture:     sha256:{}", &report.fixture_sha256[..16]);
    println!();

    assert_eq!(
        report.verdict,
        Verdict::Pass,
        "parity must pass on the 7-day golden fixture (notes: {})",
        report.notes
    );
    assert_eq!(report.candles_compared, 168);
    assert_eq!(report.predictions_compared, 168);
    assert!(report.features.max_abs_error < 1e-6, "feature max abs error must be < 1e-6");
    assert!(report.predictions.max_abs_error < 1e-6, "prediction max abs error must be < 1e-6");
    assert!(report.signals.max_abs_error < 1e-6, "signals must be exact (within tolerance)");
    assert_eq!(report.fixture_sha256, fixture.sha256());
}

#[test]
fn seven_day_parity_writes_and_reads_marker() {
    let path = fixture_path();
    let fixture = GoldenFixture::load(path.to_str().unwrap()).expect("load golden fixture");
    let candles: Vec<engine::db::Candle> = fixture
        .candles
        .iter()
        .map(|c| engine::db::Candle {
            ts: c.ts,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            vwap: c.vwap,
        })
        .collect();
    let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-6);
    assert_eq!(report.verdict, Verdict::Pass);

    let marker = ParityMarker::from_report(&report, Utc::now().timestamp());

    // Write the marker to a temp file (don't pollute the workspace root
    // during tests; the live-mode guard uses the workspace-root marker).
    let marker_path = env::temp_dir().join("parity_marker_harness_test.json");
    write_marker(&marker_path, &marker).expect("write marker");
    let loaded = read_marker(&marker_path).expect("read marker").expect("marker exists");

    assert_eq!(loaded.verified_at, marker.verified_at);
    assert_eq!(loaded.fixture_sha256, marker.fixture_sha256);
    assert_eq!(loaded.candles_compared, 168);
    assert!(loaded.max_abs_error < 1e-6);
    assert_eq!(loaded.tolerance, 1e-6);
    assert!(loaded.is_fresh(Utc::now().timestamp(), 7 * 24 * 3600));

    let _ = std::fs::remove_file(&marker_path);
}

#[test]
fn parity_check_fails_when_expected_features_diverge() {
    // Sanity check: if we corrupt the fixture's expected features, the
    // harness must catch it. This mirrors the per-component feature
    // parity test but exercises the full end-to-end path with a 7-day
    // fixture.
    let path = fixture_path();
    let mut fixture = GoldenFixture::load(path.to_str().unwrap()).expect("load golden fixture");

    // Push one feature well outside the 1e-6 tolerance.
    fixture.expected_features[100].log_return += 1.5;

    let candles: Vec<engine::db::Candle> = fixture
        .candles
        .iter()
        .map(|c| engine::db::Candle {
            ts: c.ts,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            vwap: c.vwap,
        })
        .collect();
    let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-6);
    assert_eq!(report.verdict, Verdict::Fail);
    assert!(report.features.max_abs_error > 1.0);
}

#[test]
fn parity_marker_rejects_stale_marker_in_live_mode() {
    // The live-mode guard in engine::config::Config::from_env rejects
    // markers older than PARITY_MAX_AGE_SECS. ParityMarker::is_fresh
    // encapsulates that check.
    let path = fixture_path();
    let fixture = GoldenFixture::load(path.to_str().unwrap()).expect("load golden fixture");
    let candles: Vec<engine::db::Candle> = fixture
        .candles
        .iter()
        .map(|c| engine::db::Candle {
            ts: c.ts,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            vwap: c.vwap,
        })
        .collect();
    let report = run_parity(&candles, &fixture.predictions, &fixture, 1e-6);

    // A marker verified 8 days ago is stale against the 7-day default.
    let eight_days_ago = Utc::now().timestamp() - 8 * 24 * 3600;
    let stale = ParityMarker::from_report(&report, eight_days_ago);
    assert!(!stale.is_fresh(Utc::now().timestamp(), 7 * 24 * 3600));

    // A marker verified 1 hour ago is fresh.
    let one_hour_ago = Utc::now().timestamp() - 3600;
    let fresh = ParityMarker::from_report(&report, one_hour_ago);
    assert!(fresh.is_fresh(Utc::now().timestamp(), 7 * 24 * 3600));
}
