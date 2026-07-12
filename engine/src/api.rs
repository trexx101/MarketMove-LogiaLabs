use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
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
        .route("/api/chart", get(handle_chart))
        .layer(cors)
        .with_state(state)
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
            },
        )
        .await
        .unwrap();

        let Json(status) = handle_status(test_state(pool)).await.unwrap();
        assert_eq!(status.position, "long");
        assert!((status.entry_price.unwrap() - 100.0).abs() < 1e-9);
        assert!((status.unrealized_pnl.unwrap() - 10.0).abs() < 1e-9);
    }
}
