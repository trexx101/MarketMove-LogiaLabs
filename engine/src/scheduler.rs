use std::time::Duration;

use anyhow::Result;
use tracing::{error, info, warn};

use crate::bridge::ZmqBridge;
use crate::db::{self, DbPool};
use crate::exec::ExecutorKind;
use crate::features::compute_features;
use crate::normalize::{normalize_row, NormStats};

pub struct Scheduler {
    pool: DbPool,
    bridge: Option<ZmqBridge>,
    norm_stats: NormStats,
    feature_window_size: usize,
    last_processed_ts: Option<i64>,
    strategy_params: crate::strategy::StrategyParams,
    executor: ExecutorKind,
}

impl Scheduler {
    /// Connect to ZMQ and prepare the scheduler.
    pub async fn new(
        pool: DbPool,
        zmq_endpoint: &str,
        norm_stats: NormStats,
        feature_window_size: usize,
        strategy_params: crate::strategy::StrategyParams,
        executor: ExecutorKind,
    ) -> Result<Self> {
        let bridge = ZmqBridge::connect(zmq_endpoint).await?;
        Ok(Self {
            pool,
            bridge: Some(bridge),
            norm_stats,
            feature_window_size,
            last_processed_ts: None,
            strategy_params,
            executor,
        })
    }

    /// Poll loop — runs forever under normal operation.
    /// Checks for new candles every 30 seconds.
    /// Returns Err only on unrecoverable DB errors.
    pub async fn run(&mut self) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;

            let latest = db::latest_ts(&self.pool).await?;

