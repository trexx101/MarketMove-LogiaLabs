//! Premium synthesis: generate synthetic option prices from underlying OHLCV
//!
//! Takes historical underlying price data and generates a time series of option
//! prices using Black-Scholes-Merton with constant implied volatility.

use crate::db::{DbPool, EquityCandle};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use sqlx::Row;
use tracing::info;

use super::bsm::{OptionSpec, OptionType, price_option};

/// Synthesized option price at a point in time
#[derive(Debug, Clone)]
pub struct SynthesizedOptionPrice {
    pub timestamp: NaiveDateTime,
    pub underlying_price: f64,
    pub strike: f64,
    pub expiry: NaiveDate,
    pub option_type: OptionType,
    pub time_to_expiry_days: f64,
    pub price: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
}

/// Convert Unix timestamp (seconds) to NaiveDateTime
fn ts_to_datetime(ts: i64) -> NaiveDateTime {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .naive_utc()
}

/// Synthesize option prices for a given underlying symbol
///
/// For each candle in the underlying OHLCV data, generates option prices at
/// the close price for a range of strikes and expiries.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `symbol` - Underlying symbol (e.g., "QQQ")
/// * `expiry` - Option expiry date
/// * `iv` - Constant implied volatility (e.g., 0.25 for 25%)
/// * `risk_free_rate` - Risk-free interest rate (e.g., 0.05 for 5%)
/// * `strike_offsets` - Strike offsets as fractions of underlying price (e.g., [-0.05, 0.0, 0.05])
///
/// # Returns
/// Vector of synthesized option prices, sorted by timestamp
pub async fn synthesize_option_prices(
    pool: &DbPool,
    symbol: &str,
    expiry: NaiveDate,
    iv: f64,
    risk_free_rate: f64,
    strike_offsets: &[f64],
) -> Result<Vec<SynthesizedOptionPrice>> {
    // Fetch underlying OHLCV data
    let candles = sqlx::query_as::<_, EquityCandle>(
        "SELECT symbol, ts, open, high, low, close, volume, source
         FROM equity_candles
         WHERE symbol = ? AND ts < ?
         ORDER BY ts ASC"
    )
    .bind(symbol)
    .bind(expiry.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp())
    .fetch_all(pool)
    .await
    .context("Failed to fetch underlying candles")?;

    if candles.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    for candle in &candles {
        let candle_date = ts_to_datetime(candle.ts).date();
        let days_to_expiry = (expiry - candle_date).num_days() as f64;
        if days_to_expiry <= 0.0 {
            continue; // Skip expired options
        }

        let time_to_expiry_years = days_to_expiry / 365.0;

        // Generate prices for each strike offset
        for &offset in strike_offsets {
            let strike = candle.close * (1.0 + offset);

            // Call option
            let call_spec = OptionSpec {
                underlying_price: candle.close,
                strike,
                time_to_expiry_years,
                risk_free_rate,
                volatility: iv,
                option_type: OptionType::Call,
            };
            let call_price = price_option(&call_spec);

            results.push(SynthesizedOptionPrice {
                timestamp: ts_to_datetime(candle.ts),
                underlying_price: candle.close,
                strike,
                expiry,
                option_type: OptionType::Call,
                time_to_expiry_days: days_to_expiry,
                price: call_price.price,
                delta: call_price.delta,
                gamma: call_price.gamma,
                theta: call_price.theta,
                vega: call_price.vega,
            });

            // Put option
            let put_spec = OptionSpec {
                underlying_price: candle.close,
                strike,
                time_to_expiry_years,
                risk_free_rate,
                volatility: iv,
                option_type: OptionType::Put,
            };
            let put_price = price_option(&put_spec);

            results.push(SynthesizedOptionPrice {
                timestamp: ts_to_datetime(candle.ts),
                underlying_price: candle.close,
                strike,
                expiry,
                option_type: OptionType::Put,
                time_to_expiry_days: days_to_expiry,
                price: put_price.price,
                delta: put_price.delta,
                gamma: put_price.gamma,
                theta: put_price.theta,
                vega: put_price.vega,
            });
        }
    }

    info!(
        symbol = symbol,
        expiry = %expiry,
        candles = candles.len(),
        synthesized = results.len(),
        "Synthesized option prices"
    );

    Ok(results)
}
