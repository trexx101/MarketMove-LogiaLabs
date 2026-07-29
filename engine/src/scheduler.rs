use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::api::ws::TelemetryEvent;
use crate::api::ws::TelemetrySender;
use crate::bridge::ZmqBridge;
use crate::config::TradingMode;
use crate::db::{self, DbPool};
use crate::exec::ExecutorKind;
use crate::features::equities_v2::{compute_equity_features, EquityNormStats, EQ_FEATURE_DIM};
use crate::strategy::{self, EquityStrategyParams, EquitySignalInput, Position};

/// Daily equities scheduler (Wave C, Phase 3.4 runtime mode swap).
///
/// Polls for new QQQ daily candles, computes 8-dim features, normalizes with
/// median/MAD stats, calls the V3 inference service (1d/5d/21d), persists
/// predictions, and evaluates the long/flat strategy.
///
/// Poll cadence is 5 minutes (daily bars — no need for 30s crypto polling).
///
/// ## Phase 3.4: runtime mode toggle
///
/// The scheduler holds `Arc<RwLock<TradingMode>>` and
/// `Arc<RwLock<ExecutorKind>>` so the `/api/mode` endpoint can flip both
/// while the scheduler is mid-cycle. The scheduler re-borrows the executor
/// briefly at the start of each cycle and releases it before sleeping, so a
/// concurrent flip from the API can land between cycles without contention.
pub struct EquityScheduler {
    pool: DbPool,
    symbol: String,
    bridge: Option<ZmqBridge>,
    norm_stats: EquityNormStats,
    feature_window_size: usize,
    last_processed_ts: Option<i64>,
    strategy_params: EquityStrategyParams,
    /// Trading mode. Read at the start of each cycle; flipped by `POST /api/mode`.
    trading_mode: Arc<RwLock<TradingMode>>,
    /// Executor used to place orders. Read at the start of each cycle; the
    /// runtime toggle can swap it (Paper <-> Moomoo) by acquiring the write
    /// lock between cycles.
    executor: Arc<RwLock<ExecutorKind>>,
    tx: Option<TelemetrySender>,
}

impl EquityScheduler {
    pub async fn new(
        pool: DbPool,
        symbol: String,
        zmq_endpoint: &str,
        norm_stats: EquityNormStats,
        feature_window_size: usize,
        strategy_params: EquityStrategyParams,
        trading_mode: Arc<RwLock<TradingMode>>,
        executor: Arc<RwLock<ExecutorKind>>,
        tx: Option<TelemetrySender>,
    ) -> Result<Self> {
        let bridge = ZmqBridge::connect(zmq_endpoint).await?;
        Ok(Self {
            pool,
            symbol,
            bridge: Some(bridge),
            norm_stats,
            feature_window_size,
            last_processed_ts: None,
            strategy_params,
            trading_mode,
            executor,
            tx,
        })
    }

    /// Poll loop — checks for new daily candles every 5 minutes.
    pub async fn run(&mut self) -> Result<()> {
        loop {
            let now = chrono::Utc::now().timestamp();
            let next_open = crate::market_hours::next_market_open(now);
            let sleep_secs = (next_open - now).max(0) as u64;
            let sleep_secs = sleep_secs.min(300);
            tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
            let latest = db::latest_equity_candle_ts(&self.pool, &self.symbol).await?;
            if let Some(ts) = latest {
                if ts > self.last_processed_ts.unwrap_or(0) {
                    if let Err(e) = self.process(ts).await {
                        error!(error = %e, candle_ts = ts, "failed to process equity candle");
                    }
                }
            }
        }
    }

