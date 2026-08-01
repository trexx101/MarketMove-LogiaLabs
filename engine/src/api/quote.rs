use axum::{extract::State, response::Json};
use serde::Serialize;

use crate::data::yahoo;

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

pub(crate) async fn handle_quote(State(state): State<AppState>) -> ApiResult<QuoteResponse> {
    let quote = yahoo::fetch_quote(&state.symbol)
        .await
        .map_err(|e| internal_error("fetch_quote", e))?;

    Ok(Json(QuoteResponse {
        symbol: quote.symbol,
        price: quote.price,
        prev_close: quote.prev_close,
        change: quote.change,
        change_pct: quote.change_pct,
        timestamp: quote.timestamp,
    }))
}
