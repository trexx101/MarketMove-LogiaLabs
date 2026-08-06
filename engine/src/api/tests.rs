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
    let trading_mode = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::config::TradingMode::Paper,
    ));
    let event_logger = std::sync::Arc::new(crate::event::EventLogger::new(
        pool.clone(),
        Some(tx.clone()),
        trading_mode.clone(),
    ));
    let strategy_params = std::sync::Arc::new(tokio::sync::RwLock::new(
        strategy::EquityStrategyParams {
            entry_threshold: 0.005,
            exit_threshold: -0.0017,
            sma_window: TEST_SMA_WINDOW,
            enable_shorting: false,
            short_entry_threshold: -0.004,
            short_exit_threshold: 0.001,
            pred_5d_filter: true,
            enable_sentiment_overlay: false,
            sentiment_reduce_threshold: -0.5,
            sentiment_exit_threshold: -0.8,
            sentiment_min_articles: 15,
        },
    ));
    State(AppState {
        pool,
        trading_mode,
        strategy_params,
        strategy_params_by_model: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        symbol: "BTC/USD".to_string(),
        short_symbol: "PSQ".to_string(),
        tx,
        event_logger,
        parity_marker_path: std::env::temp_dir()
            .join("parity_marker_test_default.json")
            .to_string_lossy()
            .into_owned(),
        parity_max_age_secs: 7 * 24 * 60 * 60,
        totp_secret: String::new(),
        zmq_endpoint: "tcp://127.0.0.1:5555".to_string(),
        norm_stats_path: String::new(),
        advisor: None,
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
    assert_eq!(status.entry_price, 0.0);
    assert_eq!(status.unrealized_pnl, 0.0);
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

    let query = axum::extract::Query(
        [("symbol".to_string(), "QQQ".to_string())]
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>(),
    );
    let Json(resp) = chart::handle_chart(test_state(pool), query).await.unwrap();
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
    db::insert_equity_trade(&pool, "BTC/USD", 1_000_000, "buy", 1.0, 100.0, 0.0, 0.0)
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
    assert!((status.entry_price - 100.0).abs() < 1e-9);
    assert!((status.unrealized_pnl - 10.0).abs() < 1e-9);
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
            enable_shorting: false,
            short_entry_threshold: -0.004,
            short_exit_threshold: 0.001,
            entry_threshold: 0.005,
            exit_threshold: -0.0017,
            pred_5d_filter: true,
            http_port: 0,
            symbol: "BTC/USD".to_string(),
            short_symbol: "PSQ".to_string(),
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
            finnhub_api_key: "".to_string(),
            live_executor: "paper".to_string(),
            moomoo_trd_env: "SIMULATE".to_string(),
            totp_secret: String::new(),
        };

        let app = {
            let (tx, _rx) = tokio::sync::broadcast::channel(64);
            let trading_mode = std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::config::TradingMode::Paper,
            ));
            let event_logger = std::sync::Arc::new(crate::event::EventLogger::new(
                pool.clone(),
                Some(tx.clone()),
                trading_mode.clone(),
            ));
            router(pool, &config, tx, None, event_logger, std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())))
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

// ---------------------------------------------------------------------------
// §8 Models API tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_models_returns_empty_when_registry_empty() {
    let pool = test_pool().await;
    let Json(models) = models::handle_list_models(test_state(pool)).await.unwrap();
    assert!(models.is_empty(), "fresh registry should have no models");
}

#[tokio::test]
async fn register_then_list_model() {
    let pool = test_pool().await;
    let state = test_state(pool.clone());

    let body = models::RegisterModelBody {
        model_id: "qqq-v1".to_string(),
        primary_symbol: "QQQ".to_string(),
        inverse_symbol: "PSQ".to_string(),
        model_path: "models/qqq_v1.txt".to_string(),
        norm_stats_path: "models/norm_stats_qqq.json".to_string(),
        budget_usd: 10_000.0,
        notes: Some("test model".to_string()),
    };
    let (status, Json(model)) =
        models::handle_register_model(state, axum::Json(body)).await.unwrap();
    assert_eq!(status, axum::http::StatusCode::CREATED);
    assert_eq!(model.model_id, "qqq-v1");
    assert_eq!(model.primary_symbol, "QQQ");
    assert_eq!(model.inverse_symbol, "PSQ");
    assert!(model.enabled);

    // List should now contain 1 model.
    let Json(models) = models::handle_list_models(test_state(pool)).await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "qqq-v1");
}

