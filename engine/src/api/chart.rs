use std::collections::HashMap;

use axum::{extract::State, response::Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};

use crate::{db, strategy};

use super::{internal_error, ts_to_rfc3339, ApiResult, AppState};

// ── Response shapes ────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct ChartResponse {
    pub candles: Vec<CandleDto>,
    pub sma: Vec<SmaPoint>,
    pub stale: bool,
    pub live_quote: Option<LiveQuote>,
    /// Additive §Phase 1: multi-period SMA + Ichimoku.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indicators: Option<IndicatorsDto>,
    /// Additive §Phase 2: historical prediction vs actual markers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predictions: Option<Vec<PredictionMarkerDto>>,
}

#[derive(Serialize)]
pub(crate) struct LiveQuote {
    pub price: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
}

#[derive(Serialize)]
pub(crate) struct CandleDto {
    pub ts: String,
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub vwap: f64,
}

#[derive(Serialize)]
pub(crate) struct SmaPoint {
    pub ts: String,
    pub value: f64,
}

#[derive(Serialize)]
pub(crate) struct IndicatorsDto {
    pub sma: SmaPeriodsDto,
    pub ichimoku: IchimokuDto,
}

#[derive(Serialize)]
pub(crate) struct SmaPeriodsDto {
    /// SMA 20 values, aligned to candle timestamps. Null when window not full.
    #[serde(rename = "20")]
    pub sma_20: Vec<Option<f64>>,
    #[serde(rename = "50")]
    pub sma_50: Vec<Option<f64>>,
    #[serde(rename = "200")]
    pub sma_200: Vec<Option<f64>>,
}

#[derive(Serialize)]
pub(crate) struct IchimokuDto {
    /// Tenkan-sen (conversion line): (9-period high + 9-period low) / 2
    pub tenkan: Vec<Option<f64>>,
    /// Kijun-sen (base line): (26-period high + 26-period low) / 2
    pub kijun: Vec<Option<f64>>,
    /// Senkou Span A (leading span A): (tenkan + kijun) / 2, shifted 26 forward
    pub senkou_a: Vec<Option<f64>>,
    /// Senkou Span B (leading span B): (52-period high + 52-period low) / 2,
    /// shifted 26 forward
    pub senkou_b: Vec<Option<f64>>,
    /// Chikou Span (lagging span): close shifted 26 periods back
    pub chikou: Vec<Option<f64>>,
}

#[derive(Serialize)]
pub(crate) struct PredictionMarkerDto {
    /// Candle timestamp (seconds) where the prediction was MADE.
    pub candle_ts: i64,
    /// RFC 3339 string for display.
    pub ts: String,
    /// Predicted return at 1d horizon (decimal, e.g. 0.02 = +2%).
    pub pred_1d: f64,
    /// Predicted return at 5d horizon.
    pub pred_5d: f64,
    /// Predicted return at 21d horizon.
    pub pred_21d: f64,
    /// Actual log-return at 1d horizon (None if not yet resolved).
    pub actual_1d: Option<f64>,
    /// Actual log-return at 5d horizon.
    pub actual_5d: Option<f64>,
    /// Actual log-return at 21d horizon.
    pub actual_21d: Option<f64>,
    /// Close price at prediction time.
    pub base_close: f64,
}

#[derive(Deserialize)]
pub(crate) struct ChartQuery {
    pub range: Option<String>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub symbol: Option<String>,
}

impl ChartQuery {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(500).min(1500).max(10)
    }

    fn range(&self) -> &str {
        match self.range.as_deref() {
            Some("1y" | "2y" | "5y" | "max") => self.range.as_deref().unwrap(),
            _ => "5y",
        }
    }
}

