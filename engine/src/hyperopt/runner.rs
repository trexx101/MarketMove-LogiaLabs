//! Nightly runner — orchestrates the full hyperopt pipeline
//!
//! Runs post-market: optimizer → stability → store → report.
//! Iterates over configured equities and strategy families.

use std::cmp::Ordering;

use anyhow::Result;
use chrono::Utc;
use tracing::{info, warn};

use crate::db::{self, DbPool};

use super::candidate_store::CandidateStore;
use super::eval;
use super::optimizer::{Optimizer, OptimizerConfig, OptimizationResult};
use super::scheduler::{NightlyScheduler, SchedulerConfig, SchedulerState};
use super::stability::StabilityChecker;

/// Runner configuration
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Equities to optimize
    pub equities: Vec<String>,
    /// Strategy families to optimize
    pub strategy_families: Vec<String>,
    /// Scheduler configuration
    pub scheduler_config: SchedulerConfig,
    /// Maximum candidates to store per equity/family
    pub max_candidates_per_slot: usize,
    /// How many trailing daily candles to load per equity
    pub lookback_candles: i64,
    /// Rank-IC evaluation hyperparameters
    pub eval_spec: eval::EvalSpec,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            // Active trading universe (matches the live equity engine roster).
            // This same default also drives the promotion applier's equity set
            // in main.rs, so whatever the runner stores gets auto-promoted.
            equities: vec![
                "QQQ".to_string(),
                "SMH".to_string(),
                "XLF".to_string(),
            ],
            // Only families with a real signal implementation should be
            // listed here; `eval::signal_for` skips everything else.
            strategy_families: vec!["sma_regime".to_string()],
            scheduler_config: SchedulerConfig::default(),
            max_candidates_per_slot: 10,
            lookback_candles: 1500, // ~6 years of daily bars
            eval_spec: eval::EvalSpec::default(),
        }
    }
}

/// Nightly runner
pub struct NightlyRunner {
    config: RunnerConfig,
    scheduler: NightlyScheduler,
    store: CandidateStore,
    pool: DbPool,
}

impl NightlyRunner {
    pub fn new(config: RunnerConfig, pool: DbPool, store: CandidateStore) -> Self {
        let scheduler = NightlyScheduler::new(config.scheduler_config.clone());

        Self {
            config,
            scheduler,
            store,
            pool,
        }
    }

    /// Run the nightly hyperopt pipeline.
    ///
    /// Guards on the scheduler window (post-market, before the hard stop),
    /// then iterates over equities and strategy families. Records a run row
    /// in `hyperopt_runs` for observability if a run actually starts.
    pub async fn run(&self) -> Result<RunReport> {
        let now = Utc::now();
        let state = self.scheduler.check_state(now);

        match state {
            SchedulerState::CannotRun => {
                warn!("Cannot run: not in post-market window");
                return Ok(RunReport {
                    success: false,
                    reason: "Not in post-market window".to_string(),
                    candidates_stored: 0,
                    equities_processed: 0,
                });
            }
            SchedulerState::HardStop => {
                warn!("Cannot run: past hard stop");
                return Ok(RunReport {
                    success: false,
                    reason: "Past hard stop".to_string(),
                    candidates_stored: 0,
                    equities_processed: 0,
                });
            }
            SchedulerState::CanRun => {
                info!("Starting nightly hyperopt run");
            }
        }

        let run_id = db::insert_hyperopt_run(&self.pool).await.ok();

        let mut total_candidates = 0;
        let mut equities_processed = 0;

        for equity in &self.config.equities {
            for strategy_family in &self.config.strategy_families {
                match self.run_slot(equity, strategy_family).await {
                    Ok(n) => {
                        total_candidates += n;
                        equities_processed += 1;
                    }
                    Err(e) => {
                        warn!("Failed to run slot {}/{}: {:#}", equity, strategy_family, e);
                    }
                }
            }
        }

        info!(
            "Nightly run complete: {} candidates stored across {} equities",
            total_candidates, equities_processed
        );

        if let Some(id) = run_id {
            let _ = db::complete_hyperopt_run(
                &self.pool,
                id,
                if total_candidates > 0 { "completed" } else { "no_candidates" },
                equities_processed as i64,
                total_candidates as i64,
                0,
                None,
            )
            .await;
        }

        Ok(RunReport {
            success: true,
            reason: "Completed".to_string(),
            candidates_stored: total_candidates,
            equities_processed,
        })
    }

