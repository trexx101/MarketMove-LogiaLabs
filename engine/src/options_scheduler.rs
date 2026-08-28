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
use chrono::{DateTime, Utc};
use sqlx::Row;
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
    /// Hyperopt promotion gates (min-days observation clocks).
    pub promotion_gates: crate::config::PromotionGatesConfig,
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

        // Publish ENTRY_INITIATED event for the Events tab (trade category)
        let payload = serde_json::json!({
            "candle_ts": candle_ts,
            "symbol": selected_chain.symbol,
            "expiry": selected_chain.expiry,
            "strike": selected_chain.strike,
            "option_type": selected_chain.option_type,
            "delta": selected_chain.delta,
            "dte": selected_chain.dte,
            "ask": selected_chain.ask,
            "contracts": sizing_decision.contracts,
            "total_premium": sizing_decision.total_premium,
        });
        if let Err(e) = db::insert_event(
            &self.pool,
            "trade",
            "info",
            &self.config.mode,
            "options::entry",
            &format!(
                "ENTRY_INITIATED {equity}: {} {} {} @ {:.2} x{}",
                selected_chain.symbol,
                selected_chain.expiry,
                selected_chain.option_type,
                selected_chain.ask,
                sizing_decision.contracts
            ),
            &payload.to_string(),
            Some(equity),
        )
        .await
        {
            error!(equity, error = %e, "failed to record ENTRY_INITIATED event");
        }

        Ok(EntryPipelineResult {
            entry_initiated: true,
            reason: None,
        })
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
             entry_premium, delta_at_entry, created_at, strategy_version_id \
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
            let delta: f64 = pos.get("delta_at_entry");
            let created_at: i64 = pos.get("created_at");
            // We need position_id as i64 for the executor; parse from TEXT id
            // (use a safe default if the id isn't parseable as i64 — real UUIDs aren't)
            let pos_id_i64: i64 = pos_id
                .split('-')
                .next()
                .and_then(|s| i64::from_str_radix(s, 16).ok())
                .unwrap_or(0);

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

                // Initiate staged exit
                match executor.initiate_exit(pos_id_i64, current_bid, tick_size).await {
                    Ok(_stage) => {
                        // Paper mode: advance through ladder stages immediately
                        // (no real market to wait for)
                        let fill = loop {
                            let current_stage = executor
                                .get_ladder(pos_id_i64)
                                .map(|l| l.current_stage());
                            if current_stage == Some(ExitStage::Complete) {
                                break None;
                            }

                            match executor
                                .try_fill(pos_id_i64, current_bid, current_price, Utc::now())
                                .await
                            {
                                Ok(Some(fill)) => break Some(fill),
                                Ok(None) => {
                                    // Advance to next stage
                                    executor.advance_ladder(pos_id_i64, current_bid)?;
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
                            info!(
                                equity,
                                position_id = %pos_id,
                                fill_price = fill.price,
                                "exit filled — marking position CLOSED"
                            );
                            // Mark position CLOSED
                            let _ = sqlx::query(
                                "UPDATE option_positions SET status = 'CLOSED', \
                                 closed_at = ?, updated_at = ? WHERE id = ?"
                            )
                            .bind(now)
                            .bind(now)
                            .bind(&pos_id)
                            .execute(&self.pool)
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
        // Query known chain codes from option_tape_meta
        let chain_rows = sqlx::query(
            "SELECT chain_code, last_heartbeat_ts FROM option_tape_meta WHERE underlying = ?1 ORDER BY chain_code",
        )
        .bind(equity)
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

        // TODO(Phase B): read latest Parquet tape rows for these chains,
        // extract bid/ask/delta/dte/oi, convert to CandidateChain vec.
        // For now the tape recorder isn't writing Parquet yet.
        warn!(%equity, "tape Parquet not populated yet — returning empty candidates");
        Ok(vec![])
    }

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
             position_id INTEGER NOT NULL,\
             stage TEXT NOT NULL,\
             price REAL NOT NULL,\
             quantity REAL NOT NULL,\
             timestamp INTEGER NOT NULL\
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
}