/// `GET /api/chart?symbol=QQQ&range=5y&limit=500`
///
/// Always attempts a fresh Yahoo Finance backfill so the chart is never stale.
/// Also fetches the current live quote to anchor the price line and projections.
pub(crate) async fn handle_chart(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<ChartResponse> {
    let query = ChartQuery {
        range: params.get("range").cloned(),
        limit: params.get("limit").and_then(|s| s.parse().ok()),
        symbol: params.get("symbol").cloned(),
    };
    let limit = query.limit() as i64;
    let range = query.range();
    let symbol = query.symbol.as_deref().unwrap_or(&state.symbol);

    // Always try to refresh from Yahoo (will skip if data is fresh enough).
    let backfill_started = std::time::Instant::now();
    let backfill_err = match crate::data::yahoo::backfill(
        &state.pool,
        symbol,
        200,
        range,
        43_200, // 12-hour stale threshold
    )
    .await
    {
        Ok(n) => {
            if n > 0 {
                tracing::info!(
                    symbol,
                    fetched = n,
                    range,
                    elapsed_ms = backfill_started.elapsed().as_millis(),
                    "chart backfill complete"
                );
            }
            None
        }
        Err(e) => {
            tracing::warn!(symbol, error = %e, "chart backfill failed");
            Some(e)
        }
    };

    // Yield briefly so the DB write commits.
    sleep(Duration::from_millis(50)).await;

    // Fetch candles ascending (oldest-first) for correct indicator computation.
    let candles =
        db::fetch_equity_candles_asc(&state.pool, symbol, limit)
            .await
            .map_err(|e| internal_error("fetch_equity_candles_asc", e))?;

    // Detect staleness: no candles OR NEWEST candle > 48h old.
    let now_ts = chrono::Utc::now().timestamp();
    let latest_ts = candles.last().map(|c| c.ts).unwrap_or(0);
    let stale = candles.is_empty()
        || now_ts.saturating_sub(latest_ts) > 172_800; // 48 h

    // Fetch live quote — Moomoo first, Yahoo fallback.
    let live_quote = fetch_live_quote(symbol).await;

    if candles.is_empty() && live_quote.is_none() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "no candle data for {symbol} and live quote unavailable — \
                 ensure /api/equity/backfill has been run or check network access"
            ),
        ));
    }

    let n = candles.len();

    // ── Legacy SMA (single window from strategy params) ─────────────────
    let sma_window = state.strategy_params.read().await.sma_window;
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let mut sma_points = Vec::new();
    for i in 0..n {
        let (mean, valid) = strategy::compute_sma(&closes[..=i], sma_window);
        if valid {
            sma_points.push(SmaPoint {
                ts: ts_to_rfc3339(candles[i].ts),
                value: mean,
            });
        }
    }

    // ── Multi-period SMA (20/50/200) ───────────────────────────────────
    let sma_20 = compute_sma_period(&closes, 20);
    let sma_50 = compute_sma_period(&closes, 50);
    let sma_200 = compute_sma_period(&closes, 200);

    // ── Ichimoku ───────────────────────────────────────────────────────
    let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows: Vec<f64> = candles.iter().map(|c| c.low).collect();

    let ichimoku = compute_ichimoku(&highs, &lows, &closes, n);

    // ── Prediction markers ─────────────────────────────────────────────
    let predictions = compute_prediction_markers(&state.pool, symbol, &candles).await;

    let indicators = IndicatorsDto {
        sma: SmaPeriodsDto {
            sma_20,
            sma_50,
            sma_200,
        },
        ichimoku,
    };

    // Build candle DTOs (already ascending).
    let candle_dtos: Vec<CandleDto> = candles.iter().map(equity_candle_to_dto).collect();

    Ok(Json(ChartResponse {
        candles: candle_dtos,
        sma: sma_points,
        stale,
        live_quote,
        indicators: Some(indicators),
        predictions,
    }))
}

// ── Indicator computation ───────────────────────────────────────────────────

