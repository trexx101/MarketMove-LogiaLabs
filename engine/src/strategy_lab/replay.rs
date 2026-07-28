use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::info;

use crate::db::{self, DbPool};
use crate::strategy::{next_equity_position, Position};
use crate::strategy_lab::{
    BacktestResult, BacktestTrade, BarInput, StrategyKind,
};

/// Run a backtest over the requested time window.
pub async fn run_backtest(
    pool: &DbPool,
    symbol: &str,
    request: &crate::strategy_lab::BacktestRequest,
) -> Result<BacktestResult> {
    let kind = super::parse_strategy_kind(&request.kind, &request.params)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let sma_window = match &kind {
        StrategyKind::Threshold(p) => p.sma_window,
        StrategyKind::Rhai(_) => 200, // default for Rhai
    };

    // 1. Fetch candles in the range, ascending.
    let candles =
        db::fetch_equity_candles_range_asc(pool, symbol, request.start_ts, request.end_ts)
            .await
            .context("fetching equity candles for backtest")?;

    if candles.len() < sma_window + 1 {
        anyhow::bail!(
            "insufficient candle data: need at least {} candles, got {}",
            sma_window + 1,
            candles.len()
        );
    }

    // 2. Fetch predictions in the range, ascending.
    let preds =
        db::fetch_equity_predictions_range(pool, symbol, request.start_ts, request.end_ts)
            .await
            .context("fetching equity predictions for backtest")?;

    let pred_map: HashMap<i64, &db::EquityPredictionRow> =
        preds.iter().map(|p| (p.candle_ts, p)).collect();

    info!(
        "backtest: {} candles, {} predictions in [{}, {}]",
        candles.len(),
        preds.len(),
        request.start_ts,
        request.end_ts
    );

    // 3. Pre-compute trailing SMA series.
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let sma_series = compute_sma_series(&closes, sma_window);

    // 4. Buy-and-hold return (for comparison in metrics).
    let first_close = candles.first().map(|c| c.close).unwrap_or(0.0);
    let last_close = candles.last().map(|c| c.close).unwrap_or(first_close);
    let buy_hold_return = if first_close > 0.0 {
        (last_close - first_close) / first_close
    } else {
        0.0
    };

    // 5. Replay bar-by-bar.
    let mut current_pos: Position = Position::Flat;
    let mut equity_multiplier: f64 = 1.0; // cumulative product of closed-trade returns
    let mut open_trade: Option<OpenTrade> = None;
    let mut completed_trades: Vec<BacktestTrade> = Vec::new();
    let mut equity_curve: Vec<(i64, f64)> = Vec::with_capacity(candles.len());

    for (i, candle) in candles.iter().enumerate() {
        let (sma, sma_valid) = sma_series[i];

        // Look up prediction for this timestamp.
        let pred = pred_map.get(&candle.ts);

        let bar_input = BarInput {
            pred_1d: pred.map(|p| p.pred_1d).unwrap_or(0.0),
            pred_5d: pred.map(|p| p.pred_5d).unwrap_or(0.0),
            pred_21d: pred.map(|p| p.pred_21d).unwrap_or(0.0),
            close: candle.close,
            sma,
            sma_valid,
            current_pos: current_pos.as_i64(),
        };

        let new_pos = match &kind {
            StrategyKind::Threshold(params) => {
                next_equity_position(current_pos, &bar_input.to_equity_signal(), params)
            }
            StrategyKind::Rhai(script) => {
                let signal = bar_input.to_equity_signal();
                match super::rhai_plugin::evaluate_rhai_strategy(
                    script,
                    &signal,
                    current_pos.as_i64(),
                ) {
                    Ok(v) => Position::from_i64(v),
                    Err(e) => {
                        tracing::warn!(ts = candle.ts, error = %e, "Rhai evaluation error; holding");
                        current_pos
                    }
                }
            }
        };

        // Handle position change.
        if new_pos != current_pos {
            // Close current position if any.
            if let Some(ref trade) = open_trade {
                let realized_pnl = compute_realized_pnl(trade, candle.close);
                let completed = BacktestTrade {
                    entry_ts: trade.entry_ts,
                    exit_ts: Some(candle.ts),
                    side: trade.side.clone(),
                    entry_price: trade.entry_price,
                    exit_price: Some(candle.close),
                    realized_pnl,
                };
                equity_multiplier *= 1.0 + realized_pnl;
                completed_trades.push(completed);
            }

            // Open new position if not flat.
            if new_pos != Position::Flat {
                open_trade = Some(OpenTrade {
                    entry_ts: candle.ts,
                    entry_price: candle.close,
                    side: if new_pos == Position::Long {
                        "long".to_string()
                    } else {
                        "short".to_string()
                    },
                });
            } else {
                open_trade = None;
            }

            current_pos = new_pos;
        }

        // Compute current equity for the curve.
        let current_equity = if let Some(ref trade) = open_trade {
            let unrealized = unrealized_pnl(trade, candle.close);
            equity_multiplier * (1.0 + unrealized)
        } else {
            equity_multiplier
        };

        equity_curve.push((candle.ts, current_equity));
    }

    // Close any open position at the end.
    if let Some(ref trade) = open_trade {
        let realized_pnl = compute_realized_pnl(trade, last_close);
        let completed = BacktestTrade {
            entry_ts: trade.entry_ts,
            exit_ts: Some(candles.last().unwrap().ts),
            side: trade.side.clone(),
            entry_price: trade.entry_price,
            exit_price: Some(last_close),
            realized_pnl,
        };
        equity_multiplier *= 1.0 + realized_pnl;
        completed_trades.push(completed);
    }

    // 6. Compute metrics.
    let metrics = super::metrics::compute(&equity_curve, &completed_trades, buy_hold_return);

    Ok(BacktestResult {
        equity_curve,
        metrics,
        trades: completed_trades,
    })
}

