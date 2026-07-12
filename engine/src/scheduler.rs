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
    bridge: ZmqBridge,
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
            bridge,
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

        let pred = self
            .bridge
            .predict_with_retry(&feature_window, Duration::from_secs(5), 2)
            .await?;

        let features_json = serde_json::to_string(&feature_window)?;

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

        // --- Strategy evaluation ---
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

        self.last_processed_ts = Some(candle_ts);

        Ok(())
    }
}
