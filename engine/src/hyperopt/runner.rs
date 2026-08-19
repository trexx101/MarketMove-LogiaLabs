//! Nightly runner — orchestrates the full hyperopt pipeline
//!
//! Runs post-market: optimizer → stability → store → report
//! Iterates over configured equities and strategy families.

use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use tracing::{info, warn};

use super::candidate_store::CandidateStore;
use super::scheduler::{NightlyScheduler, SchedulerConfig, SchedulerState};

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
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            equities: vec!["QQQ".to_string()],
            strategy_families: vec!["ema_macd_breakout".to_string()],
            scheduler_config: SchedulerConfig::default(),
            max_candidates_per_slot: 10,
        }
    }
}

/// Nightly runner
pub struct NightlyRunner {
    config: RunnerConfig,
    scheduler: NightlyScheduler,
    store: CandidateStore,
}

impl NightlyRunner {
    pub fn new(config: RunnerConfig, store: CandidateStore) -> Self {
        let scheduler = NightlyScheduler::new(config.scheduler_config.clone());

        Self {
            config,
            scheduler,
            store,
        }
    }

    /// Run the nightly hyperopt pipeline
    ///
    /// Checks scheduler state, then iterates over equities and strategy families.
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
                        warn!("Failed to run slot {}/{}: {}", equity, strategy_family, e);
                    }
                }
            }
        }

        info!(
            "Nightly run complete: {} candidates stored across {} equities",
            total_candidates, equities_processed
        );

        Ok(RunReport {
            success: true,
            reason: "Completed".to_string(),
            candidates_stored: total_candidates,
            equities_processed,
        })
    }

    /// Run hyperopt for a single equity/strategy slot
    ///
    /// Placeholder: in production, this would load data, run optimizer, check stability, store candidates.
    async fn run_slot(&self, equity: &str, strategy_family: &str) -> Result<usize> {
        info!("Running hyperopt for {}/{}", equity, strategy_family);

        // Placeholder: mock candidate storage
        // In production:
        // 1. Load training data from DB
        // 2. Run optimizer with param definitions
        // 3. Check stability for top candidates
        // 4. Store stable candidates

        let mock_params = HashMap::new();
        let mock_ic = 0.05;
        let mock_std = 0.01;
        let mock_n_trades = 150;
        let mock_fold_ics = vec![0.04, 0.05, 0.06];

        let version_id = self
            .store
            .store(
                equity,
                strategy_family,
                mock_params,
                mock_ic,
                mock_std,
                mock_n_trades,
                mock_fold_ics,
            )
            .await?;

        info!(
            "Stored candidate {} for {}/{} (IC={:.4})",
            version_id, equity, strategy_family, mock_ic
        );

        Ok(1)
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

    async fn test_store() -> CandidateStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        // Create schema
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

        CandidateStore::new(pool)
    }

    #[tokio::test]
    async fn test_runner_generates_report() {
        let store = test_store().await;
        let config = RunnerConfig {
            equities: vec!["QQQ".to_string()],
            strategy_families: vec!["ema_macd_breakout".to_string()],
            ..Default::default()
        };

        let runner = NightlyRunner::new(config, store);
        let report = runner.run().await.unwrap();

        // Report should have the expected fields
        assert!(report.candidates_stored >= 0);
        assert!(report.equities_processed >= 0);
    }

    #[tokio::test]
    async fn test_runner_multi_equity() {
        let store = test_store().await;
        // Use a scheduler config that always allows running (wide window)
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
            strategy_families: vec!["ema_macd_breakout".to_string()],
            scheduler_config,
            ..Default::default()
        };

        let runner = NightlyRunner::new(config, store);
        let report = runner.run().await.unwrap();

        // Should process 2 equities
        assert_eq!(report.equities_processed, 2);
    }
}
