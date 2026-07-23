use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::error;

use crate::{config::Config, db, strategy};

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Clone)]
struct AppState {
    pool: db::DbPool,
    trading_mode: crate::config::TradingMode,
    symbol: String,
    sma_window: usize,
}

pub fn router(pool: db::DbPool, config: &Config) -> Router {
    let state = AppState {
        pool,
        trading_mode: config.trading_mode,
        symbol: config.symbol.clone(),
        sma_window: config.sma_window,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/predictions", get(handle_predictions))
        .route("/api/accuracy", get(handle_accuracy))
        .route("/api/chart", get(handle_chart))
        .route("/api/equity/data", get(handle_equity_data))
        .route("/api/equity/backfill", get(handle_equity_backfill))
        .route("/api/equity/macro", get(handle_equity_macro))
        .route("/api/equity/features", get(handle_equity_features))
        .layer(cors)
        .with_state(state)
        .fallback_service(
            ServeDir::new("frontend")
                .not_found_service(ServeFile::new("frontend/index.html")),
        )
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StatusResponse {
    mode: String,
    symbol: String,
    position: String,
    entry_price: Option<f64>,
    realized_pnl: f64,
    unrealized_pnl: Option<f64>,
    last_candle_ts: Option<String>,
    last_close: Option<f64>,
    pred_1h: Option<f64>,
    pred_4h: Option<f64>,
    pred_24h: Option<f64>,
    staleness_secs: u64,
}

#[derive(Debug, Serialize)]
struct AccuracyResponse {
    directional_1h: f64,
    directional_4h: f64,
    directional_24h: f64,
    mae_1h: f64,
    mae_4h: f64,
    mae_24h: f64,
    resolved_count: usize,
}

#[derive(Serialize)]
struct PredictionsResponse {
    latest: Option<PredictionDto>,
    history: Vec<PredictionDto>,
}

#[derive(Serialize)]
struct PredictionDto {
    candle_ts: String,
    pred_1h: f64,
    pred_4h: f64,
    pred_24h: f64,
    created_at: String,
    actual_1h: Option<f64>,
    actual_4h: Option<f64>,
    actual_24h: Option<f64>,
}

#[derive(Serialize)]
struct ChartResponse {
    candles: Vec<CandleDto>,
    sma: Vec<SmaPoint>,
}

#[derive(Serialize)]
struct CandleDto {
    ts: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    vwap: f64,
}

#[derive(Serialize)]
struct SmaPoint {
    ts: String,
    value: f64,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_status(State(state): State<AppState>) -> ApiResult<StatusResponse> {
    let pool = &state.pool;

    let position_raw = db::load_position(pool)
        .await
        .map_err(|e| internal_error("load_position", e))?;
    let position = strategy::Position::from_i64(position_raw);

    let realized_pnl = db::sum_realized_pnl(pool)
        .await
        .map_err(|e| internal_error("sum_realized_pnl", e))?;

    let candle = db::fetch_latest_candle(pool)
        .await
        .map_err(|e| internal_error("fetch_latest_candle", e))?;

    let (entry_price, unrealized_pnl) = match position {
        strategy::Position::Flat => (None, None),
        strategy::Position::Long | strategy::Position::Short => {
            let entry = db::fetch_entry_trade_price(pool)
                .await
                .map_err(|e| internal_error("fetch_entry_trade_price", e))?;
            let last_close = candle.as_ref().map(|c| c.close);
            let unrealized = match (entry, last_close) {
                (Some(ep), Some(lc)) => match position {
                    strategy::Position::Long => Some((lc - ep) * 1.0),
                    strategy::Position::Short => Some((ep - lc) * 1.0),
                    strategy::Position::Flat => None,
                },
                _ => None,
            };
            (entry, unrealized)
        }
    };

    let predictions = db::fetch_recent_predictions(pool, 1)
        .await
        .map_err(|e| internal_error("fetch_recent_predictions", e))?;
    let latest_pred = predictions.first();

    // Compute staleness of the latest candle, if any.
    let staleness_secs = match db::latest_ts(pool)
        .await
        .map_err(|e| internal_error("latest_ts", e))?
    {
        Some(ts) => {
            let now = chrono::Utc::now().timestamp();
            now.saturating_sub(ts).max(0) as u64
        }
        None => u64::MAX,
    };

    Ok(Json(StatusResponse {
        mode: state.trading_mode.to_string(),
        symbol: state.symbol,
        position: position.to_string(),
        entry_price,
        realized_pnl,
        unrealized_pnl,
        last_candle_ts: candle.as_ref().map(|c| ts_to_rfc3339(c.ts)),
        last_close: candle.as_ref().map(|c| c.close),
        pred_1h: latest_pred.map(|p| p.pred_1h),
        pred_4h: latest_pred.map(|p| p.pred_4h),
        pred_24h: latest_pred.map(|p| p.pred_24h),
        staleness_secs,
    }))
}

async fn handle_predictions(State(state): State<AppState>) -> ApiResult<PredictionsResponse> {
    let history = db::fetch_recent_predictions(&state.pool, 48)
        .await
        .map_err(|e| internal_error("fetch_recent_predictions", e))?;

    let latest = history.first().map(prediction_to_dto);
    let history_dtos: Vec<PredictionDto> = history.iter().map(prediction_to_dto).collect();

    Ok(Json(PredictionsResponse {
        latest,
        history: history_dtos,
    }))
}

async fn handle_chart(State(state): State<AppState>) -> ApiResult<ChartResponse> {
    let limit = (state.sma_window * 2).min(500);
    let candles = db::fetch_recent_candles(&state.pool, limit)
        .await
        .map_err(|e| internal_error("fetch_recent_candles", e))?;

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let mut sma_points = Vec::new();

    for i in 0..candles.len() {
        let (mean, valid) = strategy::compute_sma(&closes[..=i], state.sma_window);
        if valid {
            sma_points.push(SmaPoint {
                ts: ts_to_rfc3339(candles[i].ts),
                value: mean,
            });
        }
    }

    let candle_dtos: Vec<CandleDto> = candles.iter().map(candle_to_dto).collect();

    Ok(Json(ChartResponse {
        candles: candle_dtos,
        sma: sma_points,
    }))
}

async fn handle_accuracy(State(state): State<AppState>) -> ApiResult<AccuracyResponse> {
    let stats = db::fetch_accuracy(&state.pool)
        .await
        .map_err(|e| internal_error("fetch_accuracy", e))?;

    if stats.resolved_count == 0 {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "no resolved predictions yet".to_string(),
        ));
    }

    Ok(Json(AccuracyResponse {
        directional_1h: stats.directional_1h,
        directional_4h: stats.directional_4h,
        directional_24h: stats.directional_24h,
        mae_1h: stats.mae_1h,
        mae_4h: stats.mae_4h,
        mae_24h: stats.mae_24h,
        resolved_count: stats.resolved_count,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ts_to_rfc3339(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn prediction_to_dto(row: &db::PredictionRow) -> PredictionDto {
    PredictionDto {
        candle_ts: ts_to_rfc3339(row.candle_ts),
        pred_1h: row.pred_1h,
        pred_4h: row.pred_4h,
        pred_24h: row.pred_24h,
        created_at: ts_to_rfc3339(row.created_at),
        actual_1h: row.actual_1h,
        actual_4h: row.actual_4h,
        actual_24h: row.actual_24h,
    }
}

fn candle_to_dto(c: &db::Candle) -> CandleDto {
    CandleDto {
        ts: ts_to_rfc3339(c.ts),
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume,
        vwap: c.vwap,
    }
}

fn internal_error(context: &str, err: anyhow::Error) -> (StatusCode, String) {
    error!(error = %err, context, "API handler error");
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{context}: {err:#}"))
}

#[cfg(test)]
mod tests {
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
        State(AppState {
            pool,
            trading_mode: crate::config::TradingMode::Paper,
            symbol: "BTC/USD".to_string(),
            sma_window: TEST_SMA_WINDOW,
        })
    }

    #[tokio::test]
    async fn status_returns_empty_state() {
        let pool = test_pool().await;
        let Json(status) = handle_status(test_state(pool)).await.unwrap();

        assert_eq!(status.mode, "paper");
        assert_eq!(status.symbol, "BTC/USD");
        assert_eq!(status.position, "flat");
        assert_eq!(status.realized_pnl, 0.0);
        assert!(status.entry_price.is_none());
        assert!(status.unrealized_pnl.is_none());
        assert!(status.last_candle_ts.is_none());
        assert!(status.pred_1h.is_none());
        // No candles in the empty-state DB, so staleness should be u64::MAX.
        assert_eq!(status.staleness_secs, u64::MAX);
    }

    #[tokio::test]
    async fn accuracy_returns_503_when_no_resolved() {
        let pool = test_pool().await;
        let result = handle_accuracy(test_state(pool)).await;

        let err = result.unwrap_err();
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.1.contains("no resolved predictions"), "got: {}", err.1);
    }

    #[tokio::test]
    async fn predictions_returns_history() {
        let pool = test_pool().await;
        db::insert_prediction(&pool, 1_000_000, 0.1, 0.2, 0.3, "[]")
            .await
            .unwrap();

        let Json(resp) = handle_predictions(test_state(pool)).await.unwrap();
        assert!(resp.latest.is_some());
        assert_eq!(resp.history.len(), 1);
        let latest = resp.latest.unwrap();
        assert!((latest.pred_1h - 0.1).abs() < 1e-9);
        assert!((latest.pred_4h - 0.2).abs() < 1e-9);
        assert!((latest.pred_24h - 0.3).abs() < 1e-9);
    }

    #[tokio::test]
    async fn chart_computes_rolling_sma() {
        let pool = test_pool().await;
        let closes = [100.0, 102.0, 101.0, 103.0, 104.0];
        for (i, close) in closes.iter().enumerate() {
            let ts = 1_000_000 + i as i64 * 3_600;
            db::upsert_candle(
                &pool,
                &db::Candle {
                    ts,
                    open: *close,
                    high: *close,
                    low: *close,
                    close: *close,
                    volume: 1.0,
                    vwap: *close,
                    funding_rate: 0.0,
                    basis_z: 0.0,
                    ob_imbalance: 0.0,
                },
            )
            .await
            .unwrap();
        }

        let Json(resp) = handle_chart(test_state(pool)).await.unwrap();
        let expected_sma_points = closes.len().saturating_sub(TEST_SMA_WINDOW - 1);
        assert_eq!(resp.candles.len(), closes.len());
        assert_eq!(resp.sma.len(), expected_sma_points);
        assert_eq!(resp.sma.first().unwrap().ts, resp.candles[TEST_SMA_WINDOW - 1].ts);
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
        db::upsert_candle(
            &pool,
            &db::Candle {
                ts: 1_000_000,
                open: 110.0,
                high: 110.0,
                low: 110.0,
                close: 110.0,
                volume: 1.0,
                vwap: 110.0,
                funding_rate: 0.0,
                basis_z: 0.0,
                ob_imbalance: 0.0,
            },
        )
        .await
        .unwrap();

        let Json(status) = handle_status(test_state(pool)).await.unwrap();
        assert_eq!(status.position, "long");
        assert!((status.entry_price.unwrap() - 100.0).abs() < 1e-9);
        assert!((status.unrealized_pnl.unwrap() - 10.0).abs() < 1e-9);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn router_serves_static_files_and_api() {
        use tokio::net::TcpListener;

        // router() resolves "frontend" relative to CWD; cargo test sets CWD to the package root.
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

            let app = router(pool, &config);
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
            assert!(body.contains("MarketMarkovNet"));

            let spa_res = client
                .get(format!("{base}/some/unknown/route"))
                .send()
                .await
                .unwrap();
            let spa_status = spa_res.status();
            let spa_body = spa_res.text().await.unwrap();
            assert!(
                spa_body.contains("MarketMarkovNet"),
                "SPA fallback should serve index.html, got status {} body: {}",
                spa_status,
                spa_body
            );
        }
        .await;

        std::env::set_current_dir(original_cwd).unwrap();
        result
    }
}

// ---------------------------------------------------------------------------
// Wave A: equities data endpoints
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct EquityDataPoint {
    ts: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
}

#[derive(serde::Serialize)]
struct EquityDataResponse {
    symbol: String,
    count: usize,
    source: String,
    data: Vec<EquityDataPoint>,
}

/// `GET /api/equity/data?symbol=QQQ&range=500&limit=5000`
/// Returns most recent equity candles (newest first), capped at `limit`.
async fn handle_equity_data(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<EquityDataResponse> {
    let symbol = params
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| "QQQ".to_string());
    let limit: i64 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500)
        .clamp(1, 20_000);

    let candles = match db::fetch_equity_candles(&state.pool, &symbol, limit).await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}"))),
    };
    let source = candles.first().map(|c| c.source.clone()).unwrap_or_else(|| "yahoo".to_string());
    let data: Vec<EquityDataPoint> = candles
        .iter()
        .map(|c| EquityDataPoint {
            ts: c.ts,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
        })
        .collect();
    Ok(Json(EquityDataResponse {
        count: data.len(),
        symbol,
        source,
        data,
    }))
}

#[derive(serde::Serialize)]
struct BackfillResponse {
    symbol: String,
    rows_loaded: usize,
    already_had: i64,
    source: String,
}

/// `GET /api/equity/backfill?symbol=QQQ&range=5y`
/// Triggers a Yahoo backfill for one symbol. Used for Wave A bring-up.
async fn handle_equity_backfill(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<BackfillResponse> {
    let symbol = params
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| "QQQ".to_string());
    let range = params.get("range").cloned().unwrap_or_else(|| "5y".to_string());

    let already = db::count_equity_candles(&state.pool, &symbol)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")))?;

    let rows = crate::data::yahoo::backfill(&state.pool, &symbol, 0, &range)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("yahoo: {e:#}")))?;

    Ok(Json(BackfillResponse {
        symbol,
        rows_loaded: rows,
        already_had: already,
        source: "yahoo".to_string(),
    }))
}