    /// Fetch candles, compute + normalize features, call inference, persist.
    async fn process(&mut self, candle_ts: i64) -> Result<()> {
        // Need enough candles for feature warmup (50d SMA, 20d correlation, etc.)
        // plus the feature window for the TCN sequence.
        let fetch_count = (self.feature_window_size + 60) as i64;
        let candles = db::fetch_equity_candles_asc(&self.pool, &self.symbol, fetch_count).await?;

        if candles.len() < self.feature_window_size + 50 {
            warn!(
                candle_ts,
                count = candles.len(),
                required = self.feature_window_size + 50,
                "insufficient equity candles, skipping prediction"
            );
            return Ok(());
        }

        // Fetch VIX and TLT close-aligned series (optional — features degrade to 0.0).
        let vix = db::fetch_equity_candles_asc(&self.pool, "^VIX", fetch_count).await.ok();
        let tlt = db::fetch_equity_candles_asc(&self.pool, "TLT", fetch_count).await.ok();

        let vix_close: Option<Vec<f64>> = vix.map(|c| c.iter().map(|c| c.close).collect());
        let tlt_close: Option<Vec<f64>> = tlt.map(|c| c.iter().map(|c| c.close).collect());

        // Compute ATR(14) / close for the last candle — needed by the inference
        // service to denormalize predictions back to raw log-return space.
        let atr_ratio = compute_atr_ratio(&candles);
        let last_close = candles.last().map(|c| c.close).unwrap_or(1.0).max(1.0);

        let all_features = compute_equity_features(
            &candles,
            vix_close.as_deref(),
            tlt_close.as_deref(),
        );

        if all_features.is_empty() {
            warn!(candle_ts, "no features computed, skipping");
            return Ok(());
        }

        // Take last feature_window_size rows.
        let start = all_features.len().saturating_sub(self.feature_window_size);
        let rows = &all_features[start..];

        let feature_window: Vec<[f64; EQ_FEATURE_DIM]> = rows
            .iter()
            .map(|row| self.norm_stats.normalize(row))
            .collect();

        if feature_window.is_empty() {
            warn!(candle_ts, "empty feature window, skipping prediction");
            return Ok(());
        }

        let bridge = self.bridge.as_mut()
            .expect("ZMQ bridge not configured (test mode — use process_with_prediction)");

        let pred = bridge
            .predict_v3_with_retry(&feature_window, atr_ratio, Duration::from_secs(10), 2)
            .await?;

        self.finalize_candle(candle_ts, &pred, &feature_window, &candles).await
    }

    /// Persist prediction, mark candle processed, run strategy.
    async fn finalize_candle(
        &mut self,
        candle_ts: i64,
        pred: &crate::bridge::EquityPrediction,
        feature_window: &[[f64; EQ_FEATURE_DIM]],
        candles: &[crate::db::EquityCandle],
    ) -> Result<()> {
        let features_json = serde_json::to_string(feature_window)?;

        // Compute regime label for audit.
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let (sma, sma_valid) = strategy::compute_sma(&closes, self.strategy_params.sma_window);
        let latest_close = candles.last().map(|c| c.close).unwrap_or(0.0);
        let regime = if !sma_valid {
            "unknown"
        } else if latest_close > sma {
            "bull"
        } else {
            "bear"
        };

        db::insert_equity_prediction(
            &self.pool,
            &self.symbol,
            candle_ts,
            pred.pred_1d,
            pred.pred_5d,
            pred.pred_21d,
            regime,
            &features_json,
        )
        .await?;

        info!(
            candle_ts,
            pred_1d = pred.pred_1d,
            pred_5d = pred.pred_5d,
            pred_21d = pred.pred_21d,
            regime,
            "equity prediction persisted"
        );

        // Publish telemetry event for any connected control-room clients.
        if let Some(tx) = &self.tx {
            let _ = tx.send(TelemetryEvent::PredictionUpdate {
                pred_1d: Some(pred.pred_1d),
                pred_5d: Some(pred.pred_5d),
                pred_21d: Some(pred.pred_21d),
                timestamp: candle_ts,
            });
        }

        self.last_processed_ts = Some(candle_ts);

        // --- Strategy evaluation ---
        if let Err(e) = self.evaluate_and_execute_strategy(candle_ts, pred, &closes, sma, sma_valid).await {
            error!(
                error = %e,
                candle_ts,
                "equity strategy evaluation failed (prediction already persisted)"
            );
        }

        Ok(())
    }

