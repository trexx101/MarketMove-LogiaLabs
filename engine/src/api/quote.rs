use axum::{extract::State, response::Json};
use serde::Serialize;

use crate::data::{moomoo, yahoo};

use super::{internal_error, ApiResult, AppState};

#[derive(Serialize)]
pub(crate) struct QuoteResponse {
    pub symbol: String,
    pub price: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub timestamp: i64,
}

/// GET /api/quote — Moomoo first, Yahoo fallback.
pub(crate) async fn handle_quote(State(state): State<AppState>) -> ApiResult<QuoteResponse> {
    let symbol = &state.symbol;

    // Try Moomoo first, then fall back to Yahoo.
    let moomoo_ok = moomoo::is_available().await;
    if moomoo_ok {
        match moomoo::fetch_quote(symbol).await {
            Ok(q) => {
                return Ok(Json(QuoteResponse {
                    symbol: q.symbol,
                    price: q.price,
                    prev_close: q.prev_close,
                    change: q.change,
                    change_pct: q.change_pct,
                    timestamp: q.timestamp,
                }));
            }
            Err(e) => {
                tracing::warn!(symbol, error = %e, "Moomoo quote failed — trying Yahoo");
            }
        }
    }

    // Fallback: Yahoo
    let q = yahoo::fetch_quote(symbol)
        .await
        .map_err(|e| internal_error("fetch_quote", e))?;

    Ok(Json(QuoteResponse {
        symbol: q.symbol,
        price: q.price,
        prev_close: q.prev_close,
        change: q.change,
        change_pct: q.change_pct,
        timestamp: q.timestamp,
    }))
}