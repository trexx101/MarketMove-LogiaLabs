use std::collections::HashMap;

use axum::{extract::State, response::Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};

use crate::{db, strategy};

use super::{internal_error, ts_to_rfc3339, ApiResult, AppState};

#[derive(Serialize)]
pub(crate) struct ChartResponse {
    pub candles: Vec<CandleDto>,
    pub sma: Vec<SmaPoint>,
    pub stale: bool,
    pub live_quote: Option<LiveQuote>,
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

#[derive(Deserialize)]
pub(crate) struct ChartQuery {
    pub range: Option<String>,
    pub limit: Option<i64>,
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

/// `GET /api/chart?range=5y&limit=500`
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
    };
    let limit = query.limit() as i64;
    let range = query.range();

    // Always try to refresh from Yahoo (will skip if data is fresh enough).
    let backfill_started = std::time::Instant::now();
    let backfill_err = match crate::data::yahoo::backfill(
        &state.pool,
        &state.symbol,
        200,
        range,
        43_200, // 12-hour stale threshold
    )
    .await
    {
        Ok(n) => {
            if n > 0 {
                tracing::info!(
                    symbol = %state.symbol,
                    fetched = n,
                    range,
                    elapsed_ms = backfill_started.elapsed().as_millis(),
                    "chart backfill complete"
                );
            }
            None
        }
        Err(e) => {
            tracing::warn!(symbol = %state.symbol, error = %e, "chart backfill failed");
            Some(e)
        }
    };

    // Yield briefly so the DB write commits.
    sleep(Duration::from_millis(50)).await;

    let candles =
        db::fetch_recent_equity_candles(&state.pool, &state.symbol, limit)
            .await
            .map_err(|e| internal_error("fetch_recent_equity_candles", e))?;

    // Detect staleness: no candles OR latest candle > 48h old.
    let now_ts = chrono::Utc::now().timestamp();
    let latest_ts = candles.last().map(|c| c.ts).unwrap_or(0);
    let stale = candles.is_empty()
        || now_ts.saturating_sub(latest_ts) > 172_800; // 48 h

    // Fetch live quote — Moomoo first, Yahoo fallback.
    // It anchors the live-price dashed line and the prediction projections.
    let live_quote = fetch_live_quote(&state.symbol).await;

    if candles.is_empty() && live_quote.is_none() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "no candle data for {} and live quote unavailable — \
                 ensure /api/equity/backfill has been run or check network access",
                state.symbol
            ),
        ));
    }

    let sma_window = state.strategy_params.read().await.sma_window;
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let mut sma_points = Vec::new();

    for i in 0..candles.len() {
        let (mean, valid) = strategy::compute_sma(&closes[..=i], sma_window);
        if valid {
            sma_points.push(SmaPoint {
                ts: ts_to_rfc3339(candles[i].ts),
                value: mean,
            });
        }
    }

    // fetch_recent_equity_candles returns newest-first; reverse for chart (ascending ts).
    let candle_dtos: Vec<CandleDto> = candles
        .iter()
        .rev()
        .map(equity_candle_to_dto)
        .collect();

    Ok(Json(ChartResponse {
        candles: candle_dtos,
        sma: sma_points,
        stale,
        live_quote,
    }))
}

fn equity_candle_to_dto(c: &db::EquityCandle) -> CandleDto {
    CandleDto {
        ts: ts_to_rfc3339(c.ts),
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
