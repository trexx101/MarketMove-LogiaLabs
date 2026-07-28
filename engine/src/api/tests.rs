use crate::{db, strategy};
use super::*;
use axum::extract::State;
use sqlx::sqlite::SqlitePoolOptions;

const TEST_SMA_WINDOW: usize = 3;

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

fn test_state(pool: db::DbPool) -> State<AppState> {
    let (tx, _rx) = tokio::sync::broadcast::channel(64);
    State(AppState {
        pool,
        trading_mode: crate::config::TradingMode::Paper,
        symbol: "BTC/USD".to_string(),
        sma_window: TEST_SMA_WINDOW,
        tx,
    })
}

#[tokio::test]
async fn status_returns_empty_state() {
    let pool = test_pool().await;
    let Json(status) = status::handle_status(test_state(pool)).await.unwrap();

    assert_eq!(status.mode, "paper");
    assert_eq!(status.symbol, "BTC/USD");
    assert_eq!(status.position, "flat");
    assert_eq!(status.realized_pnl, 0.0);
    assert!(status.entry_price.is_none());
    assert!(status.unrealized_pnl.is_none());
    assert!(status.last_candle_ts.is_none());
    assert!(status.pred_1d.is_none());
    assert_eq!(status.staleness_secs, u64::MAX);
}

#[tokio::test]
async fn accuracy_returns_503_when_no_resolved() {
    let pool = test_pool().await;
    let result = predictions::handle_accuracy(test_state(pool)).await;

    let err = result.unwrap_err();
    assert_eq!(err.0, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        err.1.contains("equity accuracy not yet implemented"),
        "got: {}",
        err.1
    );
}