    async fn evaluate_and_execute_strategy(
        &mut self,
        candle_ts: i64,
        pred: &crate::bridge::EquityPrediction,
        closes: &[f64],
        sma: f64,
        sma_valid: bool,
    ) -> Result<()> {
        let current_pos = Position::from_i64(
            db::load_position(&self.pool).await?
        );

        let latest_close = closes.last().copied().unwrap_or(0.0);

        let input = EquitySignalInput {
            pred_1d: pred.pred_1d,
            pred_5d: pred.pred_5d,
            pred_21d: pred.pred_21d,
            current_close: latest_close,
            sma,
            sma_valid,
        };

        let new_pos = strategy::next_equity_position(current_pos, &input, &self.strategy_params);

        let regime: i64 = if sma_valid {
            if latest_close > sma { 1 } else { -1 }
        } else {
            0
        };

        db::insert_position_event(
            &self.pool,
            candle_ts,
            new_pos.as_i64(),
            pred.pred_1d,
            pred.pred_5d,
            regime,
            sma,
        ).await?;

        db::save_position(&self.pool, new_pos.as_i64()).await?;

        if new_pos != current_pos {
            // Phase 3.4: read the current trading mode + executor at the start
            // of the trade. The runtime mode-toggle can swap either between
            // cycles, but the lock is released before we sleep again, so the
            // toggle never blocks a trade in flight.
            let mode = *self.trading_mode.read().await;
            match mode {
                TradingMode::Paper => {
                    // Paper mode uses the current executor (which is the
                    // PaperExecutor unless an earlier flip swapped it in).
                    let mut exec_guard = self.executor.write().await;
                    match exec_guard.set_target_position(new_pos, latest_close, candle_ts).await {
                        Ok(fills) => {
                            let total_pnl: f64 = fills.iter().map(|f| f.realized_pnl).sum();
                            for fill in &fills {
                                info!(
                                    side = ?fill.side,
                                    qty = fill.qty,
                                    price = fill.price,
                                    fee = fill.fee,
                                    pnl = fill.realized_pnl,
                                    "equity trade executed"
                                );
                            }

                            // Publish PnL tick after trade execution.
                            if let Some(tx) = &self.tx {
                                let _ = tx.send(TelemetryEvent::PnlTick {
                                    realized_pnl: total_pnl,
                                    unrealized_pnl: 0.0,
                                    position: format!("{}", new_pos).to_lowercase(),
                                    entry_price: if new_pos != Position::Flat {
                                        Some(latest_close)
                                    } else {
                                        None
                                    },
                                    last_close: Some(latest_close),
                                    timestamp: candle_ts,
                                });
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "equity executor failed to place order");
                        }
                    }
                }
                TradingMode::Live => {
                    // Live mode is gated by the runtime toggle. The API
                    // endpoint (POST /api/mode) flips the mode + executor
                    // atomically; here we just dispatch.
                    let mut exec_guard = self.executor.write().await;
                    match exec_guard.set_target_position(new_pos, latest_close, candle_ts).await {
                        Ok(fills) => {
                            let total_pnl: f64 = fills.iter().map(|f| f.realized_pnl).sum();
                            for fill in &fills {
                                info!(
                                    side = ?fill.side,
                                    qty = fill.qty,
                                    price = fill.price,
                                    fee = fill.fee,
                                    pnl = fill.realized_pnl,
                                    "equity LIVE trade executed"
                                );
                            }
                            if let Some(tx) = &self.tx {
                                let _ = tx.send(TelemetryEvent::PnlTick {
                                    realized_pnl: total_pnl,
                                    unrealized_pnl: 0.0,
                                    position: format!("{}", new_pos).to_lowercase(),
                                    entry_price: if new_pos != Position::Flat {
                                        Some(latest_close)
                                    } else {
                                        None
                                    },
                                    last_close: Some(latest_close),
                                    timestamp: candle_ts,
                                });
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "equity LIVE executor failed to place order");
                        }
                    }
                }
            }
            info!(
                candle_ts,
                prev = %current_pos,
                next = %new_pos,
                mode = %mode,
                regime,
                sma,
                "equity position changed"
            );
        } else {
            tracing::debug!(candle_ts, position = %new_pos, "equity position unchanged");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ATR helper
// ---------------------------------------------------------------------------

/// Compute ATR(14) / close for the most recent candle.
/// Uses Wilder's EMA smoothing (alpha = 1/14), matching the Colab label
/// computation exactly. Returns 0.005 (≈ 0.5%, a reasonable QQQ default)
/// if insufficient data is available.
fn compute_atr_ratio(candles: &[crate::db::EquityCandle]) -> f64 {
    let n = candles.len();
    if n < 15 {
        return 0.005;
    }

    // Compute True Range for all bars
    let mut tr = Vec::with_capacity(n);
    tr.push(0.0); // tr[0] = 0
    for i in 1..n {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_close = candles[i - 1].close;
        let h_l = high - low;
        let h_c = (high - prev_close).abs();
        let l_c = (low - prev_close).abs();
        tr.push(h_l.max(h_c).max(l_c));
    }

    // Wilder's smoothing of TR over 14 periods
    let period = 14.0_f64;
    let mut atr = 0.0_f64;
    // First value = simple average of first 14 TRs
    let warmup: f64 = tr[1..=14].iter().sum::<f64>() / period;
    atr = warmup;
    // Then Wilder update
    for i in 15..n {
        atr = (atr * (period - 1.0) + tr[i]) / period;
    }

    let last_close = candles.last().map(|c| c.close).unwrap_or(1.0).max(1e-6);
    if atr <= 0.0 || last_close <= 0.0 {
        0.005
    } else {
        atr / last_close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::EquityPrediction;
    use crate::db::{self, EquityCandle, DbPool};
    use crate::exec::paper::PaperExecutor;
    use crate::features::equities_v2::EquityNormStats;
    use crate::strategy::EquityStrategyParams;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        db::migrate_predictions(&pool).await.unwrap();
        pool
    }

    fn test_norm_stats() -> EquityNormStats {
        EquityNormStats {
            median: [0.0; EQ_FEATURE_DIM],
            mad: [1.0; EQ_FEATURE_DIM],
        }
    }

    fn test_scheduler(pool: DbPool) -> EquityScheduler {
        let executor = ExecutorKind::Paper(PaperExecutor::new(pool.clone(), 0.001, None));
        EquityScheduler {
            pool,
            symbol: "QQQ".to_string(),
            bridge: None,
            norm_stats: test_norm_stats(),
            feature_window_size: 10,
            last_processed_ts: None,
            strategy_params: EquityStrategyParams::default(),
            trading_mode: Arc::new(RwLock::new(TradingMode::Paper)),
            executor: Arc::new(RwLock::new(executor)),
            tx: None,
        }
    }

    async fn seed_equity_candles(pool: &DbPool, count: usize) {
        for i in 0..count {
            let ts = (i as i64) * 86400; // daily bars
            let price = 400.0 + (i as f64) * 0.5;
            db::upsert_equity_candle(pool, &EquityCandle {
                symbol: "QQQ".to_string(),
                ts,
                open: price,
                high: price + 2.0,
                low: price - 2.0,
                close: price + 1.0,
                volume: 1000000,
                source: "yahoo".to_string(),
            }).await.unwrap();
        }
    }

    fn fake_prediction() -> EquityPrediction {
        EquityPrediction {
            pred_1d: 0.005,
            pred_5d: 0.015,
            pred_21d: 0.04,
        }
    }

    /// After prediction persistence, last_processed_ts must be set even if
    /// strategy evaluation fails.
    #[tokio::test]
    async fn process_sets_last_processed_after_prediction() {
        let pool = test_pool().await;
        seed_equity_candles(&pool, 80).await;

        let mut sched = test_scheduler(pool.clone());
        let candle_ts: i64 = 79 * 86400;

        let pred = fake_prediction();
        let feature_window = vec![[0.1; EQ_FEATURE_DIM]; 10];
        let candles = db::fetch_equity_candles_asc(&pool, "QQQ", 100).await.unwrap();

        let result = sched.finalize_candle(
            candle_ts, &pred, &feature_window, &candles,
        ).await;

        assert!(result.is_ok());
        assert_eq!(sched.last_processed_ts, Some(candle_ts));
    }

    /// Insufficient candles → skip, no prediction inserted.
    #[tokio::test]
    async fn process_skips_when_insufficient_candles() {
        let pool = test_pool().await;
        seed_equity_candles(&pool, 5).await; // too few

        let mut sched = test_scheduler(pool.clone());
        let candle_ts: i64 = 4 * 86400;

        let result = sched.process(candle_ts).await;
        assert!(result.is_ok());
    }
}