#[tokio::test]
async fn set_enabled_toggles_flag() {
    let pool = test_pool().await;
    let state = test_state(pool.clone());

    let body = models::RegisterModelBody {
        model_id: "nvda-v1".to_string(),
        primary_symbol: "NVDA".to_string(),
        inverse_symbol: "NVDD".to_string(),
        model_path: "models/nvda_v1.txt".to_string(),
        norm_stats_path: "models/norm_stats_nvda.json".to_string(),
        budget_usd: 5_000.0,
        notes: None,
    };
    models::handle_register_model(state, axum::Json(body))
        .await
        .unwrap();

    // Disable it.
    let Json(model) = models::handle_set_enabled(
        test_state(pool.clone()),
        axum::extract::Path("nvda-v1".to_string()),
        axum::Json(models::SetEnabledBody { enabled: false }),
    )
    .await
    .unwrap();
    assert!(!model.enabled);

    // Re-enable.
    let Json(model) = models::handle_set_enabled(
        test_state(pool.clone()),
        axum::extract::Path("nvda-v1".to_string()),
        axum::Json(models::SetEnabledBody { enabled: true }),
    )
    .await
    .unwrap();
    assert!(model.enabled);
}

#[tokio::test]
async fn set_enabled_returns_404_for_unknown_model() {
    let pool = test_pool().await;
    let result = models::handle_set_enabled(
        test_state(pool),
        axum::extract::Path("nonexistent".to_string()),
        axum::Json(models::SetEnabledBody { enabled: true }),
    )
    .await;
    assert!(result.is_err());
    let (status, _msg) = result.unwrap_err();
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// §8.6 Per-model strategy-config tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn strategy_config_get_without_model_id_uses_default() {
    let pool = test_pool().await;
    let state = test_state(pool);
    let Json(resp) = strategy_config::handle_get(
        state,
        axum::extract::Query(strategy_config::ModelIdQuery { model_id: None }),
    )
    .await
    .unwrap();
    assert!(resp.model_id.is_none(), "no model_id should resolve to default");
    assert_eq!(resp.entry_threshold, 0.005);
}

#[tokio::test]
async fn strategy_config_get_with_unknown_model_id_falls_back_to_default() {
    let pool = test_pool().await;
    let state = test_state(pool);
    let Json(resp) = strategy_config::handle_get(
        state,
        axum::extract::Query(strategy_config::ModelIdQuery {
            model_id: Some("nonexistent".to_string()),
        }),
    )
    .await
    .unwrap();
    // Should fall back to default, model_id in response should be None.
    assert!(resp.model_id.is_none());
}

#[tokio::test]
async fn strategy_config_get_with_known_model_id_returns_per_model_params() {
    let pool = test_pool().await;
    let state = test_state(pool);

    // Register a model and insert a per-model params entry with a custom threshold.
    db::register_model(
        &state.pool,
        "test-q",
        "QQQ",
        "PSQ",
        "models/q.txt",
        "models/norm_q.json",
        10_000.0,
        None,
    )
    .await
    .unwrap();

    let custom_params = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::strategy::EquityStrategyParams {
            entry_threshold: 0.015,
            exit_threshold: -0.005,
            sma_window: 5,
            enable_shorting: true,
            short_entry_threshold: -0.008,
            short_exit_threshold: 0.002,
            pred_5d_filter: false,
            enable_sentiment_overlay: false,
            sentiment_reduce_threshold: -0.5,
            sentiment_exit_threshold: -0.8,
            sentiment_min_articles: 15,
        },
    ));
    state
        .strategy_params_by_model
        .write()
        .await
        .insert("test-q".to_string(), custom_params);

    let Json(resp) = strategy_config::handle_get(
        state,
        axum::extract::Query(strategy_config::ModelIdQuery {
            model_id: Some("test-q".to_string()),
        }),
    )
    .await
    .unwrap();

    assert_eq!(resp.model_id, Some("test-q".to_string()));
    assert_eq!(resp.entry_threshold, 0.015);
    assert_eq!(resp.sma_window, 5);
    assert!(resp.enable_shorting);
}

