//! Parity harness binary — runs the 168h golden-fixture regression check and
//! writes a `parity_verified.json` marker to the workspace root on success.
//!
//! Usage:
//!   cargo run --bin parity_harness --release
//!
//! The marker is read by the engine's live-mode gate at startup
//! (PARITY_MARKER_PATH env var, default: `parity_verified.json`).

use std::path::PathBuf;

use chrono::Utc;
use engine::parity::{write_marker, GoldenFixture, ParityMarker, ParityReport, Verdict};

fn fixture_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .expect("workspace root")
        .join("tests/fixtures/parity_golden_168h.json")
}

fn main() {
    let fixture_path = fixture_path();
    eprintln!("Loading fixture: {:?}", fixture_path);
    let fixture = GoldenFixture::load(fixture_path.to_str().unwrap())
        .expect("failed to load golden fixture");

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
            funding_rate: 0.0,
            basis_z: 0.0,
            ob_imbalance: 0.0,
        })
        .collect();

    let report: ParityReport =
        engine::parity::run_parity(&candles, &fixture.predictions, &fixture, 1e-6);

    println!(
        "verdict={:?} features_max_abs_error={:.3e} predictions_max_abs_error={:.3e} signals_max_abs_error={:.3e}",
        report.verdict,
        report.features.max_abs_error,
        report.predictions.max_abs_error,
        report.signals.max_abs_error,
    );

    if report.verdict != Verdict::Pass {
        eprintln!("Parity check FAILED — marker NOT written.");
        std::process::exit(1);
    }

    let marker = ParityMarker::from_report(&report, Utc::now().timestamp());

    // Write to workspace root by default; override with PARITY_MARKER_PATH.
    let marker_path = std::env::var("PARITY_MARKER_PATH")
        .unwrap_or_else(|_| "parity_verified.json".to_string());
    let marker_path = PathBuf::from(&marker_path);

    write_marker(&marker_path, &marker)
        .expect("failed to write parity marker");

    println!(
        "Parity marker written to {:?} (sha256: {}…, max_abs_error={:.3e})",
        marker_path,
        &marker.fixture_sha256[..16],
        marker.max_abs_error,
    );
}