/// An in-progress trade during replay.
#[derive(Debug)]
struct OpenTrade {
    entry_ts: i64,
    entry_price: f64,
    side: String,
}

/// Compute realized PnL as a fraction of entry price.
fn compute_realized_pnl(trade: &OpenTrade, exit_price: f64) -> f64 {
    if trade.entry_price <= 0.0 {
        return 0.0;
    }
    match trade.side.as_str() {
        "long" => (exit_price - trade.entry_price) / trade.entry_price,
        "short" => (trade.entry_price - exit_price) / trade.entry_price,
        _ => 0.0,
    }
}

/// Compute unrealized PnL as a fraction of entry price.
fn unrealized_pnl(trade: &OpenTrade, current_price: f64) -> f64 {
    compute_realized_pnl(trade, current_price)
}

/// Compute trailing SMA for each bar in the series.
/// Returns `[(sma_value, is_valid), ...]` where the i-th entry is the SMA
/// over `closes[max(0, i+1 - window)..=i]`.
fn compute_sma_series(closes: &[f64], window: usize) -> Vec<(f64, bool)> {
    let n = closes.len();
    let mut result = Vec::with_capacity(n);
    if window == 0 || n == 0 {
        for _ in 0..n {
            result.push((0.0, false));
        }
        return result;
    }

    let mut running_sum: f64 = 0.0;
    for i in 0..n {
        // Add the incoming bar; for windows beyond the first, drop the bar
        // that just slid out of the trailing window.
        running_sum += closes[i];
        if i >= window {
            running_sum -= closes[i - window];
        }

        let count = (i + 1).min(window);
        let sma = running_sum / count as f64;
        let valid = i + 1 >= window;
        result.push((sma, valid));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sma_series() {
        let closes = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let series = compute_sma_series(&closes, 3);

        // i=0: only [1.0] → avg=1.0, invalid
        assert!((series[0].0 - 1.0).abs() < 1e-9);
        assert!(!series[0].1);

        // i=1: [1.0, 2.0] → avg=1.5, invalid
        assert!((series[1].0 - 1.5).abs() < 1e-9);
        assert!(!series[1].1);

        // i=2: [1.0, 2.0, 3.0] → avg=2.0, valid
        assert!((series[2].0 - 2.0).abs() < 1e-9);
        assert!(series[2].1);

        // i=3: [2.0, 3.0, 4.0] → avg=3.0, valid
        assert!((series[3].0 - 3.0).abs() < 1e-9);
        assert!(series[3].1);

        // i=4: [3.0, 4.0, 5.0] → avg=4.0, valid
        assert!((series[4].0 - 4.0).abs() < 1e-9);
        assert!(series[4].1);
    }

    #[test]
    fn test_compute_sma_series_empty() {
        let series = compute_sma_series(&[], 5);
        assert!(series.is_empty());
    }

    /// Integration test: full backtest replay over an in-memory DB proves the
    /// end-to-end wiring (DB range query -> replay -> metrics) works.
    #[tokio::test]
    async fn integration_backtest_threshold_on_seeded_data() {
        use crate::db::{DbPool, EquityCandle};
        use crate::strategy_lab::BacktestRequest;
        use sqlx::sqlite::SqlitePoolOptions;

        let pool: DbPool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in crate::db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }

        let symbol = "QQQ";
        for i in 0..210u64 {
            let ts = 1_700_000_000 + i * 86_400;
            let close = 400.0 + i as f64;
            let c = EquityCandle {
                symbol: symbol.to_string(),
                ts: ts as i64,
                open: close,
                high: close + 1.0,
                low: close - 1.0,
                close,
                volume: 1_000_000,
                source: "yahoo".to_string(),
            };
            crate::db::upsert_equity_candle(&pool, &c).await.unwrap();
            crate::db::insert_equity_prediction(
                &pool, symbol, ts as i64, 0.01, 0.02, 0.03, "bullish", "{}",
            )
            .await
            .unwrap();
        }

        let req = BacktestRequest {
            strategy_id: None,
            kind: "threshold".to_string(),
            params: serde_json::json!({
                "entry_threshold": 0.003,
                "exit_threshold": -0.001,
                "sma_window": 200
            }),
            start_ts: 1_700_000_000,
            end_ts: 1_700_000_000 + 209 * 86_400,
        };

        let result = super::run_backtest(&pool, symbol, &req)
            .await
            .expect("backtest should run");
        assert!(!result.equity_curve.is_empty());
        assert!(result.metrics.trade_count > 0, "expected trades on uptrend");
        assert!(result.metrics.total_return.is_finite());
        assert!(result.metrics.buy_hold_return > 0.0, "uptrend buy&hold positive");
        eprintln!(
            "VERIFY backtest: trades={} total_return={:.4} buy_hold={:.4} sharpe={:.3} max_dd={:.4}",
            result.metrics.trade_count,
            result.metrics.total_return,
            result.metrics.buy_hold_return,
            result.metrics.sharpe,
            result.metrics.max_drawdown
        );
    }
}