#[derive(serde::Serialize)]
struct MacroResponse {
    rows: Vec<EquityDataPoint>,
    symbol: String,
}

/// `GET /api/equity/macro?symbol=$VIX&range=730`
/// Returns macro series from `equity_candles` (populated by FRED or Yahoo).
async fn handle_equity_macro(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<MacroResponse> {
    let symbol = params
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| "$VIX".to_string());
    let limit: i64 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
        .clamp(1, 20_000);

    let candles = db::fetch_equity_candles(&state.pool, &symbol, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")))?;
    let rows: Vec<EquityDataPoint> = candles
        .iter()
        .map(|c| EquityDataPoint {
            ts: c.ts,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
        })
        .collect();
    Ok(Json(MacroResponse { symbol, rows }))
}

// ---------------------------------------------------------------------------
// Wave B: equities features endpoint
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct EquityFeatureResponse {
    symbol: String,
    count: usize,
    /// The most recent feature vector (8-dim).
    latest: crate::features::equities_v2::EquityFeatureRow,
    /// All computed feature rows (oldest first, capped at `limit`).
    rows: Vec<crate::features::equities_v2::EquityFeatureRow>,
}

/// `GET /api/equity/features?symbol=QQQ&limit=500`
///
/// Computes the 8-feature equities vector from stored daily OHLCV + macro
/// series. Pulls QQQ (or the requested symbol), VIX, and TLT candles from
/// `equity_candles`, aligns them by timestamp, and runs the feature pipeline.
///
/// If VIX or TLT data is missing (e.g. FRED timed out), the corresponding
/// features are 0.0 — the pipeline degrades gracefully.
async fn handle_equity_features(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<EquityFeatureResponse> {
    let symbol = params
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| "QQQ".to_string());
    let limit: i64 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500)
        .clamp(1, 10_000);

    // Fetch main symbol candles (oldest first for feature computation).
    let candles = db::fetch_equity_candles_asc(&state.pool, &symbol, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")))?;
    if candles.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("no equity candles found for symbol '{symbol}' — run /api/equity/backfill first"),
        ));
    }

    // Fetch VIX and TLT close-aligned series (best effort).
    let vix_candles = db::fetch_equity_candles_asc(&state.pool, "$VIX", limit)
        .await
        .unwrap_or_default();
    let tlt_candles = db::fetch_equity_candles_asc(&state.pool, "TLT", limit)
        .await
        .unwrap_or_default();

    // Align VIX and TLT closes to the main symbol's timestamps.
    let vix_close = align_series(&candles, &vix_candles);
    let tlt_close = align_series(&candles, &tlt_candles);

    let rows = crate::features::equities_v2::compute_equity_features(
        &candles,
        vix_close.as_deref(),
        tlt_close.as_deref(),
    );
    let count = rows.len();
    let latest = rows
        .last()
        .cloned()
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "feature computation returned 0 rows".to_string(),
        ))?;

    Ok(Json(EquityFeatureResponse {
        symbol,
        count,
        latest,
        rows,
    }))
}

