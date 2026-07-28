use axum::{extract::State, http::StatusCode, response::Json};
use std::collections::HashMap;

use crate::db;

use super::{ApiResult, AppState};

#[derive(serde::Serialize)]
pub(crate) struct EquityDataPoint {
    ts: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
}

#[derive(serde::Serialize)]
pub(crate) struct EquityDataResponse {
    symbol: String,
    count: usize,
    source: String,
    data: Vec<EquityDataPoint>,
}

#[derive(serde::Serialize)]
pub(crate) struct BackfillResponse {
    symbol: String,
    rows_loaded: usize,
    already_had: i64,
    source: String,
}

#[derive(serde::Serialize)]
pub(crate) struct MacroResponse {
    rows: Vec<EquityDataPoint>,
    symbol: String,
}

#[derive(serde::Serialize)]
pub(crate) struct EquityFeatureResponse {
    symbol: String,
    count: usize,
    latest: crate::features::equities_v2::EquityFeatureRow,
    rows: Vec<crate::features::equities_v2::EquityFeatureRow>,
}

/// `GET /api/equity/data?symbol=QQQ&limit=500`
pub(crate) async fn handle_equity_data(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
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
    let source = candles
        .first()
        .map(|c| c.source.clone())
        .unwrap_or_else(|| "yahoo".to_string());
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

/// `GET /api/equity/backfill?symbol=QQQ&range=5y`
pub(crate) async fn handle_equity_backfill(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<BackfillResponse> {
    let symbol = params
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| "QQQ".to_string());
    let range = params
        .get("range")
        .cloned()
        .unwrap_or_else(|| "5y".to_string());

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

/// `GET /api/equity/macro?symbol=$VIX&limit=1000`
pub(crate) async fn handle_equity_macro(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
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

/// `GET /api/equity/features?symbol=QQQ&limit=500`
pub(crate) async fn handle_equity_features(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
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

    let candles = db::fetch_equity_candles_asc(&state.pool, &symbol, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")))?;
    if candles.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("no equity candles found for symbol '{symbol}' — run /api/equity/backfill first"),
        ));
    }

    let vix_candles = db::fetch_equity_candles_asc(&state.pool, "^VIX", limit)
        .await
        .unwrap_or_default();
    let tlt_candles = db::fetch_equity_candles_asc(&state.pool, "TLT", limit)
        .await
        .unwrap_or_default();

    let vix_close = align_series(&candles, &vix_candles);
    let tlt_close = align_series(&candles, &tlt_candles);

    let rows = crate::features::equities_v2::compute_equity_features(
        &candles,
        vix_close.as_deref(),
        tlt_close.as_deref(),
    );
    let count = rows.len();
    let latest = rows.last().cloned().ok_or((
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
    let mut sec_map: std::collections::HashMap<i64, f64> =
        std::collections::HashMap::with_capacity(secondary.len());
    for c in secondary {
        sec_map.insert(c.ts, c.close);
    }
    let mut aligned = Vec::with_capacity(primary.len());
    let mut last_val: Option<f64> = None;
    let sec_sorted: Vec<(i64, f64)> = {
        let mut v: Vec<(i64, f64)> = sec_map.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_by_key(|x| x.0);
        v
    };
    for p in primary {
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
