//! Phase 3.4 integration tests for the runtime paper/live mode toggle API.
//!
//! Exercises the full `POST /api/mode` flow against an in-process Axum server,
//! including:
//!
//! - TOTP validation (valid code accepted, invalid rejected)
//! - Parity marker freshness check (stale marker blocks live-mode flips)
//! - Mode flip from paper -> live
//! - Audit row appended to `mode_switches`
//! - `GET /api/mode` returns the current mode
//!
//! Each test starts with a fresh in-memory SQLite DB seeded with a fresh
//! parity marker. The TOTP secret is generated per-test so we can compute a
//! valid current code without time-machine shenanigans.

use std::sync::Arc;
use tokio::sync::RwLock;

use engine::api::{router, AppState};
use engine::config::{Config, TradingMode};
use engine::db;
use engine::parity::{write_marker, ParityMarker};
use engine::totp;
use sqlx::sqlite::SqlitePoolOptions;

const FRESH_PARITY: &str = "engine/tests/fixtures/parity_mode_fresh.json";

fn test_config(totp_secret: String, parity_marker_path: String) -> Config {
    Config {
        trading_mode: TradingMode::Paper,
        zmq_endpoint: "tcp://127.0.0.1:5555".to_string(),
        magnitude_threshold: 0.005,
        paper_fee: 0.0015,
        sma_window: 200,
        enable_shorting: false,
        short_entry_threshold: -0.004,
        short_exit_threshold: 0.001,
        pred_5d_filter: false,
        short_pred_5d_filter: false,
        http_port: 0,
        symbol: "QQQ".to_string(),
        short_symbol: "PSQ".to_string(),
        database_url: ":memory:".to_string(),
        norm_stats_path: "models/norm_stats.json".to_string(),
        feature_window_size: 72,
        parity_marker_path,
        parity_max_age_secs: 7 * 24 * 60 * 60,
        moomoo_creds_path: "~/.moomoo/credentials.json".to_string(),
        fred_api_key: String::new(),
        live_executor: "paper".to_string(),
        moomoo_trd_env: "SIMULATE".to_string(),
        totp_secret,
    }
}

async fn test_pool() -> db::DbPool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for stmt in db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(stmt).execute(&pool).await.unwrap();
    }
    db::migrate_predictions(&pool).await.unwrap();
    pool
}

fn write_fresh_marker(path: &str) {
    let marker = ParityMarker {
        verified_at: chrono::Utc::now().timestamp() - 60, // 1 minute ago
        fixture_sha256: "abc".to_string(),
        candles_compared: 168,
        max_abs_error: 1e-9,
        tolerance: 1e-6,
        notes: "fresh marker for mode toggle test".to_string(),
    };
    write_marker(std::path::Path::new(path), &marker).expect("write fresh marker");
}

fn write_stale_marker(path: &str) {
    let marker = ParityMarker {
        verified_at: chrono::Utc::now().timestamp() - 30 * 24 * 3600, // 30 days ago
        fixture_sha256: "abc".to_string(),
        candles_compared: 168,
        max_abs_error: 1e-9,
        tolerance: 1e-6,
        notes: "stale marker".to_string(),
    };
    write_marker(std::path::Path::new(path), &marker).expect("write stale marker");
}

fn build_app(
    pool: db::DbPool,
    cfg: &Config,
) -> axum::Router {
    let (tx, _rx) = tokio::sync::broadcast::channel(64);
    let trading_mode = Arc::new(RwLock::new(cfg.trading_mode));
    let event_logger = Arc::new(engine::event::EventLogger::new(
        pool.clone(),
        Some(tx.clone()),
        trading_mode.clone(),
    ));
    let state = AppState {
        pool,
        trading_mode,
        strategy_params: Arc::new(RwLock::new(
            engine::strategy::EquityStrategyParams::default(),
        )),
        symbol: cfg.symbol.clone(),
        short_symbol: cfg.short_symbol.clone(),
        tx,
        event_logger,
        parity_marker_path: cfg.parity_marker_path.clone(),
        parity_max_age_secs: cfg.parity_max_age_secs,
        totp_secret: cfg.totp_secret.clone(),
        zmq_endpoint: cfg.zmq_endpoint.clone(),
        norm_stats_path: cfg.norm_stats_path.clone(),
        advisor: None,
    };
    // Bypass the public `router()` constructor since it does its own
    // AppState construction; we want a fully-wired AppState so the test
    // can read the live trading_mode through the same Arc.
    axum::Router::new()
        .route("/api/mode", axum::routing::get(engine::api::mode::handle_get_mode))
        .route("/api/mode", axum::routing::post(engine::api::mode::handle_set_mode))
        .with_state(state)
}