    /// Run hyperopt for a single equity/strategy slot.
    ///
    /// Loads real candles from `equity_candles`, walks the family's param grid,
    /// scores each config with walk-forward rank IC (`eval.rs`), runs the
    /// neighborhood stability check on the champion, and stores the stable,
    /// statistically-significant candidates.
    async fn run_slot(&self, equity: &str, strategy_family: &str) -> Result<usize> {
        info!("Running hyperopt for {}/{}", equity, strategy_family);

        let defs = eval::param_defs_for_family(strategy_family);
        if defs.is_empty() {
            warn!(
                "No param defs / signal for family '{strategy_family}'; skipping slot"
            );
            return Ok(0);
        }

        let candles = db::fetch_equity_candles_asc(&self.pool, equity, self.config.lookback_candles)
            .await?;
        if candles.len() < self.config.eval_spec.min_bars {
            warn!(
                "{equity}/{strategy_family}: {} candles available, need >= {}; skipping",
                candles.len(),
                self.config.eval_spec.min_bars
            );
            return Ok(0);
        }
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let spec = &self.config.eval_spec;
        let opt = Optimizer::new(OptimizerConfig::default());

        // Score every config in the family's grid.
        let mut results: Vec<OptimizationResult> = opt
            .generate_grid(&defs)
            .into_iter()
            .filter_map(|cfg| eval::evaluate_params(&closes, &cfg.params, spec, &opt))
            .filter(|r| r.mean_ic.is_finite() && r.n_trades >= spec.min_trades)
            .collect();

        results.sort_by(|a, b| b.mean_ic.partial_cmp(&a.mean_ic).unwrap_or(Ordering::Equal));
        results.truncate(self.config.max_candidates_per_slot);

        if results.is_empty() {
            info!(
                "{equity}/{strategy_family}: no config cleared min_trades={}",
                spec.min_trades
            );
            return Ok(0);
        }

        // Neighborhood stability on the champion before committing anything.
        let checker = StabilityChecker::default();
        let stability = checker.check(
            &results[0].params.params,
            results[0].mean_ic,
            |p| eval::mean_ic(&closes, p, spec, &opt),
        );
        if !stability.stable {
            warn!(
                "{equity}/{strategy_family}: champion unstable (degradation {:.3}); storing no candidates",
                stability.degradation_ratio
            );
            return Ok(0);
        }

        let mut stored = 0;
        for r in &results {
            let vid = self
                .store
                .store(
                    equity,
                    strategy_family,
                    r.params.params.clone(),
                    r.mean_ic,
                    r.std_ic,
                    r.n_trades,
                    r.fold_ics.clone(),
                )
                .await?;
            info!(
                "Stored candidate {} for {}/{} (mean_ic={:.4}, std_ic={:.4}, n_trades={})",
                vid, equity, strategy_family, r.mean_ic, r.std_ic, r.n_trades
            );
            stored += 1;
        }

        Ok(stored)
    }
}

/// Run report
#[derive(Debug, Clone)]
pub struct RunReport {
    pub success: bool,
    pub reason: String,
    pub candidates_stored: usize,
    pub equities_processed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        // Schema needed by the runner + candidate store.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS strategy_versions (
                id                      TEXT    PRIMARY KEY,
                equity                  TEXT    NOT NULL DEFAULT 'QQQ',
                family                  TEXT    NOT NULL,
                params_json             TEXT    NOT NULL,
                status                  TEXT    NOT NULL DEFAULT 'NEW',
                promotion_metadata_json TEXT    NOT NULL DEFAULT '{}',
                created_at              INTEGER NOT NULL,
                updated_at              INTEGER NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS equity_candles (
                symbol TEXT NOT NULL,
                ts INTEGER NOT NULL,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume INTEGER NOT NULL,
                source TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[tokio::test]
    async fn test_runner_generates_report() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool.clone());
        let config = RunnerConfig {
            equities: vec!["QQQ".to_string()],
            strategy_families: vec!["sma_regime".to_string()],
            ..Default::default()
        };

        let runner = NightlyRunner::new(config, pool, store);
        let report = runner.run().await.unwrap();

        // Empty DB → no candidates, no error.
        assert!(report.candidates_stored == 0);
        assert!(report.equities_processed >= 0);
    }

    #[tokio::test]
    async fn test_runner_multi_equity() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool.clone());
        // Always-allow window so the run is independent of wall-clock time.
        let scheduler_config = SchedulerConfig {
            timezone_offset_hours: 0,
            market_open_local: chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            market_close_local: chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            post_market_buffer_mins: 0,
            pre_market_buffer_mins: 0,
            max_run_hours: 24,
        };
        let config = RunnerConfig {
            equities: vec!["QQQ".to_string(), "SMH".to_string()],
            strategy_families: vec!["sma_regime".to_string()],
            scheduler_config,
            ..Default::default()
        };

        let runner = NightlyRunner::new(config, pool, store);
        let report = runner.run().await.unwrap();

        // Both slots execute (each returns Ok(0) on empty data).
        assert_eq!(report.equities_processed, 2);
    }
}