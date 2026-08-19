//! Options scheduler — daily entry pipeline for options positions
//!
//! Polls for new daily candles, runs the entry pipeline:
//! 1. Macro gate (VIX + calendar)
//! 2. Chain selector (DTE/delta/liquidity)
//! 3. Position sizing
//! 4. Entry executor (2-stage ladder)
//!
//! Runs as a separate tokio task alongside EquityScheduler.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::db::{self, DbPool};
use crate::options::chain_selector::{CandidateChain, ChainSelector, ChainSelectorConfig};
use crate::options::config_store::OptionsConfigStore;
use crate::options::entry_executor::EntryExecutor;
use crate::options::entry_integration::EntryPipeline;
use crate::options::macro_gate::{MacroGate, MacroGateConfig};
use crate::options::sizing::{PositionSizer, SizingConfig};

/// Build runtime configs from the DB-backed options config store.
/// Values missing from the store fall back to the boot-time scheduler config,
/// then to registry defaults inside the store itself.
async fn configs_from_store(
    store: &OptionsConfigStore,
    boot: &OptionsSchedulerConfig,
) -> (MacroGateConfig, ChainSelectorConfig, SizingConfig) {
    macro_rules! f64_or {
        ($key:expr, $default:expr) => {
            store.get_f64($key).await.unwrap_or($default)
        };
    }
    macro_rules! i64_or {
        ($key:expr, $default:expr) => {
            store.get_i64($key).await.unwrap_or($default)
        };
    }

    let macro_gate_config = MacroGateConfig {
        vix_level_threshold: f64_or!("vix_level_gate", boot.macro_gate_config.vix_level_threshold),
        vix_slope_threshold: f64_or!("vix_slope_threshold", boot.macro_gate_config.vix_slope_threshold),
        blackout_hours: f64_or!("blackout_hours", boot.macro_gate_config.blackout_hours),
    };

    let chain_selector_config = ChainSelectorConfig {
        min_dte: i64_or!("dte_min", boot.chain_selector_config.min_dte as i64) as u32,
        max_dte: i64_or!("dte_max", boot.chain_selector_config.max_dte as i64) as u32,
        target_delta: f64_or!("delta_target", boot.chain_selector_config.target_delta),
        max_spread_pct: f64_or!("spread_cap_pct", boot.chain_selector_config.max_spread_pct),
        min_open_interest: i64_or!("oi_min", boot.chain_selector_config.min_open_interest),
    };

    let sizing_config = SizingConfig {
        risk_per_trade: f64_or!("risk_pct", boot.sizing_config.risk_per_trade),
        max_premium_per_position: f64_or!("max_premium_pct", boot.sizing_config.max_premium_per_position),
        max_contracts_per_position: i64_or!("contracts_cap", boot.sizing_config.max_contracts_per_position as i64) as u32,
        max_portfolio_premium: f64_or!("deployed_cap_pct", boot.sizing_config.max_portfolio_premium),
    };

    (macro_gate_config, chain_selector_config, sizing_config)
}

/// Options scheduler configuration
#[derive(Debug, Clone)]
pub struct OptionsSchedulerConfig {
    /// Equities to monitor (e.g., QQQ, SMH, XLF)
    pub equities: Vec<String>,
    /// Poll interval in seconds
    pub poll_interval_secs: u64,
    /// Macro gate configuration
    pub macro_gate_config: MacroGateConfig,
    /// Chain selector configuration
    pub chain_selector_config: ChainSelectorConfig,
    /// Sizing configuration
    pub sizing_config: SizingConfig,
    /// Trading mode — used for event attribution ("paper" | "live")
    pub mode: String,
}

impl Default for OptionsSchedulerConfig {
    fn default() -> Self {
        Self {
            equities: vec!["QQQ".to_string()],
            poll_interval_secs: 300, // 5 minutes
            macro_gate_config: MacroGateConfig::default(),
            chain_selector_config: ChainSelectorConfig::default(),
            sizing_config: SizingConfig::default(),
            mode: "paper".to_string(),
        }
    }
}