#[tokio::test]
async fn strategy_config_put_with_model_id_updates_per_model_params() {
    let pool = test_pool().await;
    let state = test_state(pool.clone());

    db::register_model(
        &state.pool,
        "test-nvda",
        "NVDA",
        "NVDD",
        "models/nvda.txt",
        "models/norm_nvda.json",
        5_000.0,
        None,
    )
    .await
    .unwrap();

    // Insert default per-model params.
    let custom_params = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::strategy::EquityStrategyParams {
            entry_threshold: 0.005,
            exit_threshold: -0.0017,
            sma_window: 3,
            enable_shorting: false,
            short_entry_threshold: -0.004,
            short_exit_threshold: 0.001,
            pred_5d_filter: true,
            enable_sentiment_overlay: false,
            sentiment_reduce_threshold: -0.5,
            sentiment_exit_threshold: -0.8,
            sentiment_min_articles: 15,
        },
    ));
    state
        .strategy_params_by_model
        .write()
        .await
        .insert("test-nvda".to_string(), custom_params.clone());

    // PUT to change entry_threshold for this model only.
    let update = strategy_config::StrategyConfigUpdate {
        entry_threshold: Some(0.012),
        exit_threshold: None,
        sma_window: None,
        pred_5d_filter: None,
        enable_shorting: None,
        short_entry_threshold: None,
        short_exit_threshold: None,
        enable_sentiment_overlay: None,
        sentiment_reduce_threshold: None,
        sentiment_exit_threshold: None,
        sentiment_min_articles: None,
    };
    let Json(resp) = strategy_config::handle_put(
        state.clone(),
        axum::extract::Query(strategy_config::ModelIdQuery {
            model_id: Some("test-nvda".to_string()),
        }),
        axum::Json(update),
    )
    .await
    .unwrap();

    assert_eq!(resp.model_id, Some("test-nvda".to_string()));
    assert_eq!(resp.entry_threshold, 0.012);

    // Verify the underlying Arc<RwLock<>> was updated (the one the scheduler holds).
    let sp = custom_params.read().await;
    assert_eq!(sp.entry_threshold, 0.012);
}

#[tokio::test]
async fn strategy_config_sentiment_overlay_update_isolated_per_model() {
    let pool = test_pool().await;
    let state = test_state(pool);

    // Insert default per-model params.
    let custom_params = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::strategy::EquityStrategyParams {
            entry_threshold: 0.005,
            exit_threshold: -0.0017,
            sma_window: 3,
            enable_shorting: false,
            short_entry_threshold: -0.004,
            short_exit_threshold: 0.001,
            pred_5d_filter: true,
            enable_sentiment_overlay: false,
            sentiment_reduce_threshold: -0.5,
            sentiment_exit_threshold: -0.8,
            sentiment_min_articles: 15,
        },
    ));
    state
        .strategy_params_by_model
        .write()
        .await
        .insert("test-sentiment".to_string(), custom_params.clone());

    let update = strategy_config::StrategyConfigUpdate {
        entry_threshold: None,
        exit_threshold: None,
        sma_window: None,
        pred_5d_filter: None,
        enable_shorting: None,
        short_entry_threshold: None,
        short_exit_threshold: None,
        enable_sentiment_overlay: Some(true),
        sentiment_reduce_threshold: Some(-0.55),
        sentiment_exit_threshold: Some(-0.85),
        sentiment_min_articles: Some(20),
    };
    let Json(resp) = strategy_config::handle_put(
        state.clone(),
        axum::extract::Query(strategy_config::ModelIdQuery {
            model_id: Some("test-sentiment".to_string()),
        }),
        axum::Json(update),
    )
    .await
    .unwrap();

    assert_eq!(resp.model_id, Some("test-sentiment".to_string()));
    assert!(resp.enable_sentiment_overlay);
    assert_eq!(resp.sentiment_reduce_threshold, -0.55);
    assert_eq!(resp.sentiment_exit_threshold, -0.85);
    assert_eq!(resp.sentiment_min_articles, 20);

    let sp = custom_params.read().await;
    assert!(sp.enable_sentiment_overlay);
    assert_eq!(sp.sentiment_reduce_threshold, -0.55);
    assert_eq!(sp.sentiment_exit_threshold, -0.85);
    assert_eq!(sp.sentiment_min_articles, 20);
}