#[tokio::test]
async fn predictions_returns_history() {
    let pool = test_pool().await;
    sqlx::query(
        r#"INSERT INTO equity_predictions
               (symbol, candle_ts, pred_1d, pred_5d, pred_21d, regime, features_json, created_at, source)
           VALUES ('BTC/USD', 1_000_000, 0.0065, 0.0325, 0.15, 'bull', '[]', 1_000_000, 'test')"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let Json(resp) = predictions::handle_predictions(test_state(pool)).await.unwrap();
    assert!(resp.latest.is_some());
    assert_eq!(resp.history.len(), 1);
    let latest = resp.latest.unwrap();
    assert!((latest.pred_1h - 0.001).abs() < 1e-9);
    assert!((latest.pred_4h - 0.004).abs() < 1e-9);
    assert!((latest.pred_24h - 0.0065).abs() < 1e-9);
}

#[test]
fn prediction_dto_computes_approx_fields() {
    let row = db::PredictionRow {
        id: 1,
        candle_ts: 1_000_000,
        pred_1h: 0.01,
        pred_4h: 0.02,
        pred_24h: 0.026,
        features_json: "[]".to_string(),
        created_at: 1_000_000,
        actual_1h: None,
        actual_4h: None,
        actual_24h: None,
    };
    let dto = predictions::prediction_to_dto(&row);
    let expected_1h = 0.026 / 6.5;
    let expected_5h = 0.026 * (5.0 / 6.5);
    assert!((dto.pred_1h_approx - expected_1h).abs() < 1e-12);
    assert!(dto.pred_1h_approx > 0.0039 && dto.pred_1h_approx < 0.0041);
    assert!((dto.pred_5h_approx - expected_5h).abs() < 1e-12);
    assert!(dto.pred_5h_approx > 0.019 && dto.pred_5h_approx < 0.021);
}

#[test]
fn prediction_dto_handles_negative_pred() {
    let row = db::PredictionRow {
        id: 1,
        candle_ts: 1_000_000,
        pred_1h: -0.005,
        pred_4h: -0.008,
        pred_24h: -0.013,
        features_json: "[]".to_string(),
        created_at: 1_000_000,
        actual_1h: None,
        actual_4h: None,
        actual_24h: None,
    };
    let dto = predictions::prediction_to_dto(&row);
    let expected_1h = -0.013 / 6.5;
    let expected_5h = -0.013 * (5.0 / 6.5);
    assert!((dto.pred_1h_approx - expected_1h).abs() < 1e-12);
    assert!(dto.pred_1h_approx < -0.0019 && dto.pred_1h_approx > -0.0021);
    assert!((dto.pred_5h_approx - expected_5h).abs() < 1e-12);
    assert!(dto.pred_5h_approx < -0.009 && dto.pred_5h_approx > -0.011);
}

#[tokio::test]
async fn chart_computes_rolling_sma() {
    let pool = test_pool().await;
    let closes = [100.0, 102.0, 101.0, 103.0, 104.0];
    for (i, close) in closes.iter().enumerate() {
        let ts = 1_000_000 + i as i64 * 3_600;
        db::upsert_equity_candle(
            &pool,
            &db::EquityCandle {
                symbol: "BTC/USD".to_string(),
                ts,
                open: *close,
                high: *close,
                low: *close,
                close: *close,
                volume: 1,
                source: "test".to_string(),
            },
        )
        .await
        .unwrap();
    }

    let Json(resp) = chart::handle_chart(test_state(pool)).await.unwrap();
    let expected_sma_points = closes.len().saturating_sub(TEST_SMA_WINDOW - 1);
    assert_eq!(resp.candles.len(), closes.len());
    assert_eq!(resp.sma.len(), expected_sma_points);
    assert_eq!(
        resp.sma.first().unwrap().ts,
        resp.candles[TEST_SMA_WINDOW - 1].ts
    );
}

#[tokio::test]
async fn status_reports_unrealized_pnl_for_open_position() {
    let pool = test_pool().await;
    db::save_position(&pool, strategy::Position::Long.as_i64())
        .await
        .unwrap();
    db::insert_trade(&pool, 1_000_000, "buy", 1.0, 100.0, 0.0, 0.0)
        .await
        .unwrap();
    db::upsert_equity_candle(
        &pool,
        &db::EquityCandle {
            symbol: "BTC/USD".to_string(),
            ts: 1_000_000,
            open: 110.0,
            high: 110.0,
            low: 110.0,
            close: 110.0,
            volume: 1,
            source: "test".to_string(),
        },
    )
    .await
    .unwrap();

    let Json(status) = status::handle_status(test_state(pool)).await.unwrap();
    assert_eq!(status.position, "long");
    assert!((status.entry_price.unwrap() - 100.0).abs() < 1e-9);
    assert!((status.unrealized_pnl.unwrap() - 10.0).abs() < 1e-9);
}

#[tokio::test(flavor = "current_thread")]
async fn router_serves_static_files_and_api() {
    use tokio::net::TcpListener;

    let original_cwd = std::env::current_dir().unwrap();
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    std::env::set_current_dir(workspace_root).unwrap();

    let result = async {
        let pool = test_pool().await;
        let config = crate::config::Config {
            trading_mode: crate::config::TradingMode::Paper,
            zmq_endpoint: "tcp://127.0.0.1:5555".to_string(),
            magnitude_threshold: 0.005,
            paper_fee: 0.0015,
            sma_window: 3,
            http_port: 0,
            symbol: "BTC/USD".to_string(),
            database_url: ":memory:".to_string(),
            norm_stats_path: "models/norm_stats.json".to_string(),
            feature_window_size: 72,
            parity_marker_path: std::env::temp_dir()
                .join("parity_marker_api_test.json")
                .to_string_lossy()
                .into_owned(),
            parity_max_age_secs: 7 * 24 * 60 * 60,
            moomoo_creds_path: "~/.moomoo/credentials.json".to_string(),
            fred_api_key: "".to_string(),
        };

        let app = {
            let (tx, _rx) = tokio::sync::broadcast::channel(64);
            router(pool, &config, tx)
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        let base = format!("http://{}", addr);

        let status_res = client
            .get(format!("{base}/api/status"))
            .send()
            .await
            .unwrap();
        assert_eq!(status_res.status(), 200);
        let status: serde_json::Value = status_res.json().await.unwrap();
        assert_eq!(status["mode"], "paper");
        assert_eq!(status["position"], "flat");

        let index_res = client.get(format!("{base}/")).send().await.unwrap();
        assert_eq!(index_res.status(), 200);
        let ct = index_res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/html"));
        let body = index_res.text().await.unwrap();
        assert!(
            body.contains("MarketMoves") || body.contains("MarketMarkovNet"),
            "index.html should contain app title"
        );

        let spa_res = client
            .get(format!("{base}/some/unknown/route"))
            .send()
            .await
            .unwrap();
        let spa_status = spa_res.status();
        let spa_body = spa_res.text().await.unwrap();
        assert!(
            spa_body.contains("MarketMoves") || spa_body.contains("MarketMarkovNet"),
            "SPA fallback should serve index.html, got status {} body: {}",
            spa_status,
            spa_body
        );
    }
    .await;

    std::env::set_current_dir(original_cwd).unwrap();
    result
}