/// Options scheduler state
#[derive(Debug, Clone, PartialEq)]
pub enum OptionsSchedulerState {
    /// Idle, waiting for next candle
    Idle,
    /// Processing new candle(s)
    Processing,
    /// Running entry pipeline
    EntryPipeline,
    /// Error state
    Error(String),
}

/// Options scheduler — manages the daily entry pipeline
pub struct OptionsScheduler {
    pool: DbPool,
    config: OptionsSchedulerConfig,
    /// DB-backed config store; runtime configs are rebuilt from it each tick
    /// so UI edits take effect without a restart.
    config_store: OptionsConfigStore,
    state: Arc<RwLock<OptionsSchedulerState>>,
    last_processed_ts: Arc<RwLock<Option<i64>>>,
}

impl OptionsScheduler {
    pub fn new(pool: DbPool, config: OptionsSchedulerConfig) -> Self {
        let mode = config.mode.clone();
        let config_store = OptionsConfigStore::new(pool.clone(), &mode);
        Self {
            pool,
            config,
            config_store,
            state: Arc::new(RwLock::new(OptionsSchedulerState::Idle)),
            last_processed_ts: Arc::new(RwLock::new(None)),
        }
    }

    /// Get current scheduler state
    pub async fn state(&self) -> OptionsSchedulerState {
        self.state.read().await.clone()
    }

    /// Get last processed candle timestamp
    pub async fn last_processed_ts(&self) -> Option<i64> {
        *self.last_processed_ts.read().await
    }

    /// Run the scheduler loop
    pub async fn run(&self) -> Result<()> {
        info!("Options scheduler starting with {} equities", self.config.equities.len());

        loop {
            // Check for new candles
            if let Err(e) = self.tick().await {
                error!(error = %e, "options scheduler tick failed");
                *self.state.write().await = OptionsSchedulerState::Error(e.to_string());
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(
                self.config.poll_interval_secs,
            ))
            .await;
        }
    }

    /// Single tick: check for new candles, run entry pipeline
    pub async fn tick(&self) -> Result<()> {
        *self.state.write().await = OptionsSchedulerState::Processing;

        for equity in &self.config.equities {
            if let Err(e) = self.process_equity(equity).await {
                error!(equity, error = %e, "failed to process equity");
            }
        }

        *self.state.write().await = OptionsSchedulerState::Idle;
        Ok(())
    }

    /// Process a single equity: check for new candle, run entry pipeline
    async fn process_equity(&self, equity: &str) -> Result<()> {
        // Fetch latest candle timestamp
        let latest_ts = db::latest_equity_candle_ts(&self.pool, equity).await?;
        let latest_ts = match latest_ts {
            Some(ts) => ts,
            None => {
                warn!(equity, "no candles found, skipping");
                return Ok(());
            }
        };

        // Check if we've already processed this candle
        let last_processed = *self.last_processed_ts.read().await;
        if last_processed == Some(latest_ts) {
            return Ok(());
        }

        info!(equity, candle_ts = latest_ts, "processing new candle");

        // D13: apply queued promotions at the daily candle boundary,
        // before the entry pipeline runs (mid-exit re-check inside).
        let pipeline = crate::hyperopt::PromotionPipeline::new();
        let cand_store = crate::hyperopt::CandidateStore::new(self.pool.clone());
        match crate::hyperopt::promotion::apply_pending_promotions(
            &self.pool, equity, &pipeline, &cand_store,
        )
        .await
        {
            Ok((applied, skipped)) if applied > 0 || skipped > 0 => {
                info!(equity, applied, skipped, "applied pending promotions at candle boundary");
            }
            Err(e) => error!(equity, error = %e, "failed to apply pending promotions"),
            _ => {}
        }

        // Run entry pipeline
        *self.state.write().await = OptionsSchedulerState::EntryPipeline;

        match self.run_entry_pipeline(equity, latest_ts).await {
            Ok(result) => {
                info!(
                    equity,
                    entry_initiated = result.entry_initiated,
                    "entry pipeline complete"
                );
                *self.last_processed_ts.write().await = Some(latest_ts);
            }
            Err(e) => {
                error!(equity, error = %e, "entry pipeline failed");
                return Err(e);
            }
        }

        Ok(())
    }

