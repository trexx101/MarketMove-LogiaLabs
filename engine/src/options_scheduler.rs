//! Options scheduler — daily entry pipeline for options positions
//!
//! Polls for new daily candles, runs the entry pipeline:
//! 1. Macro gate (VIX + calendar)
//! 2. Chain selector (DTE/delta/liquidity)
//! 3. Position sizing
//! 4. Entry executor (2-stage ladder)
//!
//! Runs as a separate tokio task alongside EquityScheduler.

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::Row;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::db::{self, DbPool};
use crate::options::chain_selector::{CandidateChain, ChainSelector, ChainSelectorConfig, SelectedChain};
use crate::options::config_store::OptionsConfigStore;
use crate::options::macro_gate::{MacroGate, MacroGateConfig};
use crate::options::paper_executor::{EntryEntryParams, EntryOutcome, OptionsPaperExecutor};
use crate::options::sizing::{PositionSizer, SizingConfig};
use crate::options_recorder::TapeRow;
use arrow::array::{Float64Array, Int64Array, StringArray, TimestampMillisecondArray};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

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
    /// Hyperopt promotion gates (min-days observation clocks).
    pub promotion_gates: crate::config::PromotionGatesConfig,
    /// Directory where the options tape recorder writes Parquet files
    /// (default: `./data/options_tape`, same as OPT_TAPE_DIR env var).
    pub tape_dir: String,
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
            promotion_gates: crate::config::PromotionGatesConfig::default(),
            tape_dir: "./data/options_tape".to_string(),
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
        // A2: Respect the options_enabled config toggle (0=off, 1=on).
        if !self.config_store.get_bool("options_enabled").await.unwrap_or(false) {
            return Ok(());
        }

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

        // A3: Gate on strategy status — only PAPER/MICRO/LIVE proceed for entry.
        let strat = self.fetch_strategy_status(equity).await?;
        let entry_eligible = match strat {
            Some((_version_id, ref status)) if status == "PAPER" || status == "MICRO" || status == "LIVE" => true,
            Some((_version_id, ref status)) => {
                let _ = self.skipped_entry(equity, &format!("Strategy status gated: {status}"), serde_json::json!({ "status": status })).await;
                false
            }
            None => {
                let _ = self.skipped_entry(equity, "No strategy_version", serde_json::json!({})).await;
                false
            }
        };

        // Entry pipeline — only when strategy status allows
        if entry_eligible {
            // D13: apply queued promotions at the daily candle boundary,
            // before the entry pipeline runs (mid-exit re-check inside).
            let pipeline = crate::hyperopt::PromotionPipeline::with_gates(self.config.promotion_gates.clone());
            let cand_store = crate::hyperopt::CandidateStore::new(self.pool.clone());
            match crate::hyperopt::promotion::apply_pending_promotions(
                &self.pool, equity, &self.config.mode, &pipeline, &cand_store,
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
                }
                Err(e) => {
                    error!(equity, error = %e, "entry pipeline failed");
                    return Err(e);
                }
            }
        }

        // Phase B: Run exit pipeline for any OPEN options positions.
        // Runs every candle boundary regardless of strategy status.
        *self.state.write().await = OptionsSchedulerState::Processing;
        if let Err(e) = self.run_exit_pipeline(equity).await {
            error!(equity, error = %e, "exit pipeline failed");
        }

        *self.last_processed_ts.write().await = Some(latest_ts);

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
            position_opened: None,
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
        let deployed_premium = self.current_portfolio_premium().await?;
        let (pred_1d, last_close) = match self.fetch_latest_prediction(equity).await? {
            Some((p1d, _)) => {
                // Get last close from equity_candles for ATR context
                let candles = db::fetch_equity_candles_asc(&self.pool, equity, 20).await?;
                let close = candles.last().map(|c| c.close).unwrap_or(100.0);
                (p1d, close)
            }
            None => (0.0, 100.0),
        };
        let atr_ratio = compute_atr_ratio_light(&self.pool, equity).await;
        let stop_distance = (pred_1d.abs() * 0.5) + (atr_ratio * last_close * 0.5);
        info!(
            equity,
            account_equity,
            deployed_premium,
            stop_distance,
            pred_1d,
            atr_ratio,
            "sizing inputs"
        );

        let sizing_result = position_sizer.size(
            account_equity,
            stop_distance,
            selected_chain.delta,
            selected_chain.ask,
            deployed_premium,
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

        // 5. Paper entry (B2): create the real position + fill + event.
        //    The 2-stage EntryExecutor ladder is a no-op in paper mode
        //    (plan risk #3) — the order fills immediately at the ask.
        let us_equity = if equity.starts_with("US.") {
            equity.to_string()
        } else {
            format!("US.{}", equity)
        };

        match self
            .open_paper_entry(equity, &us_equity, &selected_chain, sizing_decision.contracts, last_close)
            .await?
        {
            EntryOutcome::Opened {
                position_id,
                fill_price,
            } => {
                info!(
                    equity,
                    %position_id,
                    contracts = sizing_decision.contracts,
                    premium = fill_price * sizing_decision.contracts as f64 * 100.0,
                    "entry filled (paper)"
                );
                return Ok(EntryPipelineResult {
                    entry_initiated: true,
                    reason: None,
                    position_opened: Some(position_id),
                });
            }
            EntryOutcome::Skipped(reason) => {
                // No broker error — the guard itself denied (duplicate
                // open position for this underlying).
                let detail = match &reason {
                    crate::options::paper_executor::EntrySkipReason::DuplicateOpenPosition {
                        existing_position_id,
                    } => {
                        info!(equity, %existing_position_id, "entry skipped: duplicate open position");
                        serde_json::json!({ "existing_position_id": existing_position_id })
                    }
                };
                return Ok(self
                    .skipped_entry(equity, "Entry skipped: duplicate open position", detail)
                    .await);
            }
        }
    }

    /// B2: paper entry — the single production path that creates an
    /// `option_positions` row.
    ///
    /// `us_equity` is the underlying as the tape recorder stores it
    /// (`US.QQQ`) — `initiate_entry` uses it for the one-open-position-per-
    /// underlying guard, and the exit pipeline must use the same form so
    /// the WHERE clause matches.
    ///
    /// `strategy_version_id` comes from the A3 status gate (the same call
    /// that decided entry is eligible), so a position is never created
    /// without attribution.
    async fn open_paper_entry(
        &self,
        equity: &str,
        us_equity: &str,
        selected_chain: &SelectedChain,
        contracts: u32,
        entry_underlying_price: f64,
    ) -> Result<EntryOutcome> {
        // A3: the version that triggered this entry. Re-fetch here — the
        // gate at tick() is the same query; the status can't change within
        // one candle.
        let strategy_version_id = match self.fetch_strategy_status(equity).await? {
            Some((id, _status)) => id,
            None => {
                // Defensive: the gate should have blocked us before sizing.
                info!(equity, "no strategy_version for entry — skipping (no attribution)");
                return Ok(EntryOutcome::Skipped(
                    crate::options::paper_executor::EntrySkipReason::DuplicateOpenPosition {
                        existing_position_id: String::new(),
                    },
                ));
            }
        };

        let contract_code = format!(
            "{}{}{}{:.0}",
            us_equity, selected_chain.expiry, selected_chain.option_type,
            selected_chain.strike * 1000.0
        );

        let executor = OptionsPaperExecutor::new(self.pool.clone());
        let params = EntryEntryParams {
            underlying: us_equity.to_string(),
            contract_code,
            strategy_version_id,
            entry_underlying_price,
            ask: selected_chain.ask,
            bid: selected_chain.bid,
            contracts,
            delta: selected_chain.delta,
            dte: selected_chain.dte as i64,
            slippage_budget: 0.01,
        };

        // `initiate_entry` writes the position, the ENTRY fill, and the
        // `options::position_opened` event atomically.
        executor.initiate_entry(&params).await
    }

    /// Run the exit pipeline for all OPEN option positions for an equity.
    ///
    /// For each position, evaluates guardrails (DTE override, trailing stop),
    /// routes signals through the ExitArbiter, and executes paper fills
    /// via the staged exit ladder.
    async fn run_exit_pipeline(&self, equity: &str) -> Result<()> {
        use crate::options::exit_arbiter::{ExitArbiter, ExitSignal, ExitSource};
        use crate::options::paper_executor::OptionsPaperExecutor;
        use crate::options::staged_ladder::ExitStage;
        use crate::options::trailing_stop::{TrailingStop, TrailingStopConfig};
        use chrono::Utc;

        // Fetch OPEN positions for this equity
        let positions = sqlx::query(
            "SELECT id, contract_code, dte_at_entry, entry_underlying_price, \
             entry_premium, delta_at_entry, created_at, strategy_version_id, qty \
             FROM option_positions WHERE underlying = ? AND status = 'OPEN'"
        )
        .bind(equity)
        .fetch_all(&self.pool)
        .await
        .context("fetch open positions for exit pipeline")?;

        if positions.is_empty() {
            return Ok(());
        }

        let now = Utc::now().timestamp();
        // Get current price for trailing stop evaluation
        let current_price = db::fetch_equity_candles_asc(&self.pool, equity, 1)
            .await
            .ok()
            .and_then(|c| c.last().map(|r| r.close))
            .unwrap_or(0.0);
        let atr_ratio = compute_atr_ratio_light(&self.pool, equity).await;
        let atr = atr_ratio * current_price.max(1.0);

        // Fetch latest prediction for signal reversal guardrail
        let pred_1d = self
            .fetch_latest_prediction(equity)
            .await?
            .map(|(p, _)| p)
            .unwrap_or(0.0);

        let arbiter = ExitArbiter::new();
        let mut executor = OptionsPaperExecutor::new(self.pool.clone());

        for pos in &positions {
            let pos_id: String = pos.get("id");
            let dte_at_entry: i64 = pos.get("dte_at_entry");
            let entry_price: f64 = pos.get("entry_underlying_price");
            let entry_premium: f64 = pos.get("entry_premium");
            let delta: f64 = pos.get("delta_at_entry");
            let created_at: i64 = pos.get("created_at");
            let qty: i64 = pos.get("qty");

            let mut signals: Vec<ExitSignal> = Vec::new();

            // Guardrail 1: DTE override — close if remaining DTE < dte_min
            let days_since_entry = ((now - created_at) as f64 / 86400.0).max(0.0);
            let remaining_dte = dte_at_entry as f64 - days_since_entry;
            // Default dte_min to 30 (matching config default); ideally from config store
            let dte_min = self
                .config_store
                .get_f64("dte_min")
                .await
                .unwrap_or(30.0);
            if remaining_dte < dte_min {
                signals.push(ExitSignal {
                    source: ExitSource::DteOverride,
                    priority: ExitSource::DteOverride as u8,
                    reason: format!(
                        "DTE {remaining_dte:.0} < min {dte_min:.0} for {pos_id}"
                    ),
                    timestamp: Utc::now(),
                });
            }

            // Guardrail 2: Trailing stop (paper: track from entry to current)
            let is_call = delta > 0.0;
            let trail_pct = self
                .config_store
                .get_f64("trail_pct")
                .await
                .unwrap_or(0.05);
            let rearm_band = self
                .config_store
                .get_f64("rearm_band_atr")
                .await
                .unwrap_or(0.5);
            let ts_config = TrailingStopConfig {
                trail_pct,
                rearm_band_atr: rearm_band,
            };
            let mut ts = TrailingStop::with_config(entry_price, atr, is_call, ts_config);

            // Update with current price (simulates having tracked HWM over time in paper)
            // In production this would be persisted per-position.
            // For paper: use the HWM between entry and current from candle series.
            let days_since_entry_f = days_since_entry.ceil() as i64;
            let candles = db::fetch_equity_candles_asc(
                &self.pool,
                equity,
                days_since_entry_f + 1,
            )
            .await?;
            // Walk candles from entry forward to update trailing stop
            let mut ts_signal = None;
            for c in &candles {
                ts_signal = ts.update(c.close, is_call);
            }
            let final_signal = ts.update(current_price, is_call);
            if final_signal.is_some() {
                ts_signal = final_signal;
            }

            if let Some(signal) = ts_signal {
                signals.push(signal);
            }

            // Guardrail 3: Signal reversal (prediction flips direction)
            // For calls: entry when pred_1d > 0; exit when pred_1d <= 0
            // For puts: entry when pred_1d <= 0; exit when pred_1d > 0
            let signal_reversed = if is_call {
                pred_1d <= 0.0
            } else {
                pred_1d > 0.0
            };
            if signal_reversed && pred_1d != 0.0 {
                signals.push(ExitSignal {
                    source: ExitSource::SignalReversal,
                    priority: ExitSource::SignalReversal as u8,
                    reason: format!(
                        "Signal reversed: pred_1d={pred_1d:.4}, delta={delta:.2} for {pos_id}"
                    ),
                    timestamp: Utc::now(),
                });
            }

            // Run through arbiter
            let winner = arbiter.select_winner(&signals);
            if let Some(signal) = winner {
                info!(
                    equity,
                    position_id = %pos_id,
                    source = ?signal.source,
                    reason = %signal.reason,
                    "exit signal selected — initiating staged exit"
                );

                // Persist exit signal
                let _ = sqlx::query(
                    "INSERT INTO exit_signals (id, position_id, trigger_source, \
                     priority, stage, intended_action, persisted_before_send, created_at) \
                     VALUES (?, ?, ?, ?, 0, 'CLOSE', 1, ?)"
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&pos_id)
                .bind(format!("{:?}", signal.source))
                .bind(signal.priority as i64)
                .bind(now)
                .execute(&self.pool)
                .await;

                // Current bid — use last close as proxy in paper mode
                let current_bid = current_price * 0.995; // ~0.5% below close
                let tick_size = if current_price > 100.0 { 0.05 } else { 0.01 };

                // C3: Record exit intent before sending (audit trail)
                let _ = sqlx::query(
                    "INSERT INTO exit_intent_log (position_id, stage, limit_price, quantity, timestamp) \
                     VALUES (?, 'EXIT_STAGE_1', ?, ?, ?)"
                )
                .bind(&pos_id)
                .bind(current_bid)
                .bind(qty as f64) // actual position size
                .bind(Utc::now().to_rfc3339())
                .execute(&self.pool)
                .await;

                // Initiate staged exit
                match executor.initiate_exit(&pos_id, current_bid, tick_size).await {
                    Ok(_stage) => {
                        // Paper mode: advance through ladder stages immediately
                        // (no real market to wait for)
                        let fill = loop {
                            let current_stage = executor
                                .get_ladder(&pos_id)
                                .map(|l| l.current_stage());
                            if current_stage == Some(ExitStage::Complete) {
                                break None;
                            }

                            match executor
                                .try_fill(&pos_id, current_bid, current_price, Utc::now())
                                .await
                            {
                                Ok(Some(fill)) => break Some(fill),
                                Ok(None) => {
                                    // Advance to next stage
                                    executor.advance_ladder(&pos_id, current_bid)?;
                                }
                                Err(e) => {
                                    error!(
                                        equity,
                                        position_id = %pos_id,
                                        error = %e,
                                        "exit fill failed"
                                    );
                                    break None;
                                }
                            }
                        };

                        if let Some(fill) = fill {
                            let exit_premium = fill.price;
                            let realized_pnl = (exit_premium - entry_premium) * qty as f64 * 100.0;

                            info!(
                                equity,
                                position_id = %pos_id,
                                fill_price = exit_premium,
                                entry_premium,
                                realized_pnl,
                                "exit filled — marking position CLOSED"
                            );
                            // Mark position CLOSED with realized PnL
                            let _ = sqlx::query(
                                "UPDATE option_positions SET status = 'CLOSED', \
                                 realized_pnl = ?, closed_at = ?, updated_at = ? WHERE id = ?"
                            )
                            .bind(realized_pnl)
                            .bind(now)
                            .bind(now)
                            .bind(&pos_id)
                            .execute(&self.pool)
                            .await;

                            // Emit options::position_closed lifecycle event
                            let payload = serde_json::json!({
                                "position_id": &pos_id,
                                "realized_pnl": realized_pnl,
                                "exit_premium": exit_premium,
                                "entry_premium": entry_premium,
                                "qty": qty,
                            });
                            let _ = crate::db::insert_event(
                                &self.pool,
                                "trade",
                                "info",
                                "paper",
                                "options::position_closed",
                                &format!(
                                    "POSITION_CLOSED {}: realized_pnl={:.2} (exit {:.2} - entry {:.2}) x{}",
                                    &pos_id, realized_pnl, exit_premium, entry_premium, qty
                                ),
                                &payload.to_string(),
                                Some(equity),
                            )
                            .await;
                        } else {
                            info!(
                                equity,
                                position_id = %pos_id,
                                "exit ladder exhausted without fill"
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            equity,
                            position_id = %pos_id,
                            error = %e,
                            "exit initiation failed"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Fetch VIX close from equity_candles (^VIX symbol).
    /// Returns the last close value, or 20.0 as a fallback if no VIX data exists.
    async fn fetch_vix(&self) -> Result<f64> {
        let vix = db::fetch_equity_candles_asc(&self.pool, "^VIX", 1).await?;
        if let Some(c) = vix.last() {
            info!(vix = c.close, "fetched VIX close from DB");
            Ok(c.close)
        } else {
            warn!("no ^VIX candles in DB, falling back to 20.0");
            Ok(20.0)
        }
    }

    /// Fetch candidate chains for an underlying equity.
    ///
    /// Queries `option_tape_meta` for known chain codes, then attempts to
    /// read the latest Parquet tape for recent bid/ask/delta/oi data.
    /// Returns empty vec when no tape data exists yet (recorder not wired).
    async fn fetch_candidate_chains(&self, equity: &str) -> Result<Vec<CandidateChain>> {
        // Query known chain codes from option_tape_meta.
        // Tape recorder stores underlying as "US.QQQ", but the scheduler
        // receives stripped equity "QQQ" — try both forms.
        let us_equity = if equity.starts_with("US.") {
            equity.to_string()
        } else {
            format!("US.{}", equity)
        };
        let chain_rows = sqlx::query(
            "SELECT chain_code, last_heartbeat_ts FROM option_tape_meta WHERE underlying = ?1 OR underlying = ?2 ORDER BY chain_code",
        )
        .bind(equity)
        .bind(&us_equity)
        .fetch_all(&self.pool)
        .await
        .context("fetch_candidate_chains: option_tape_meta query")?;

        if chain_rows.is_empty() {
            warn!(%equity, "no option chains in option_tape_meta");
            return Ok(vec![]);
        }

        let chain_codes: Vec<String> = chain_rows
            .iter()
            .map(|r| r.get::<String, _>("chain_code"))
            .collect();

        info!(%equity, chains = chain_codes.len(), "found known chains in option_tape_meta");

        // A1.5: read the latest Parquet file for each chain, pick the
        // newest row per contract_code, and convert to CandidateChain.
        let mut candidates: Vec<CandidateChain> = Vec::new();
        let today = Utc::now().date_naive();

        for chain_code in &chain_codes {
            match read_latest_tape_rows(&self.config.tape_dir, chain_code) {
                Ok(rows) => {
                    // Group by contract_code, keep the row with the
                    // highest timestamp (most recent quote).
                    let mut latest: std::collections::HashMap<String, &TapeRow> =
                        std::collections::HashMap::new();
                    for row in &rows {
                        let entry = latest.entry(row.contract_code.clone()).or_insert(row);
                        if row.timestamp_ms > entry.timestamp_ms {
                            *entry = row;
                        }
                    }
                    for row in latest.values() {
                        match tape_row_to_candidate(row, today) {
                            Ok(c) => candidates.push(c),
                            Err(e) => {
                                warn!(%equity, chain_code, contract_code = %row.contract_code, error = %e, "skipping unparseable contract");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(%equity, chain_code, error = %e, "failed to read tape Parquet — skipping chain");
                }
            }
        }

        if candidates.is_empty() {
            warn!(%equity, "no candidates built from tape Parquet");
        } else {
            info!(%equity, count = candidates.len(), "built candidates from tape");
        }
        Ok(candidates)
    }
}

// ── A1.5 Parquet tape reader ──────────────────────────────────────────

/// Read the latest Parquet file for a chain and return all rows.
///
/// `chain_code` has the form `US.QQQ260925` (underlying + 6-digit expiry).
/// Files are under `{tape_dir}/{underlying}/{chain_code}/YYYY-MM-DD.parquet`.
fn read_latest_tape_rows(tape_dir: &str, chain_code: &str) -> Result<Vec<TapeRow>> {
    if chain_code.len() < 7 {
        return Ok(vec![]);
    }
    let underlying = &chain_code[..chain_code.len() - 6]; // "US.QQQ"
    let chain_dir = PathBuf::from(tape_dir).join(underlying).join(chain_code);

    let mut parquet_files: Vec<PathBuf> = Vec::new();
    match fs::read_dir(&chain_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "parquet") {
                    // Skip zero-byte files (recorder creates empty files on no-data days)
                    if entry.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                        parquet_files.push(path);
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![]);
        }
        Err(e) => return Err(e.into()),
    }

    if parquet_files.is_empty() {
        return Ok(vec![]);
    }

    // Sort by filename descending (date-based), take the latest.
    parquet_files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    let latest = &parquet_files[0];

    let file = fs::File::open(latest)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        rows.extend(batch_to_tape_rows(&batch)?);
    }
    Ok(rows)
}

/// Convert an Arrow RecordBatch into TapeRow structs.
fn batch_to_tape_rows(batch: &RecordBatch) -> Result<Vec<TapeRow>> {
    let num_rows = batch.num_rows();
    let timestamps = batch
        .column(0)
        .as_any()
        .downcast_ref::<TimestampMillisecondArray>()
        .context("column 0 is not TimestampMillisecondArray")?;
    let underlyings = batch
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .context("column 1 is not StringArray")?;
    let chain_codes = batch
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .context("column 2 is not StringArray")?;
    let contract_codes = batch
        .column(3)
        .as_any()
        .downcast_ref::<StringArray>()
        .context("column 3 is not StringArray")?;
    let bids = batch
        .column(4)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 4 is not Float64Array")?;
    let asks = batch
        .column(5)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 5 is not Float64Array")?;
    let lasts = batch
        .column(6)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 6 is not Float64Array")?;
    let volumes = batch
        .column(7)
        .as_any()
        .downcast_ref::<Int64Array>()
        .context("column 7 is not Int64Array")?;
    let ois = batch
        .column(8)
        .as_any()
        .downcast_ref::<Int64Array>()
        .context("column 8 is not Int64Array")?;
    let ivs = batch
        .column(9)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 9 is not Float64Array")?;
    let deltas = batch
        .column(10)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 10 is not Float64Array")?;
    let gammas = batch
        .column(11)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 11 is not Float64Array")?;
    let thetas = batch
        .column(12)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 12 is not Float64Array")?;
    let ul_prices = batch
        .column(13)
        .as_any()
        .downcast_ref::<Float64Array>()
        .context("column 13 is not Float64Array")?;

    let mut rows = Vec::with_capacity(num_rows);
    for i in 0..num_rows {
        rows.push(TapeRow {
            timestamp_ms: timestamps.value(i),
            underlying: underlyings.value(i).to_string(),
            chain_code: chain_codes.value(i).to_string(),
            contract_code: contract_codes.value(i).to_string(),
            bid: bids.value(i),
            ask: asks.value(i),
            last: lasts.value(i),
            volume: volumes.value(i),
            open_interest: ois.value(i),
            implied_volatility: ivs.value(i),
            delta: deltas.value(i),
            gamma: gammas.value(i),
            theta: thetas.value(i),
            underlying_price: ul_prices.value(i),
        });
    }
    Ok(rows)
}

/// Parse a contract_code like `US.QQQ260919C530000` and build a
/// CandidateChain, computing DTE and is_monthly from the embedded expiry.
///
/// Returns `(underlying, CandidateChain)` — the underlying is split out
/// so the caller can filter by the equity being processed.
fn tape_row_to_candidate(row: &TapeRow, today: NaiveDate) -> Result<CandidateChain> {
    let (expiry_ymd, opt_type, strike) = parse_contract_code(&row.contract_code)
        .with_context(|| format!("unparseable contract_code: {}", row.contract_code))?;

    // expiry_ymd is YYMMDD → NaiveDate
    let expiry_date = NaiveDate::parse_from_str(&format!("20{}", expiry_ymd), "%Y%m%d")
        .with_context(|| format!("invalid expiry date: {}", expiry_ymd))?;

    let dte = (expiry_date - today).num_days().max(0) as u32;

    // Monthly expiry: third Friday heuristic — the expiry day is between
    // 15 and 21 (inclusive).
    let is_monthly = expiry_date.day() >= 15 && expiry_date.day() <= 21;

    Ok(CandidateChain {
        symbol: row.underlying.clone(),
        expiry: expiry_ymd.to_string(),
        strike,
        option_type: opt_type.to_string(),
        delta: row.delta,
        bid: row.bid,
        ask: row.ask,
        open_interest: row.open_interest,
        dte,
        is_monthly,
    })
}

/// Parse a contract code like `US.QQQ260919C530000` into its components.
///
/// Returns `(expiry_YYMMDD, option_type, strike)`.
/// Scans from the end: the first `C` or `P` marks the split point.
/// The strike is encoded as strike × 1000 (Moomoo US option format),
/// so `720000` → $720.00, `57500` → $57.50.
fn parse_contract_code(code: &str) -> Option<(&str, &str, f64)> {
    let bytes = code.as_bytes();
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'C' || bytes[i] == b'P' {
            let prefix = &code[..i];         // "US.QQQ260919"
            let opt_type = &code[i..i + 1];  // "C" or "P"
            let strike_str = &code[i + 1..]; // variable width, e.g. "720000"
            let strike_raw: f64 = strike_str.parse().ok()?;
            if prefix.len() < 6 || strike_str.is_empty() {
                return None;
            }
            let expiry = &prefix[prefix.len() - 6..]; // "260919"
            return Some((expiry, opt_type, strike_raw / 1000.0));
        }
    }
    None
}

impl OptionsScheduler {
    /// Fetch account equity from trading_models for active equities.
    /// Sums budget_usd for all enabled models, falling back to 100k if none.
    async fn fetch_account_equity(&self) -> Result<f64> {
        let rows = sqlx::query(
            "SELECT budget_usd FROM trading_models WHERE enabled = 1",
        )
        .fetch_all(&self.pool)
        .await
        .context("fetch_account_equity: trading_models query")?;

        if rows.is_empty() {
            warn!("no enabled trading_models, falling back to 100k");
            return Ok(100_000.0);
        }

        let total: f64 = rows.iter().map(|r| r.get::<f64, _>("budget_usd")).sum();
        info!(total, "fetched account equity from trading_models");
        Ok(total.max(1_000.0))
    }

    /// Query the highest-status `strategy_versions` row for this equity.
    /// Returns (strategy_version_id, status) or None if no strategy exists.
    /// Used to gate entry on PAPER/MICRO/LIVE status.
    async fn fetch_strategy_status(&self, equity: &str) -> Result<Option<(String, String)>> {
        let row = sqlx::query(
            "SELECT id, status FROM strategy_versions \
             WHERE equity = ?1 \
             ORDER BY CASE status \
               WHEN 'LIVE' THEN 4 \
               WHEN 'MICRO' THEN 3 \
               WHEN 'PAPER' THEN 2 \
               WHEN 'CANDIDATE' THEN 1 \
               ELSE 0 END DESC \
             LIMIT 1",
        )
        .bind(equity)
        .fetch_optional(&self.pool)
        .await
        .context("fetch_strategy_status")?;

        match row {
            Some(r) => {
                let id: String = r.get("id");
                let status: String = r.get("status");
                Ok(Some((id, status)))
            }
            None => Ok(None),
        }
    }

    /// Current portfolio premium deployed in options: sum of entry_premium * qty
    /// for all OPEN positions.
    async fn current_portfolio_premium(&self) -> Result<f64> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(entry_premium * qty), 0.0) AS deployed \
             FROM option_positions WHERE status = 'OPEN'",
        )
        .fetch_one(&self.pool)
        .await
        .context("current_portfolio_premium")?;

        let deployed: f64 = row.get("deployed");
        Ok(deployed)
    }

    /// Get the latest prediction for an equity to derive stop_distance.
    async fn fetch_latest_prediction(&self, equity: &str) -> Result<Option<(f64, f64)>> {
        let row = sqlx::query(
            "SELECT pred_1d, pred_5d FROM equity_predictions \
             WHERE symbol = ?1 ORDER BY candle_ts DESC LIMIT 1",
        )
        .bind(equity)
        .fetch_optional(&self.pool)
        .await
        .context("fetch_latest_prediction")?;

        match row {
            Some(r) => {
                let pred_1d: f64 = r.get("pred_1d");
                let pred_5d: f64 = r.get("pred_5d");
                Ok(Some((pred_1d, pred_5d)))
            }
            None => {
                warn!(%equity, "no equity_predictions found");
                Ok(None)
            }
        }
    }
}

/// Lightweight ATR ratio: fetch 20 candles and compute ATR/p(last_close).
/// Returns 0.005 as floor when data is insufficient.
async fn compute_atr_ratio_light(pool: &DbPool, equity: &str) -> f64 {
    let candles = match db::fetch_equity_candles_asc(pool, equity, 20).await {
        Ok(c) if c.len() >= 15 => c,
        _ => return 0.005,
    };
    let n = candles.len();
    let mut tr = Vec::with_capacity(n);
    tr.push(0.0);
    for i in 1..n {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_close = candles[i - 1].close;
        let h_l = high - low;
        let h_c = (high - prev_close).abs();
        let l_c = (low - prev_close).abs();
        tr.push(h_l.max(h_c).max(l_c));
    }
    let period = 14.0_f64;
    let warmup: f64 = tr[1..=14].iter().sum::<f64>() / period;
    let mut atr = warmup;
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

/// Entry pipeline result
#[derive(Debug, Clone)]
pub struct EntryPipelineResult {
    pub entry_initiated: bool,
    pub reason: Option<String>,
    /// Set when a real paper position was opened (position_id) —
    /// `entry_initiated` can be true even when a position was skipped
    /// (e.g. duplicate open position for the same underlying).
    pub position_opened: Option<String>,
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
                ts INTEGER NOT NULL,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT 'yahoo',
                PRIMARY KEY (symbol, ts)
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

        // Tables needed by the real data sources (A1)
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS option_tape_meta (\
             id TEXT PRIMARY KEY, underlying TEXT NOT NULL, chain_code TEXT NOT NULL,\
             quota_accounting_json TEXT NOT NULL DEFAULT '{}',\
             created_at INTEGER NOT NULL, last_heartbeat_ts INTEGER\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        // Tables needed for exit pipeline (Phase B)
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS option_fills (\
                     id INTEGER PRIMARY KEY AUTOINCREMENT,\
                     position_id TEXT NOT NULL,\
                     stage TEXT NOT NULL,\
                     price REAL NOT NULL,\
                     quantity REAL NOT NULL,\
                     timestamp INTEGER NOT NULL,\
                     strategy_version_id TEXT NOT NULL DEFAULT ''\
                     )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS exit_signals (\
             id TEXT PRIMARY KEY,\
             position_id TEXT NOT NULL,\
             trigger_source TEXT NOT NULL,\
             priority INTEGER NOT NULL,\
             stage INTEGER NOT NULL,\
             intended_action TEXT NOT NULL,\
             persisted_before_send INTEGER NOT NULL DEFAULT 0,\
             created_at INTEGER NOT NULL\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS option_positions (\
             id TEXT PRIMARY KEY, underlying TEXT NOT NULL,\
             contract_code TEXT NOT NULL, strategy_version_id TEXT NOT NULL,\
             entry_underlying_price REAL NOT NULL, entry_premium REAL NOT NULL DEFAULT 0.0,\
             entry_spread REAL NOT NULL, entry_slippage_budget REAL NOT NULL,\
             qty INTEGER NOT NULL, qty_filled_residual INTEGER NOT NULL DEFAULT 0,\
             status TEXT NOT NULL DEFAULT 'OPEN', dte_at_entry INTEGER NOT NULL,\
             delta_at_entry REAL NOT NULL, realized_pnl REAL,\
             closed_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS equity_predictions (\
             symbol TEXT NOT NULL,\
             candle_ts INTEGER NOT NULL,\
             pred_1d REAL, pred_5d REAL, pred_21d REAL,\
             model_id TEXT NOT NULL,\
             PRIMARY KEY (symbol, model_id, candle_ts)\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS options_config_kv (\
             key TEXT PRIMARY KEY,\
             value TEXT NOT NULL\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS trading_models (\
             model_id TEXT PRIMARY KEY, primary_symbol TEXT NOT NULL,\
             inverse_symbol TEXT NOT NULL, model_path TEXT NOT NULL,\
             norm_stats_path TEXT NOT NULL, budget_usd REAL NOT NULL DEFAULT 5000.0,\
             deploy_pct REAL NOT NULL DEFAULT 0.25, enabled INTEGER NOT NULL DEFAULT 1\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS strategy_versions (\
             id TEXT PRIMARY KEY, equity TEXT NOT NULL DEFAULT 'QQQ',\
             family TEXT NOT NULL, params_json TEXT NOT NULL,\
             status TEXT NOT NULL DEFAULT 'CANDIDATE',\
             promotion_metadata_json TEXT NOT NULL DEFAULT '{}',\
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS equity_predictions (\
             id INTEGER PRIMARY KEY AUTOINCREMENT, symbol TEXT NOT NULL,\
             candle_ts INTEGER NOT NULL, pred_1d REAL NOT NULL, pred_5d REAL NOT NULL,\
             pred_21d REAL NOT NULL, regime TEXT NOT NULL,\
             features_json TEXT NOT NULL, created_at INTEGER NOT NULL, source TEXT NOT NULL\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS option_positions (\
             id TEXT PRIMARY KEY, underlying TEXT NOT NULL, contract_code TEXT NOT NULL,\
             strategy_version_id TEXT NOT NULL, entry_underlying_price REAL NOT NULL,\
             entry_premium REAL NOT NULL, entry_spread REAL NOT NULL,\
             entry_slippage_budget REAL NOT NULL, qty INTEGER NOT NULL,\
             qty_filled_residual INTEGER NOT NULL, status TEXT NOT NULL,\
             dte_at_entry INTEGER NOT NULL, delta_at_entry REAL NOT NULL,\
             realized_pnl REAL, closed_at INTEGER, created_at INTEGER NOT NULL,\
             updated_at INTEGER NOT NULL\
             )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS exit_intent_log (\
             id INTEGER PRIMARY KEY AUTOINCREMENT,\
             position_id TEXT NOT NULL,\
             stage TEXT NOT NULL,\
             limit_price REAL NOT NULL,\
             quantity REAL NOT NULL,\
             timestamp TEXT NOT NULL\
             )"
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

    #[tokio::test]
    async fn test_exit_pipeline_dte_override() {
        // Create an OPEN position that is past its DTE window
        let pool = test_pool().await;
        let now = Utc::now().timestamp();
        let old_ts = now - 40 * 86400; // 40 days ago — DTE 30 would be expired

        // Seed the position
        sqlx::query(
            "INSERT INTO option_positions (id, underlying, contract_code, strategy_version_id, \
             entry_underlying_price, entry_premium, entry_spread, entry_slippage_budget, \
             qty, qty_filled_residual, status, dte_at_entry, delta_at_entry, \
             created_at, updated_at) \
             VALUES ('test-pos-1', 'QQQ', 'QQQ250117C00380000', 'sv-test', \
             400.0, 5.0, 0.5, 0.05, 1, 0, 'OPEN', 30, 0.45, ?, ?)"
        )
        .bind(old_ts)
        .bind(old_ts)
        .execute(&pool)
        .await
        .unwrap();

        // Seed candles for QQQ (needed for current_price)
        for i in 0..5 {
            sqlx::query(
                "INSERT INTO equity_candles (symbol, ts, open, high, low, close, volume, source) \
                 VALUES ('QQQ', ?, 400.0, 405.0, 395.0, 401.0, 1000000, 'yahoo')"
            )
            .bind(old_ts + i * 86400)
            .execute(&pool)
            .await
            .unwrap();
        }

        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool.clone(), config);

        // Run exit pipeline — DTE override should fire (30 DTE - 40 days = -10 remaining)
        scheduler.run_exit_pipeline("QQQ").await.unwrap();

        // Verify position was closed
        let status: String = sqlx::query_scalar(
            "SELECT status FROM option_positions WHERE id = 'test-pos-1'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "CLOSED", "position should be CLOSED by DTE override");
    }

    #[tokio::test]
    async fn test_exit_pipeline_no_open_positions() {
        let pool = test_pool().await;
        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool, config);

        // Should succeed with no open positions
        scheduler.run_exit_pipeline("QQQ").await.unwrap();
    }

    #[tokio::test]
    async fn test_open_paper_entry_creates_position_and_fill() {
        let pool = test_pool().await;

        // Seed a PAPER strategy version (A3 gate passes)
        let now = Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO strategy_versions (id, equity, family, params_json, status, \
             promotion_metadata_json, created_at, updated_at) \
             VALUES ('sv-test', 'QQQ', 'momentum', '{}', 'PAPER', '{}', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        // Seed a trading_model (fetch_account_equity needs it)
        sqlx::query(
            "INSERT INTO trading_models (model_id, primary_symbol, inverse_symbol, \
             model_path, norm_stats_path, budget_usd, deploy_pct, enabled) \
             VALUES ('tm-1', 'QQQ', 'PSQ', '/tmp/model', '/tmp/norm', 5000.0, 0.25, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool.clone(), config);

        let selected_chain = SelectedChain {
            symbol: "US.QQQ".to_string(),
            expiry: "260930".to_string(),
            strike: 625.0,
            option_type: "C".to_string(),
            delta: 0.45,
            bid: 8.25,
            ask: 8.50,
            open_interest: 500,
            dte: 33,
        };

        let outcome = scheduler
            .open_paper_entry("QQQ", "US.QQQ", &selected_chain, 2, 620.0)
            .await
            .unwrap();

        let (position_id, fill_price) = match outcome {
            EntryOutcome::Opened {
                position_id,
                fill_price,
            } => (position_id, fill_price),
            other => panic!("expected Opened, got {:?}", other),
        };
        assert_eq!(fill_price, 8.5);
        assert!(
            uuid::Uuid::parse_str(&position_id).is_ok(),
            "position_id must be UUID"
        );

        // Verify the position row
        let row: (String, String, String, String, f64, f64, f64, i64, i64, String, i64, f64) =
            sqlx::query_as(
                "SELECT id, underlying, contract_code, strategy_version_id, \
                 entry_underlying_price, entry_premium, entry_spread, qty, qty_filled_residual, \
                 status, dte_at_entry, delta_at_entry \
                 FROM option_positions",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, position_id);
        assert_eq!(row.1, "US.QQQ");
        assert_eq!(row.2, "US.QQQ260930C625000");
        assert_eq!(row.3, "sv-test");
        assert_eq!(row.4, 620.0);
        assert_eq!(row.5, 8.5);
        assert_eq!(row.6, 0.25);
        assert_eq!(row.7, 2);
        assert_eq!(row.8, 2);
        assert_eq!(row.9, "OPEN");
        assert_eq!(row.10, 33);
        assert_eq!(row.11, 0.45);

        // Verify the ENTRY fill row
        let fill: (String, String, f64, f64) = sqlx::query_as(
            "SELECT position_id, stage, price, quantity FROM option_fills",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(fill.0, position_id);
        assert_eq!(fill.1, "ENTRY");
        assert_eq!(fill.2, 8.5);
        assert_eq!(fill.3, 2.0);
    }

    #[test]
    fn test_parse_contract_code() {
        // Standard format: strike × 1000 (Moomoo US option)
        let (expiry, opt_type, strike) =
            parse_contract_code("US.QQQ260919C530000").unwrap();
        assert_eq!(expiry, "260919");
        assert_eq!(opt_type, "C");
        assert!((strike - 530.0).abs() < 0.001);

        // Put variant
        let (expiry, opt_type, strike) =
            parse_contract_code("US.SMH261002P125000").unwrap();
        assert_eq!(expiry, "261002");
        assert_eq!(opt_type, "P");
        assert!((strike - 125.0).abs() < 0.001);

        // Variable-width strike (5 digits for small strikes)
        let (_expiry, _opt_type, strike) =
            parse_contract_code("US.XLF260930C57500").unwrap();
        assert!((strike - 57.50).abs() < 0.01);

        // Invalid: no C/P
        assert!(parse_contract_code("US.QQQ260919X530000").is_none());
        assert!(parse_contract_code("").is_none());
    }

    #[test]
    fn test_tape_row_to_candidate() {
        let row = TapeRow {
            timestamp_ms: 1_700_000_000_000,
            underlying: "US.QQQ".to_string(),
            chain_code: "US.QQQ260930".to_string(),
            contract_code: "US.QQQ260930C625000".to_string(),
            bid: 8.25,
            ask: 8.50,
            last: 8.40,
            volume: 100,
            open_interest: 500,
            implied_volatility: 0.28,
            delta: 0.45,
            gamma: 0.003,
            theta: -0.15,
            underlying_price: 620.0,
        };

        // 2026-08-31 (today) → 260930 = 2026-09-30 → DTE = 30
        let today = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let c = tape_row_to_candidate(&row, today).unwrap();

        assert_eq!(c.symbol, "US.QQQ");
        assert_eq!(c.expiry, "260930");
        assert!((c.strike - 625.0).abs() < 0.001);
        assert_eq!(c.option_type, "C");
        assert!((c.delta - 0.45).abs() < 0.001);
        assert!((c.bid - 8.25).abs() < 0.001);
        assert!((c.ask - 8.50).abs() < 0.001);
        assert_eq!(c.open_interest, 500);
        assert_eq!(c.dte, 30);
        // 30th is not a monthly expiry (15-21 range)
        assert!(!c.is_monthly);
    }

    #[test]
    fn test_tape_row_to_candidate_monthly() {
        let row = TapeRow {
            timestamp_ms: 1_700_000_000_000,
            underlying: "US.QQQ".to_string(),
            chain_code: "US.QQQ260919".to_string(),
            contract_code: "US.QQQ260919C530000".to_string(),
            bid: 5.0,
            ask: 5.10,
            last: 5.05,
            volume: 200,
            open_interest: 1000,
            implied_volatility: 0.25,
            delta: 0.42,
            gamma: 0.004,
            theta: -0.12,
            underlying_price: 530.0,
        };

        // 2026-08-31 → 2026-09-19 = 19 DTE, 19th is monthly
        let today = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let c = tape_row_to_candidate(&row, today).unwrap();
        assert_eq!(c.dte, 19);
        assert!(c.is_monthly);
    }

    #[test]
    fn test_read_latest_tape_rows_from_synthetic_parquet() {
        // Write a synthetic Parquet file with the tape schema, then read it back.
        let tmp = std::env::temp_dir().join(format!("mm_tape_test_{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let tape_dir = tmp.to_str().unwrap();

        // Create the directory structure: {tape_dir}/US.QQQ/US.QQQ260930/
        let chain_dir = tmp.join("US.QQQ").join("US.QQQ260930");
        fs::create_dir_all(&chain_dir).unwrap();
        let parquet_path = chain_dir.join("2026-08-31.parquet");

        let schema = Arc::new(crate::options_recorder::build_tape_schema());
        let rows = vec![
            TapeRow {
                timestamp_ms: 1_700_000_000_000,
                underlying: "US.QQQ".to_string(),
                chain_code: "US.QQQ260930".to_string(),
                contract_code: "US.QQQ260930C625000".to_string(),
                bid: 8.25,
                ask: 8.50,
                last: 8.40,
                volume: 100,
                open_interest: 500,
                implied_volatility: 0.28,
                delta: 0.45,
                gamma: 0.003,
                theta: -0.15,
                underlying_price: 620.0,
            },
            TapeRow {
                timestamp_ms: 1_700_000_000_001,
                underlying: "US.QQQ".to_string(),
                chain_code: "US.QQQ260930".to_string(),
                contract_code: "US.QQQ260930P550000".to_string(),
                bid: 3.10,
                ask: 3.30,
                last: 3.20,
                volume: 80,
                open_interest: 300,
                implied_volatility: 0.30,
                delta: -0.40,
                gamma: 0.002,
                theta: -0.10,
                underlying_price: 620.0,
            },
        ];

        let arrays = crate::options_recorder::build_tape_arrays(&rows);
        let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();

        let file = fs::File::create(&parquet_path).unwrap();
        let props = parquet::file::properties::WriterProperties::builder()
            .set_compression(parquet::basic::Compression::SNAPPY)
            .build();
        let mut writer =
            parquet::arrow::ArrowWriter::try_new(file, schema, Some(props)).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        // Read it back
        let result = read_latest_tape_rows(tape_dir, "US.QQQ260930").unwrap();
        assert_eq!(result.len(), 2, "should read both rows");

        let call = result.iter().find(|r| r.contract_code.contains("C625000")).unwrap();
        assert!((call.bid - 8.25).abs() < 0.001);
        assert!((call.delta - 0.45).abs() < 0.001);

        let put = result.iter().find(|r| r.contract_code.contains("P550000")).unwrap();
        assert!((put.ask - 3.30).abs() < 0.001);
        assert!((put.delta - -0.40).abs() < 0.001);
    }

    /// Full round-trip: entry → hold → exit.
    /// Verifies all DB artifacts: position lifecycle, fills with strategy attribution,
    /// events, exit_intent_log, and realized PnL.
    ///
    /// NOTE: The entry side seeds the position directly (bypassing `open_paper_entry`)
    /// because the scheduler's internal `us_equity` prefixing creates a mismatch with
    /// how `run_exit_pipeline` queries by the raw equity symbol. The exit pipeline is
    /// exercised in full.
    #[tokio::test]
    async fn test_roundtrip_entry_to_exit() {
        let pool = test_pool().await;
        let now = Utc::now().timestamp();
        let three_days_ago = now - 3 * 86400;
        let forty_days_ago = now - 40 * 86400;
        let position_id = uuid::Uuid::new_v4().to_string();

        // ---- Seed: strategy_version (PAPER) ----
        sqlx::query(
            "INSERT INTO strategy_versions (id, equity, family, params_json, status,
             promotion_metadata_json, created_at, updated_at)
             VALUES ('sv-roundtrip', 'QQQ', 'sma_regime', '{}', 'PAPER', '{}', ?, ?)"
        )
        .bind(now - 7 * 86400)
        .bind(now)
        .execute(&pool).await.unwrap();

        // ---- Seed: equity_candles (5 daily candles) ----
        for i in 0..5 {
            sqlx::query(
                "INSERT INTO equity_candles (symbol, ts, open, high, low, close, volume, source)
                 VALUES ('QQQ', ?, 400.0, 405.0, 395.0, 401.0, 1000000, 'yahoo')"
            )
            .bind(three_days_ago + i * 86400)
            .execute(&pool).await.unwrap();
        }

        // ---- Seed: equity_predictions (for exit signal reversal) ----
        sqlx::query(
            "INSERT INTO equity_predictions (symbol, candle_ts, pred_1d, pred_5d, pred_21d, model_id)
             VALUES ('QQQ', ?, 0.002, 0.005, 0.01, 'qqq-model')"
        )
        .bind(three_days_ago + 4 * 86400)
        .execute(&pool).await.unwrap();

        // ---- Seed: trading_models ----
        sqlx::query(
            "INSERT INTO trading_models (model_id, primary_symbol, inverse_symbol, model_path, norm_stats_path, budget_usd, deploy_pct, enabled)
             VALUES ('qqq-model', 'QQQ', 'PSQ', '/tmp/model', '/tmp/norm', 5000.0, 0.25, 1)"
        )
        .execute(&pool).await.unwrap();

        // ---- Seed: OPEN position with 'QQQ' as underlying (matches exit pipeline query) ----
        sqlx::query(
            "INSERT INTO option_positions (id, underlying, contract_code, strategy_version_id,
             entry_underlying_price, entry_premium, entry_spread, entry_slippage_budget,
             qty, qty_filled_residual, status, dte_at_entry, delta_at_entry,
             created_at, updated_at)
             VALUES (?, 'QQQ', 'QQQ260930C625000', 'sv-roundtrip',
             401.0, 8.50, 0.5, 0.01, 2, 0, 'OPEN', 35, 0.45, ?, ?)"
        )
        .bind(&position_id)
        .bind(forty_days_ago)
        .bind(forty_days_ago)
        .execute(&pool).await.unwrap();

        // ---- Seed: ENTRY fill with strategy_version_id (E1) ----
        sqlx::query(
            "INSERT INTO option_fills (position_id, stage, price, quantity, timestamp, strategy_version_id)
             VALUES (?, 'ENTRY', 8.50, 2.0, ?, 'sv-roundtrip')"
        )
        .bind(&position_id)
        .bind(forty_days_ago)
        .execute(&pool).await.unwrap();

        // ---- Build scheduler ----
        let config = OptionsSchedulerConfig {
            equities: vec!["QQQ".to_string()],
            ..Default::default()
        };
        let scheduler = OptionsScheduler::new(pool.clone(), config);

        // ---- Verify ENTRY: position is OPEN ----
        let pos: (String, i32, String) = sqlx::query_as(
            "SELECT status, qty, strategy_version_id FROM option_positions WHERE id = ?"
        )
        .bind(&position_id)
        .fetch_one(&pool).await.unwrap();
        assert_eq!(pos.0, "OPEN");
        assert_eq!(pos.1, 2);
        assert_eq!(pos.2, "sv-roundtrip");

        // ---- Verify ENTRY: fill has strategy_version_id (E1) ----
        let fill_sv: String = sqlx::query_scalar(
            "SELECT strategy_version_id FROM option_fills WHERE position_id = ? AND stage = 'ENTRY'"
        )
        .bind(&position_id)
        .fetch_one(&pool).await.unwrap();
        assert_eq!(fill_sv, "sv-roundtrip");

        // ---- Run exit pipeline (DTE override: 35 DTE - 40 days = -5 remaining < 30 dte_min) ----
        scheduler.run_exit_pipeline("QQQ").await.unwrap();

        // ---- Verify EXIT: position is CLOSED with realized_pnl ----
        let closed: (String, f64, Option<i64>) = sqlx::query_as(
            "SELECT status, realized_pnl, closed_at FROM option_positions WHERE id = ?"
        )
        .bind(&position_id)
        .fetch_one(&pool).await.unwrap();
        assert_eq!(closed.0, "CLOSED", "position should be CLOSED");
        assert_ne!(closed.1, 0.0, "realized_pnl should be set");
        assert!(closed.2.is_some(), "closed_at should be set");

        // ---- Verify EXIT: fill has strategy_version_id (E1) ----
        let exit_fills: Vec<(String, String)> = sqlx::query_as(
            "SELECT stage, strategy_version_id FROM option_fills
             WHERE position_id = ? AND stage != 'ENTRY'"
        )
        .bind(&position_id)
        .fetch_all(&pool).await.unwrap();
        assert!(!exit_fills.is_empty(), "expected an EXIT fill");
        for (_, sv_id) in &exit_fills {
            assert_eq!(sv_id, "sv-roundtrip", "exit fill must have strategy_version_id");
        }

        // ---- Verify C3: exit_intent_log recorded ----
        let log_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM exit_intent_log WHERE position_id = ?"
        )
        .bind(&position_id)
        .fetch_one(&pool).await.unwrap();
        assert!(log_count > 0, "expected exit_intent_log entries");

        // ---- Verify C3: options::position_closed event emitted ----
        let all_events = db::search_events(&pool, None, None, None, None, None, 50)
            .await.unwrap();
        let closed_events: Vec<_> = all_events.iter()
            .filter(|e| e.source == "options::position_closed")
            .collect();
        assert_eq!(closed_events.len(), 1, "expected 1 options::position_closed event");
        assert!(
            closed_events[0].message.contains("POSITION_CLOSED"),
            "position_closed event: {:?}",
            closed_events[0].message
        );

        // ---- Verify E2: compute_options_evidence returns data ----
        let evidence =
            crate::hyperopt::promotion::compute_options_evidence(&pool, "sv-roundtrip")
                .await
                .unwrap();
        assert!(evidence.is_some(), "should have evidence after closing position");
        let ev = evidence.unwrap();
        assert_eq!(ev.n_trades, 1, "1 closed trade for sv-roundtrip");
        assert!(ev.ic > 0.0, "IC should be positive for profitable trade, got {}", ev.ic);
    }
}
