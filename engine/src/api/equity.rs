use axum::{extract::State, http::StatusCode, response::Json};
use std::collections::HashMap;
use tracing::info;

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

    let rows = crate::data::yahoo::backfill(&state.pool, &symbol, 0, &range, 0)
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

// ---------------------------------------------------------------------------
// POST /api/equity/backfill_predictions — replay inference over historical bars
// ---------------------------------------------------------------------------

use crate::features::equities_v2::{compute_equity_features, EquityNormStats};

/// Optional request body for the backfill_predictions endpoint.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct BackfillPredictionsQuery {
    /// Override feature window size (default from FEATURE_WINDOW_SIZE env).
    #[serde(default = "default_feature_window_size")]
    pub feature_window_size: usize,
    /// Override ATR lookback period (default 14).
    #[serde(default = "default_atr_period")]
    pub atr_period: usize,
    /// Symbol to backfill (default: engine bootstrap symbol). Must match a
    /// `trading_models` primary_symbol (or the bootstrap symbol) so the
    /// correct per-model norm stats can be loaded.
    #[serde(default)]
    pub symbol: Option<String>,
}

fn default_feature_window_size() -> usize {
    126
}
fn default_atr_period() -> usize {
    14
}

/// Response shape for the backfill_predictions endpoint.
#[derive(serde::Serialize)]
pub(crate) struct BackfillPredictionsResponse {
    pub symbol: String,
    pub candles_processed: usize,
    pub predictions_written: usize,
    pub skipped_already_had: usize,
    pub errors: usize,
}