#[tokio::test(flavor = "current_thread")]
async fn get_mode_returns_paper_by_default() {
    let pool = test_pool().await;
    let secret = totp::generate_secret().unwrap();
    let marker = format!("/tmp/hermes-phase34-fresh-1-{}.json", std::process::id());
    let _ = std::fs::remove_file(&marker);
    write_fresh_marker(&marker);
    let cfg = test_config(secret, marker.clone());
    let app = build_app(pool.clone(), &cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let res = reqwest::Client::new()
        .get(format!("http://{addr}/api/mode"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["mode"], "paper");
    assert_eq!(body["parity_valid"], true);
    assert!(body["parity_marker_age_secs"].is_number());
    let _ = std::fs::remove_file(&marker);
}

#[tokio::test(flavor = "current_thread")]
async fn post_mode_rejects_invalid_totp() {
    let pool = test_pool().await;
    let secret = totp::generate_secret().unwrap();
    let marker = format!("/tmp/hermes-phase34-fresh-2-{}.json", std::process::id());
    let _ = std::fs::remove_file(&marker);
    write_fresh_marker(&marker);
    let cfg = test_config(secret, marker.clone());
    let app = build_app(pool.clone(), &cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let res = reqwest::Client::new()
        .post(format!("http://{addr}/api/mode"))
        .json(&serde_json::json!({ "mode": "live", "auth_token": "000000" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let body = res.text().await.unwrap();
    assert!(body.contains("TOTP"), "expected 'TOTP' in body, got: {body}");
    let _ = std::fs::remove_file(&marker);
}

#[tokio::test(flavor = "current_thread")]
async fn post_mode_rejects_stale_parity_marker() {
    let pool = test_pool().await;
    let secret = totp::generate_secret().unwrap();
    let code = totp::current_code(&secret).unwrap();
    let marker = format!("/tmp/hermes-phase34-stale-{}.json", std::process::id());
    let _ = std::fs::remove_file(&marker);
    write_stale_marker(&marker);
    let cfg = test_config(secret, marker.clone());
    let app = build_app(pool.clone(), &cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let res = reqwest::Client::new()
        .post(format!("http://{addr}/api/mode"))
        .json(&serde_json::json!({ "mode": "live", "auth_token": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let body = res.text().await.unwrap();
    assert!(
        body.contains("parity") || body.contains("old"),
        "expected parity/old in body, got: {body}"
    );
    let _ = std::fs::remove_file(&marker);
}

#[tokio::test(flavor = "current_thread")]
async fn post_mode_paper_to_live_with_valid_totp() {
    let pool = test_pool().await;
    let secret = totp::generate_secret().unwrap();
    let code = totp::current_code(&secret).unwrap();
    let marker = format!("/tmp/hermes-phase34-flip-{}.json", std::process::id());
    let _ = std::fs::remove_file(&marker);
    write_fresh_marker(&marker);
    let cfg = test_config(secret, marker.clone());
    let app = build_app(pool.clone(), &cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // POST /api/mode { mode: "live" }
    let res = reqwest::Client::new()
        .post(format!("http://{addr}/api/mode"))
        .json(&serde_json::json!({ "mode": "live", "auth_token": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["mode"], "live");

    // GET /api/mode should now return "live"
    let res2 = reqwest::Client::new()
        .get(format!("http://{addr}/api/mode"))
        .send()
        .await
        .unwrap();
    assert_eq!(res2.status(), 200);
    let body2: serde_json::Value = res2.json().await.unwrap();
    assert_eq!(body2["mode"], "live");

    // Audit row was appended.
    let recent = db::fetch_recent_mode_switches(&pool, 1).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].previous_mode, "paper");
    assert_eq!(recent[0].new_mode, "live");

    let _ = std::fs::remove_file(&marker);
}

#[tokio::test(flavor = "current_thread")]
async fn post_mode_live_to_paper_does_not_require_parity() {
    // We start in Live mode (config) so flipping to Paper doesn't trigger the
    // parity-marker check. This validates the asymmetric gating: only paper->live
    // requires the marker; live->paper is always allowed.
    let pool = test_pool().await;
    let secret = totp::generate_secret().unwrap();
    let code = totp::current_code(&secret).unwrap();
    let marker = format!("/tmp/hermes-phase34-paper-{}.json", std::process::id());
    let _ = std::fs::remove_file(&marker);
    // No marker written — flipping live->paper must still succeed.
    let mut cfg = test_config(secret, marker.clone());
    cfg.trading_mode = TradingMode::Live;

    let (tx, _rx) = tokio::sync::broadcast::channel(64);
    let trading_mode = Arc::new(RwLock::new(cfg.trading_mode));
    let event_logger = Arc::new(engine::event::EventLogger::new(
        pool.clone(),
        Some(tx.clone()),
        trading_mode.clone(),
    ));
    let state = AppState {
        pool: pool.clone(),
        trading_mode,
        strategy_params: Arc::new(RwLock::new(
            engine::strategy::EquityStrategyParams::default(),
        )),
        symbol: cfg.symbol.clone(),
        short_symbol: cfg.short_symbol.clone(),
        tx,
        event_logger,
        parity_marker_path: cfg.parity_marker_path.clone(),
        parity_max_age_secs: cfg.parity_max_age_secs,
        totp_secret: cfg.totp_secret.clone(),
        zmq_endpoint: cfg.zmq_endpoint.clone(),
        norm_stats_path: cfg.norm_stats_path.clone(),
        advisor: None,
    };
    let app = axum::Router::new()
        .route("/api/mode", axum::routing::get(engine::api::mode::handle_get_mode))
        .route("/api/mode", axum::routing::post(engine::api::mode::handle_set_mode))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let res = reqwest::Client::new()
        .post(format!("http://{addr}/api/mode"))
        .json(&serde_json::json!({ "mode": "paper", "auth_token": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["mode"], "paper");
    let _ = std::fs::remove_file(&marker);
}