            if let Some(ts) = latest {
                if ts > self.last_processed_ts.unwrap_or(0) {
                    if let Err(e) = self.process(ts).await {
                        error!(error = %e, candle_ts = ts, "failed to process candle");
                    }
                }
            }
        }
    }

    /// Fetch candles, compute + normalize features, call inference, persist prediction.
    async fn process(&mut self, candle_ts: i64) -> Result<()> {
        // Fetch extra candle for ATR warmup
        let fetch_count = self.feature_window_size + 1;
        let candles = db::fetch_recent_candles(&self.pool, fetch_count).await?;

        if candles.len() < self.feature_window_size + 1 {
            warn!(
                candle_ts,
                count = candles.len(),
                required = self.feature_window_size + 1,
                "insufficient candles, skipping prediction"
            );
            return Ok(());
        }

        let all_features = compute_features(&candles);

        // Take last feature_window_size rows (or all if fewer)
        let rows = if all_features.len() > self.feature_window_size {
            &all_features[all_features.len() - self.feature_window_size..]
        } else {
            &all_features[..]
        };

        let feature_window: Vec<[f64; 3]> = rows
            .iter()
            .map(|row| normalize_row(row, &self.norm_stats))
            .collect();

        if feature_window.is_empty() {
            warn!(candle_ts, "empty feature window, skipping prediction");
            return Ok(());
        }

        let bridge = self.bridge.as_mut()
            .expect("ZMQ bridge not configured (test mode — use process_with_prediction)");
        let pred = bridge
            .predict_with_retry(&feature_window, Duration::from_secs(5), 2)
            .await?;

        self.finalize_candle(candle_ts, &pred, &feature_window, &candles).await
    }

    /// Persist prediction, mark candle as processed, then run strategy evaluation.
    ///
    /// `last_processed_ts` is set immediately after `insert_prediction` succeeds so
    /// that the 30-second poll loop in `run()` will NOT re-request inference for
    /// this candle even if strategy evaluation subsequently fails.
    async fn finalize_candle(
        &mut self,
        candle_ts: i64,
        pred: &crate::bridge::Prediction,
        feature_window: &[[f64; 3]],
        candles: &[crate::db::Candle],
    ) -> Result<()> {
        let features_json = serde_json::to_string(feature_window)?;

        db::insert_prediction(
            &self.pool,
            candle_ts,
            pred.pred_1h,
            pred.pred_4h,
            pred.pred_24h,
            &features_json,
        )
        .await?;

        info!(
            candle_ts,
            pred_1h = pred.pred_1h,
            pred_4h = pred.pred_4h,
            pred_24h = pred.pred_24h,
            "prediction persisted"
        );

        // Mark candle as processed IMMEDIATELY after prediction persistence.
        // This prevents the 30s retry loop: even if strategy evaluation below
        // fails, the prediction is already persisted and must not be re-requested.
        self.last_processed_ts = Some(candle_ts);

        // --- Strategy evaluation ---
        // Errors here are logged but do NOT propagate — the prediction is
        // already persisted and the candle is marked processed.
        if let Err(e) = self.evaluate_and_execute_strategy(candle_ts, pred, candles).await {
            error!(
                error = %e,
                candle_ts,
                "strategy evaluation failed (prediction already persisted)"
            );
        }

        Ok(())
    }

    /// Evaluate the trading strategy and execute position changes.
    ///
    /// Returns `Err` on DB or computation failures — the caller decides
    /// whether to propagate or log-and-continue.
    async fn evaluate_and_execute_strategy(
        &mut self,
        candle_ts: i64,
        pred: &crate::bridge::Prediction,
        candles: &[crate::db::Candle],
    ) -> Result<()> {
        // Load current persisted position
        let current_pos = crate::strategy::Position::from_i64(
            db::load_position(&self.pool).await?
        );

        // Compute SMA using all fetched candles
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let (sma, sma_valid) = crate::strategy::compute_sma(&closes, self.strategy_params.sma_window);

        let latest_close = candles.last().map(|c| c.close).unwrap_or(0.0);

        let input = crate::strategy::SignalInput {
            pred_4h: pred.pred_4h,
            pred_24h: pred.pred_24h,
            current_close: latest_close,
            sma,
            sma_valid,
        };

        let new_pos = crate::strategy::next_position(current_pos, &input, &self.strategy_params);

        // Regime for audit log
        let regime: i64 = if sma_valid {
            if latest_close > sma { 1 } else { -1 }
        } else {
            0
        };

        // Always persist the position event (audit trail)
        db::insert_position_event(
            &self.pool,
            candle_ts,
            new_pos.as_i64(),
            pred.pred_4h,
            pred.pred_24h,
            regime,
            sma,
        ).await?;

        // Persist signal_state (for restart resume)
        db::save_position(&self.pool, new_pos.as_i64()).await?;

        // Execute trades if position changed
        if new_pos != current_pos {
            match self.executor.set_target_position(new_pos, latest_close, candle_ts).await {
                Ok(fills) => {
                    for fill in &fills {
                        info!(
                            side = ?fill.side,
                            qty = fill.qty,
                            price = fill.price,
                            fee = fill.fee,
                            pnl = fill.realized_pnl,
                            "trade executed"
                        );
                    }
                }
                Err(e) => {
                    error!(error = %e, "executor failed to place order");
                }
            }
            info!(
                candle_ts,
                prev = %current_pos,
                next = %new_pos,
                regime,
                sma,
                "position changed"
            );
        } else {
            tracing::debug!(candle_ts, position = %new_pos, "position unchanged");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Prediction;
    use crate::db::{self, Candle, DbPool};
    use crate::exec::paper::PaperExecutor;
    use crate::normalize::NormStats;
    use crate::strategy::StrategyParams;
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

    fn test_scheduler(pool: DbPool) -> Scheduler {
        let executor = ExecutorKind::Paper(PaperExecutor::new(pool.clone(), 0.0015));
        Scheduler {
            pool,
            bridge: None,
            norm_stats: NormStats {
                mean: [0.0; 3],
                std: [1.0; 3],
            },
            feature_window_size: 10,
            last_processed_ts: None,
            strategy_params: StrategyParams {
                magnitude_threshold: 0.005,
                sma_window: 200,
            },
            executor,
        }
    }

    async fn seed_candles(pool: &DbPool, count: usize) {
        for i in 0..count {
            let ts = (i as i64) * 3600;
            let price = 50000.0 + (i as f64) * 10.0;
            db::upsert_candle(pool, &Candle {
                ts,
                open: price,
                high: price + 100.0,
                low: price - 100.0,
                close: price + 50.0,
                volume: 1000.0,
                vwap: price + 25.0,
            }).await.unwrap();
        }
    }

    fn fake_prediction() -> Prediction {
        Prediction {
            pred_1h: 0.01,
            pred_4h: 0.02,
            pred_24h: 0.03,
        }
    }

    /// After `insert_prediction` succeeds, `last_processed_ts` must be set
    /// even if subsequent strategy operations fail.
    #[tokio::test]
    async fn process_sets_last_processed_after_prediction() {
        let pool = test_pool().await;
        seed_candles(&pool, 15).await;

        let mut sched = test_scheduler(pool.clone());
        let candle_ts: i64 = 14 * 3600;

        let pred = fake_prediction();
        let feature_window = vec![[0.1, 0.2, 0.3]; 10];
        let candles = db::fetch_recent_candles(&pool, 16).await.unwrap();

        // finalize_candle persists prediction, sets last_processed_ts, runs strategy
        let result = sched.finalize_candle(
            candle_ts, &pred, &feature_window, &candles,
        ).await;

        assert!(result.is_ok(), "finalize_candle should return Ok");
        assert_eq!(
            sched.last_processed_ts, Some(candle_ts),
            "last_processed_ts must be set after prediction persistence"
        );

        // Verify prediction was actually persisted
        let preds = db::fetch_recent_predictions(&pool, 1).await.unwrap();
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].candle_ts, candle_ts);
    }

    /// Strategy/execution errors must be logged but must NOT prevent the
    /// candle from being marked as processed (prevents 30s retry loop).
    #[tokio::test]
    async fn process_does_not_retry_on_strategy_failure() {
        let pool = test_pool().await;
        seed_candles(&pool, 15).await;

        // Drop signal_state table to force load_position to fail inside strategy
        sqlx::query("DROP TABLE signal_state")
            .execute(&pool)
            .await
            .unwrap();

        let mut sched = test_scheduler(pool.clone());
        let candle_ts: i64 = 14 * 3600;

        let pred = fake_prediction();
        let feature_window = vec![[0.1, 0.2, 0.3]; 10];
        let candles = db::fetch_recent_candles(&pool, 16).await.unwrap();

        // finalize_candle should return Ok even though strategy fails
        let result = sched.finalize_candle(
            candle_ts, &pred, &feature_window, &candles,
        ).await;

        assert!(
            result.is_ok(),
            "finalize_candle must return Ok even when strategy fails (got: {:?})",
            result.err()
        );
        assert_eq!(
            sched.last_processed_ts, Some(candle_ts),
            "last_processed_ts must be set despite strategy failure"
        );

        // Verify prediction was persisted (it happened before strategy)
        let preds = db::fetch_recent_predictions(&pool, 1).await.unwrap();
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].candle_ts, candle_ts);
    }

    /// With fewer candles than feature_window_size + 1, process() must skip
    /// inference and return Ok without inserting a prediction.
    #[tokio::test]
    async fn process_skips_when_insufficient_candles() {
        let pool = test_pool().await;
        // Seed only 5 candles — less than feature_window_size(10) + 1 = 11
        seed_candles(&pool, 5).await;

        let mut sched = test_scheduler(pool.clone());
        let candle_ts: i64 = 4 * 3600;

        // Call process directly — bridge is None so if it reaches inference it will panic
        // The guard should prevent that
        let result = sched.process(candle_ts).await;
        assert!(result.is_ok(), "process should return Ok when candles are insufficient");

        // No prediction should have been inserted
        let preds = db::fetch_recent_predictions(&pool, 1).await.unwrap();
        assert_eq!(preds.len(), 0, "no prediction should be inserted when candles are insufficient");
    }
}