/// Compute SMA for a single window, returning None for positions where the
/// window is not yet full. Result is aligned to the input slice.
fn compute_sma_period(closes: &[f64], window: usize) -> Vec<Option<f64>> {
    let mut out = Vec::with_capacity(closes.len());
    for i in 0..closes.len() {
        if i + 1 < window {
            out.push(None);
        } else {
            let slice = &closes[(i + 1 - window)..=i];
            let mean = slice.iter().sum::<f64>() / slice.len() as f64;
            out.push(Some(mean));
        }
    }
    out
}

/// Compute Ichimoku components aligned to the candle arrays.
/// All arrays are the same length as input; positions where the lookback is
/// insufficient are None.
fn compute_ichimoku(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    n: usize,
) -> IchimokuDto {
    let mut tenkan: Vec<Option<f64>> = Vec::with_capacity(n);
    let mut kijun: Vec<Option<f64>> = Vec::with_capacity(n);
    let mut senkou_a: Vec<Option<f64>> = Vec::with_capacity(n);
    let mut senkou_b: Vec<Option<f64>> = Vec::with_capacity(n);
    let mut chikou: Vec<Option<f64>> = Vec::with_capacity(n);

    for i in 0..n {
        // Tenkan-sen: (9-period high + 9-period low) / 2
        if i + 1 >= 9 {
            let h = highs[(i + 1 - 9)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let l = lows[(i + 1 - 9)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
            tenkan.push(Some((h + l) / 2.0));
        } else {
            tenkan.push(None);
        }

        // Kijun-sen: (26-period high + 26-period low) / 2
        if i + 1 >= 26 {
            let h = highs[(i + 1 - 26)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let l = lows[(i + 1 - 26)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
            kijun.push(Some((h + l) / 2.0));
        } else {
            kijun.push(None);
        }

        // Senkou Span A: (tenkan + kijun) / 2, shifted 26 periods forward.
        // At position i, we show the value computed 26 periods ago.
        if i >= 26 {
            if let (Some(Some(t)), Some(Some(k))) =
                (tenkan.get(i - 26), kijun.get(i - 26))
            {
                senkou_a.push(Some((t + k) / 2.0));
            } else {
                senkou_a.push(None);
            }
        } else {
            senkou_a.push(None);
        }

        // Senkou Span B: (52-period high + 52-period low) / 2, shifted 26 forward.
        if i >= 26 && i + 1 >= 52 {
            // Compute at the shifted-back position (i - 26), using 52 periods
            // ending at that point.
            let idx = i - 26;
            if idx + 1 >= 52 {
                let h = highs[(idx + 1 - 52)..=idx].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let l = lows[(idx + 1 - 52)..=idx].iter().cloned().fold(f64::INFINITY, f64::min);
                senkou_b.push(Some((h + l) / 2.0));
            } else {
                senkou_b.push(None);
            }
        } else {
            senkou_b.push(None);
        }

        // Chikou Span: close shifted 26 periods back.
        // At position i, we show the close from i+26 (future).
        if i + 26 < n {
            chikou.push(Some(closes[i + 26]));
        } else {
            chikou.push(None);
        }
    }

    IchimokuDto {
        tenkan,
        kijun,
        senkou_a,
        senkou_b,
        chikou,
    }
}

// ── Prediction markers ──────────────────────────────────────────────────────

/// Compute prediction markers: for each prediction row, look up the actual
/// future close for each horizon (1d, 5d, 21d) and return a marker DTO.
async fn compute_prediction_markers(
    pool: &db::DbPool,
    symbol: &str,
    candles: &[db::EquityCandle],
) -> Option<Vec<PredictionMarkerDto>> {
    use crate::db::EquityPredictionRow;

    let preds: Vec<EquityPredictionRow> = sqlx::query_as::<_, EquityPredictionRow>(
        r#"SELECT id, symbol, candle_ts, pred_1d, pred_5d, pred_21d,
                  regime, features_json, created_at, source
           FROM equity_predictions
           WHERE symbol = ?1
           ORDER BY candle_ts ASC"#,
    )
    .bind(symbol)
    .fetch_all(pool)
    .await
    .ok()?;

    if preds.is_empty() {
        return Some(Vec::new());
    }

    // Build candle close lookup: ts -> close
    let candle_map: HashMap<i64, f64> = candles
        .iter()
        .map(|c| (c.ts, c.close))
        .collect();
    let candle_tss: Vec<i64> = {
        let mut v: Vec<i64> = candle_map.keys().copied().collect();
        v.sort_unstable();
        v
    };

    let day = 86_400_i64;
    let tolerance = 3 * day;

    let mut markers = Vec::with_capacity(preds.len());

    for p in &preds {
        let base_close = match candle_map.get(&p.candle_ts) {
            Some(&c) if c > 0.0 => c,
            _ => continue,
        };

        let actual_1d = find_closest_close(&candle_tss, &candle_map, p.candle_ts + day, tolerance);
        let actual_5d = find_closest_close(&candle_tss, &candle_map, p.candle_ts + 5 * day, tolerance);
        let actual_21d = find_closest_close(&candle_tss, &candle_map, p.candle_ts + 21 * day, tolerance);

        let actual_log = |fc: Option<f64>| -> Option<f64> {
            fc.filter(|&v| v > 0.0).map(|v| (v / base_close).ln())
        };

        markers.push(PredictionMarkerDto {
            candle_ts: p.candle_ts,
            ts: ts_to_rfc3339(p.candle_ts),
            pred_1d: p.pred_1d,
            pred_5d: p.pred_5d,
            pred_21d: p.pred_21d,
            actual_1d: actual_log(actual_1d),
            actual_5d: actual_log(actual_5d),
            actual_21d: actual_log(actual_21d),
            base_close,
        });
    }

    Some(markers)
}

/// Binary-search for the closest close within tolerance.
fn find_closest_close(
    sorted_tss: &[i64],
    candle_map: &HashMap<i64, f64>,
    target_ts: i64,
    tolerance: i64,
) -> Option<f64> {
    let idx = sorted_tss.binary_search(&target_ts).unwrap_or_else(|i| i);
    let mut best: Option<(i64, f64)> = None;
    for &check_idx in &[idx, idx.saturating_sub(1)] {
        if check_idx < sorted_tss.len() {
            let ts = sorted_tss[check_idx];
            let diff = (ts - target_ts).abs();
            if diff <= tolerance {
                if let Some(&close) = candle_map.get(&ts) {
                    if best.is_none() || diff < best.unwrap().0 {
                        best = Some((diff, close));
                    }
                }
            }
        }
    }
    best.map(|(_, c)| c)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn equity_candle_to_dto(c: &db::EquityCandle) -> CandleDto {
    CandleDto {
        ts: ts_to_rfc3339(c.ts),
        time: c.ts,
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume as f64,
        vwap: c.close,
    }
}

/// Fetch a live quote: Moomoo first, Yahoo fallback.
async fn fetch_live_quote(symbol: &str) -> Option<LiveQuote> {
    use crate::data::{moomoo, yahoo};

    let moomoo_ok = moomoo::is_available().await;
    if moomoo_ok {
        match moomoo::fetch_quote(symbol).await {
            Ok(q) => {
                tracing::debug!(symbol, price = q.price, "Moomoo live quote");
                return Some(LiveQuote {
                    price: q.price,
                    prev_close: q.prev_close,
                    change: q.change,
                    change_pct: q.change_pct,
                });
            }
            Err(e) => {
                tracing::warn!(symbol, error = %e, "Moomoo quote failed — trying Yahoo");
            }
        }
    }

    match yahoo::fetch_quote(symbol).await {
        Ok(q) => {
            tracing::debug!(symbol, price = q.price, "Yahoo live quote");
            Some(LiveQuote {
                price: q.price,
                prev_close: q.prev_close,
                change: q.change,
                change_pct: q.change_pct,
            })
        }
        Err(e) => {
            tracing::warn!(symbol, error = %e, "Yahoo quote also failed");
            None
        }
    }
}