/// Align a secondary series (e.g. VIX, TLT) to the main symbol's timestamps.
/// Returns `None` if the secondary series is empty.
/// Uses nearest-prior timestamp matching (forward-fill).
fn align_series(
    primary: &[db::EquityCandle],
    secondary: &[db::EquityCandle],
) -> Option<Vec<f64>> {
    if secondary.is_empty() {
        return None;
    }
    // Build a map of ts → close for the secondary series.
    let mut sec_map: std::collections::HashMap<i64, f64> =
        std::collections::HashMap::with_capacity(secondary.len());
    for c in secondary {
        sec_map.insert(c.ts, c.close);
    }
    // Walk the primary timestamps; for each, find the matching close or the
    // most recent prior close (forward-fill).
    let mut aligned = Vec::with_capacity(primary.len());
    let mut last_val: Option<f64> = None;
    let sec_sorted: Vec<(i64, f64)> = {
        let mut v: Vec<(i64, f64)> = sec_map.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_by_key(|x| x.0);
        v
    };
    for p in primary {
        // Binary search for the nearest-prior timestamp.
        let idx = sec_sorted
            .binary_search_by_key(&p.ts, |x| x.0)
            .unwrap_or_else(|i| i.saturating_sub(1));
        if idx < sec_sorted.len() && sec_sorted[idx].0 <= p.ts {
            last_val = Some(sec_sorted[idx].1);
        }
        aligned.push(last_val.unwrap_or(0.0));
    }
    Some(aligned)
}