    /// Record a SKIPPED_ENTRY event and return the not-entered result.
    async fn skipped_entry(
        &self,
        equity: &str,
        reason: &str,
        detail: serde_json::Value,
    ) -> EntryPipelineResult {
        let payload = serde_json::json!({ "reason": reason, "detail": detail });
        if let Err(e) = db::insert_event(
            &self.pool,
            "strategy",
            "info",
            &self.config.mode,
            "options::entry",
            &format!("SKIPPED_ENTRY {equity}: {reason}"),
            &payload.to_string(),
            Some(equity),
        )
        .await
        {
            error!(equity, error = %e, "failed to record SKIPPED_ENTRY event");
        }
        EntryPipelineResult {
            entry_initiated: false,
            reason: Some(reason.to_string()),
        }
    }

    /// Run the full entry pipeline for an equity
    async fn run_entry_pipeline(&self, equity: &str, candle_ts: i64) -> Result<EntryPipelineResult> {
        // Rebuild runtime configs from the DB-backed store each pipeline run,
        // so Settings-page edits take effect without a restart.
        let (macro_cfg, chain_cfg, sizing_cfg) =
            configs_from_store(&self.config_store, &self.config).await;
        let macro_gate = MacroGate::new(macro_cfg);
        let chain_selector = ChainSelector::new(chain_cfg);
        let position_sizer = PositionSizer::new(sizing_cfg);

        // 1. Macro gate check
        let vix = self.fetch_vix().await?;
        let now = Utc::now();
        let gate_result = macro_gate.evaluate(vix, vix, now, &[]);

        match gate_result {
            crate::options::macro_gate::MacroGateDecision::Denied(reason) => {
                info!(equity, ?reason, "macro gate denied entry");
                return Ok(self
                    .skipped_entry(
                        equity,
                        &format!("Macro gate denied: {reason:?}"),
                        serde_json::json!({ "vix": vix }),
                    )
                    .await);
            }
            crate::options::macro_gate::MacroGateDecision::Allowed => {
                // Continue to chain selection
            }
        }

        // 2. Fetch candidate chains (mock for now — would come from OpenD)
        let candidates = self.fetch_candidate_chains(equity).await?;

        if candidates.is_empty() {
            info!(equity, "no candidate chains available");
            return Ok(self
                .skipped_entry(equity, "No candidate chains", serde_json::json!({}))
                .await);
        }

        // 3. Chain selection
        let chain_result = chain_selector.select(&candidates);
        let selected_chain = match chain_result {
            crate::options::chain_selector::ChainSelectionResult::Selected(chain) => chain,
            crate::options::chain_selector::ChainSelectionResult::Skipped(reason) => {
                info!(equity, ?reason, "chain selection skipped");
                return Ok(self
                    .skipped_entry(
                        equity,
                        &format!("Chain selection skipped: {reason:?}"),
                        serde_json::json!({}),
                    )
                    .await);
            }
        };

        // 4. Position sizing
        let account_equity = self.fetch_account_equity().await?;
        let sizing_result = position_sizer.size(
            account_equity,
            0.0, // stop_distance — would come from strategy
            selected_chain.delta,
            selected_chain.ask,
            0.0, // current_portfolio_premium
        );

        let sizing_decision = match sizing_result {
            crate::options::sizing::SizingResult::Sized(decision) => decision,
            crate::options::sizing::SizingResult::Skipped(reason) => {
                info!(equity, ?reason, "sizing denied");
                return Ok(self
                    .skipped_entry(
                        equity,
                        &format!("Sizing denied: {reason:?}"),
                        serde_json::json!({ "account_equity": account_equity }),
                    )
                    .await);
            }
        };

        // 5. Entry executor (2-stage ladder)
        let mut executor = EntryExecutor::new(
            0, // position_id — would be assigned after fill
            selected_chain.ask,
            0.01, // slippage_budget
        );

        let now = Utc::now();
        if executor.should_advance(now) {
            executor.advance();
        }

        // In production: submit order to broker, wait for fill, advance to stage 2
        // For now: mark as filled (mock)
        executor.mark_filled();

        info!(
            equity,
            contracts = sizing_decision.contracts,
            premium = selected_chain.ask * sizing_decision.contracts as f64 * 100.0,
            "entry initiated"
        );

        Ok(EntryPipelineResult {
            entry_initiated: true,
            reason: None,
        })
    }