/// `POST /api/equity/backfill_predictions`
///
/// Replays the scheduler's inference pipeline over every candle in the DB that
/// does NOT already have a corresponding `equity_predictions` row.
///
/// Uses the ZMQ bridge to call the Python inference service, then upserts each
/// prediction into `equity_predictions`.  Feature normalization uses the same
/// `EquityNormStats` loaded at startup (norm_stats_qqq_v1.json).
///
/// This populates the prediction history needed by:
///   - `POST /api/backtest`  (strategy replay)
///   - `GET /api/accuracy`   (IC / directional accuracy)
pub(crate) async fn handle_backfill_predictions(
    State(state): State<super::AppState>,
    axum::extract::Json(params): axum::extract::Json<BackfillPredictionsQuery>,
) -> ApiResult<BackfillPredictionsResponse> {
    let feature_window_size = params.feature_window_size;
    let atr_period = params.atr_period;
    let symbol = params
        .symbol
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.symbol.clone());

    // Resolve the norm stats for THIS symbol. The scheduler bootstrap loads
    // per-model files from the `trading_models` registry; do the same here so
    // a backfill for SMH/XLF uses their own stats, not the QQQ default.
    let norm_stats_path = match db::load_enabled_models(&state.pool).await {
        Ok(models) => models
            .iter()
            .find(|m| m.primary_symbol.eq_ignore_ascii_case(&symbol))
            .map(|m| m.norm_stats_path.clone())
            .unwrap_or_else(|| state.norm_stats_path.clone()),
        Err(_) => state.norm_stats_path.clone(),
    };

    let norm_stats = EquityNormStats::load_named(&norm_stats_path)
        .map_err(|e| {
            tracing::error!(error = %e, path = %norm_stats_path, "failed to load norm_stats");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("norm_stats load: {e}"))
        })?;

    // Fetch VIX and TLT macro series once (used for every bar).
    let fetch_count = (feature_window_size + atr_period + 50) as i64;
    let vix_candles = db::fetch_equity_candles_asc(&state.pool, "^VIX", fetch_count)
        .await
        .unwrap_or_default();
    let tlt_candles = db::fetch_equity_candles_asc(&state.pool, "TLT", fetch_count)
        .await
        .unwrap_or_default();

    // Pull the full candle history for this symbol (oldest-first for feature computation).
    let candles = db::fetch_equity_candles_asc(&state.pool, &symbol, 50_000)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")))?;

    if candles.len() < feature_window_size + atr_period + 10 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "insufficient candles: {} available, need {}",
                candles.len(),
                feature_window_size + atr_period + 10
            ),
        ));
    }

    // Build lookups for macro series.
    let vix_map: HashMap<i64, f64> = vix_candles
        .iter()
        .map(|c| (c.ts, c.close))
        .collect();
    let tlt_map: HashMap<i64, f64> = tlt_candles
        .iter()
        .map(|c| (c.ts, c.close))
        .collect();

    // Pre-load existing prediction timestamps so we skip bars that already have one.
    let existing: HashMap<i64, ()> = db::fetch_recent_equity_predictions(&state.pool, &symbol, 100_000)
        .await
        .map(|rows| rows.into_iter().map(|p| (p.candle_ts, ())).collect())
        .unwrap_or_default();

    // Align macro series to QQQ timestamps.
    let vix_aligned = align_for_features(&candles, &vix_map);
    let tlt_aligned = align_for_features(&candles, &tlt_map);

    // Compute all features in one pass.
    let all_features =
        compute_equity_features(&candles, Some(&vix_aligned), Some(&tlt_aligned));
    let norm_stats_clone = norm_stats.clone();

    if all_features.len() < feature_window_size {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "feature computation returned {} rows, need {}",
                all_features.len(),
                feature_window_size
            ),
        ));
    }

    // Connect ZMQ bridge to the inference service.
    let mut bridge = crate::bridge::ZmqBridge::connect(&state.zmq_endpoint)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("ZMQ connect: {e}")))?;

    let mut written = 0;
    let mut skipped = 0;
    let mut errors = 0;

    // Iterate every candle that has enough warmup.
    let start_idx = feature_window_size.max(atr_period);
    for i in start_idx..candles.len() {
        let candle = &candles[i];
        let candle_ts = candle.ts;

        // Skip if already predicted.
        if existing.contains_key(&candle_ts) {
            skipped += 1;
            continue;
        }

        // Build feature window: last `feature_window_size` rows, normalized.
        let fw_start = i.saturating_sub(feature_window_size);
        let feature_window: Vec<[f64; 8]> = all_features[fw_start..=i]
            .iter()
            .map(|row| norm_stats_clone.normalize(row))
            .collect();

        // Compute ATR ratio for this candle.
        let atr_ratio = compute_atr_for_bar(&candles[..=i], atr_period);
        let atr_ratio = atr_ratio.unwrap_or(0.005);

        // Call inference service.
        match bridge
            .predict_v3_with_retry(
                &symbol,
                &feature_window,
                atr_ratio,
                std::time::Duration::from_secs(10),
                2,
            )
            .await
        {
            Ok(pred) => {
                // Compute regime for this bar.
                let closes: Vec<f64> = candles[..=i].iter().map(|c| c.close).collect();
                let (sma, sma_valid) =
                    crate::strategy::compute_sma(&closes, state.strategy_params.read().await.sma_window);
                let regime = if !sma_valid {
                    "unknown"
                } else if candle.close > sma {
                    "bull"
                } else {
                    "bear"
                };

                let features_json =
                    serde_json::to_string(&feature_window).unwrap_or_else(|_| "{}".into());

                if let Err(e) = db::insert_equity_prediction(
                    &state.pool,
                    &symbol,
                    candle_ts,
                    pred.pred_1d,
                    pred.pred_5d,
                    pred.pred_21d,
                    regime,
                    &features_json,
                )
                .await
                {
                    tracing::warn!(candle_ts, error = %e, "failed to persist prediction");
                    errors += 1;
                } else {
                    written += 1;
                    if written % 50 == 0 {
                        info!(processed = i, written, skipped, errors, "backfill_predictions progress");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(candle_ts, error = %e, "inference failed for bar");
                errors += 1;
            }
        }
    }

    info!(
        symbol = %symbol,
        candles_processed = candles.len(),
        predictions_written = written,
        skipped_already_had = skipped,
        errors = errors,
        "backfill_predictions complete"
    );

    Ok(Json(BackfillPredictionsResponse {
        symbol,
        candles_processed: candles.len(),
        predictions_written: written,
        skipped_already_had: skipped,
        errors,
    }))
}

/// Align a macro close series (HashMap ts→close) to the main candle list.
/// Returns a Vec<f64> indexed by main-candle position, 0.0 for gaps.
fn align_for_features(candles: &[crate::db::EquityCandle], series: &HashMap<i64, f64>) -> Vec<f64> {
    let mut result = Vec::with_capacity(candles.len());
    for c in candles {
        result.push(series.get(&c.ts).copied().unwrap_or(0.0));
    }
    result
}

/// Compute ATR(period) / close for the bar at index `i`.
fn compute_atr_for_bar(candles: &[crate::db::EquityCandle], period: usize) -> Option<f64> {
    let n = candles.len();
    if n <= period {
        return None;
    }
    let mut tr = Vec::with_capacity(n);
    tr.push(0.0_f64);
    for i in 1..n {
        let h = candles[i].high;
        let l = candles[i].low;
        let pc = candles[i - 1].close;
        let h_l = h - l;
        let h_c = (h - pc).abs();
        let l_c = (l - pc).abs();
        tr.push(h_l.max(h_c).max(l_c));
    }
    let warmup: f64 = tr[1..=period].iter().sum::<f64>() / period as f64;
    let mut atr = warmup;
    for i in (period + 1)..n {
        atr = atr * ((period - 1) as f64 / period as f64) + tr[i] / period as f64;
    }
    let close = candles.last().map(|c| c.close).unwrap_or(1.0);
    if atr <= 0.0 || close <= 0.0 {
        Some(0.005)
    } else {
        Some(atr / close)
    }
}

// ---------------------------------------------------------------------------
// Helper: align a secondary series to main candle timestamps
// ---------------------------------------------------------------------------

/// Align a secondary series (e.g. VIX, TLT) to the main symbol's timestamps.
#[derive(serde::Serialize)]
pub(crate) struct EquityTradePoint {
    id: i64,
    ts: String,
    symbol: String,
    side: String,
    qty: f64,
    price: f64,
    fee: f64,
    realized_pnl: f64,
    cumulative_pnl: f64,
}

#[derive(serde::Serialize)]
pub(crate) struct EquityTradesResponse {
    symbol: String,
    count: usize,
    total_realized_pnl: f64,
    trades: Vec<EquityTradePoint>,
}

/// `GET /api/equity/trades?symbol=QQQ&limit=500`
///
/// Returns the trading ledger for a symbol: every fill in chronological order
/// with a running cumulative PnL column, plus the grand total.
pub(crate) async fn handle_equity_trades(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<EquityTradesResponse> {
    let symbol = params
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| "QQQ".to_string());
    let limit: i64 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500)
        .clamp(1, 10_000);

    // fetch_recent_equity_trades returns newest-first; reverse for chronological.
    let all_symbols = symbol == "*" || symbol == "ALL";
    let mut rows = if all_symbols {
        db::fetch_recent_all_equity_trades(&state.pool, limit as usize).await
    } else {
        db::fetch_recent_equity_trades(&state.pool, &symbol, limit as usize).await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")))?;
    rows.reverse();

    let mut cumulative = 0.0_f64;
    let trades: Vec<EquityTradePoint> = rows
        .iter()
        .map(|r| {
            cumulative += r.realized_pnl;
            EquityTradePoint {
                id: r.id,
                ts: super::ts_to_rfc3339(r.candle_ts),
                symbol: r.symbol.clone(),
                side: r.side.clone(),
                qty: r.qty,
                price: r.price,
                fee: r.fee,
                realized_pnl: r.realized_pnl,
                cumulative_pnl: cumulative,
            }
        })
        .collect();

    let total_realized_pnl = db::sum_equity_realized_pnl(&state.pool, &symbol)
        .await
        .unwrap_or(0.0);

    let count = trades.len();
    Ok(Json(EquityTradesResponse {
        symbol,
        count,
        total_realized_pnl,
        trades,
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