    /// Fetch VIX close (mock — would come from DB)
    async fn fetch_vix(&self) -> Result<f64> {
        // Mock: return a default VIX value
        Ok(20.0)
    }

    /// Fetch candidate chains (mock — would come from OpenD)
    async fn fetch_candidate_chains(&self, equity: &str) -> Result<Vec<CandidateChain>> {
        // Mock: return empty list (no real chains available)
        Ok(vec![])
    }

    /// Fetch account equity (mock — would come from broker)
    async fn fetch_account_equity(&self) -> Result<f64> {
        // Mock: return default equity
        Ok(100_000.0)
    }
}

/// Entry pipeline result
#[derive(Debug, Clone)]
pub struct EntryPipelineResult {
    pub entry_initiated: bool,
    pub reason: Option<String>,
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

        // Create minimal schema
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS equity_candles (
                symbol TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume REAL NOT NULL,
                PRIMARY KEY (symbol, timestamp)
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS engine_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                category TEXT NOT NULL, severity TEXT NOT NULL, mode TEXT NOT NULL,
                source TEXT NOT NULL, message TEXT NOT NULL,
                payload_json TEXT NOT NULL DEFAULT '{}', equity TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[tokio::test]
    async fn test_scheduler_initial_state() {
        let pool = test_pool().await;
        let config = OptionsSchedulerConfig::default();
        let scheduler = OptionsScheduler::new(pool, config);

        assert_eq!(scheduler.state().await, OptionsSchedulerState::Idle);
        assert_eq!(scheduler.last_processed_ts().await, None);
    }

    #[tokio::test]
    async fn test_scheduler_tick_no_candles() {
        let pool = test_pool().await;
        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool, config);

        // Tick should succeed even with no candles
        let result = scheduler.tick().await;
        assert!(result.is_ok());
        assert_eq!(scheduler.state().await, OptionsSchedulerState::Idle);
    }

    #[tokio::test]
    async fn test_scheduler_multi_equity() {
        let pool = test_pool().await;
        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string(), "SMH".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool, config);

        // Tick should process both equities
        let result = scheduler.tick().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_entry_pipeline_macro_gate_denied() {
        let pool = test_pool().await;
        let check_pool = pool.clone();
        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            macro_gate_config: MacroGateConfig {
                vix_level_threshold: 15.0, // Block if VIX > 15
                ..Default::default()
            },
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool, config);

        // Mock VIX = 20.0 > 15.0 → gate should deny
        let result = scheduler.run_entry_pipeline("QQQ", 0).await.unwrap();
        assert!(!result.entry_initiated);
        assert!(result.reason.unwrap().contains("Macro gate denied"));

        // SKIPPED_ENTRY event must be recorded (per item 5)
        let events = db::search_events(&check_pool, Some("strategy"), None, None, Some("QQQ"), None, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1, "expected exactly one SKIPPED_ENTRY event");
        assert!(events[0].message.contains("SKIPPED_ENTRY QQQ"));
        assert!(events[0].message.contains("Macro gate denied"));
    }

    #[tokio::test]
    async fn test_entry_pipeline_no_chains() {
        let pool = test_pool().await;
        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool, config);

        // Mock: no candidate chains available
        let result = scheduler.run_entry_pipeline("QQQ", 0).await.unwrap();
        assert!(!result.entry_initiated);
        assert!(result.reason.unwrap().contains("No candidate chains"));
    }
}
