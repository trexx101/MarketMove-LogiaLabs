//! Entry integration — wires macro gate, chain selector, sizing, and entry executor
//!
//! Pre-entry guards:
//! - Reconciliation gate must be clean
//! - Circuit breaker must not be active
//!
//! Pipeline: signal → macro gate → chain selector → sizing → entry executor → arbiter ownership

use serde::{Deserialize, Serialize};

use super::chain_selector::{CandidateChain, ChainSelectionResult, ChainSelector, ChainSelectorConfig};
use super::circuit_breaker::CircuitBreaker;
use super::entry_executor::{EntryExecutor, EntryStage};
use super::macro_gate::{CalendarEvent, MacroGate, MacroGateConfig, MacroGateDecision};
use super::reconciliation::ReconciliationResult;
use super::sizing::{PositionSizer, SizingConfig, SizingResult};

/// Entry pipeline result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryPipelineResult {
    /// Entry proceeded through the pipeline
    Initiated(EntryInitiated),
    /// Entry was skipped with a reason
    Skipped(EntrySkipReason),
}

/// Entry was initiated successfully
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryInitiated {
    pub position_id: i64,
    pub symbol: String,
    pub expiry: String,
    pub strike: f64,
    pub option_type: String,
    pub contracts: u32,
    pub limit_price: f64,
    pub stage: EntryStage,
}

/// Reason entry was skipped
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntrySkipReason {
    ReconciliationDirty,
    CircuitBreakerActive,
    MacroGateDenied { reason: String },
    NoChainSelected { reason: String },
    SizingFailed { reason: String },
}

/// Pre-entry guard state
pub struct PreEntryGuards {
    pub circuit_breaker: CircuitBreaker,
}

impl PreEntryGuards {
    pub fn new(circuit_breaker: CircuitBreaker) -> Self {
        Self { circuit_breaker }
    }

    /// Check if reconciliation is clean
    pub fn check_reconciliation(&self, result: &ReconciliationResult) -> bool {
        result.is_clean
    }

    /// Check if circuit breaker is inactive
    pub fn check_circuit_breaker(&self) -> bool {
        !self.circuit_breaker.is_triggered()
    }
}

/// Full entry pipeline
pub struct EntryPipeline {
    pub macro_gate: MacroGate,
    pub chain_selector: ChainSelector,
    pub sizer: PositionSizer,
}

impl EntryPipeline {
    pub fn new(
        macro_gate: MacroGate,
        chain_selector: ChainSelector,
        sizer: PositionSizer,
    ) -> Self {
        Self {
            macro_gate,
            chain_selector,
            sizer,
        }
    }

    /// Run the full entry pipeline
    pub fn run(
        &self,
        position_id: i64,
        guards: &PreEntryGuards,
        reconciliation: &ReconciliationResult,
        current_vix: f64,
        vix_5d_ago: f64,
        now: chrono::DateTime<chrono::Utc>,
        calendar_events: &[CalendarEvent],
        candidates: &[CandidateChain],
        equity: f64,
        stop_distance: f64,
        current_portfolio_premium: f64,
    ) -> EntryPipelineResult {
        // Guard 1: Reconciliation must be clean
        if !guards.check_reconciliation(reconciliation) {
            return EntryPipelineResult::Skipped(EntrySkipReason::ReconciliationDirty);
        }

        // Guard 2: Circuit breaker must not be active
        if !guards.check_circuit_breaker() {
            return EntryPipelineResult::Skipped(EntrySkipReason::CircuitBreakerActive);
        }

        // Step 1: Macro gate
        let macro_decision = self.macro_gate.evaluate(current_vix, vix_5d_ago, now, calendar_events);
        if let MacroGateDecision::Denied(reason) = macro_decision {
            return EntryPipelineResult::Skipped(EntrySkipReason::MacroGateDenied {
                reason: format!("{:?}", reason),
            });
        }

        // Step 2: Chain selection
        let chain_result = self.chain_selector.select(candidates);
        let selected = match chain_result {
            ChainSelectionResult::Selected(chain) => chain,
            ChainSelectionResult::Skipped(reason) => {
                return EntryPipelineResult::Skipped(EntrySkipReason::NoChainSelected {
                    reason: format!("{:?}", reason),
                });
            }
        };

        // Step 3: Position sizing
        let sizing_result = self.sizer.size(
            equity,
            stop_distance,
            selected.delta,
            selected.ask,
            current_portfolio_premium,
        );
        let sizing = match sizing_result {
            SizingResult::Sized(d) => d,
            SizingResult::Skipped(reason) => {
                return EntryPipelineResult::Skipped(EntrySkipReason::SizingFailed {
                    reason: format!("{:?}", reason),
                });
            }
        };

        // Step 4: Initiate entry executor
        let executor = EntryExecutor::new(position_id, selected.ask, 0.10);

        EntryPipelineResult::Initiated(EntryInitiated {
            position_id,
            symbol: selected.symbol,
            expiry: selected.expiry,
            strike: selected.strike,
            option_type: selected.option_type,
            contracts: sizing.contracts,
            limit_price: executor.current_limit_price(),
            stage: executor.current_stage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::reconciliation::Mismatch;
    use chrono::Utc;

    fn clean_reconciliation() -> ReconciliationResult {
        ReconciliationResult::new()
    }

    fn dirty_reconciliation() -> ReconciliationResult {
        let mut r = ReconciliationResult::new();
        r.add_mismatch(Mismatch::MissingOrder("test".to_string()));
        r
    }

    fn make_candidate() -> CandidateChain {
        CandidateChain {
            symbol: "QQQ".to_string(),
            expiry: "2026-09-20".to_string(),
            strike: 450.0,
            option_type: "call".to_string(),
            delta: 0.45,
            bid: 5.0,
            ask: 5.10,
            open_interest: 200,
            dte: 35,
            is_monthly: true,
        }
    }

    fn default_pipeline() -> EntryPipeline {
        EntryPipeline::new(
            MacroGate::new(MacroGateConfig::default()),
            ChainSelector::new(ChainSelectorConfig::default()),
            PositionSizer::new(SizingConfig::default()),
        )
    }

    fn default_guards() -> PreEntryGuards {
        PreEntryGuards::new(CircuitBreaker::new(300, 4))
    }

    #[test]
    fn test_skip_on_dirty_reconciliation() {
        let pipeline = default_pipeline();
        let guards = default_guards();
        let dirty = dirty_reconciliation();
        let now = Utc::now();

        let result = pipeline.run(
            1, &guards, &dirty,
            20.0, 20.0, now, &[],
            &[make_candidate()],
            100_000.0, 10.0, 0.0,
        );

        assert!(matches!(result, EntryPipelineResult::Skipped(EntrySkipReason::ReconciliationDirty)));
    }

    #[test]
    fn test_skip_on_circuit_breaker_active() {
        let pipeline = default_pipeline();

        let mut cb = CircuitBreaker::new(300, 4);
        for _ in 0..4 {
            cb.record_loss();
        }
        assert!(cb.is_triggered());

        let guards = PreEntryGuards::new(cb);
        let clean = clean_reconciliation();
        let now = Utc::now();

        let result = pipeline.run(
            1, &guards, &clean,
            20.0, 20.0, now, &[],
            &[make_candidate()],
            100_000.0, 10.0, 0.0,
        );

        assert!(matches!(result, EntryPipelineResult::Skipped(EntrySkipReason::CircuitBreakerActive)));
    }

    #[test]
    fn test_skip_on_macro_gate_denied() {
        let pipeline = default_pipeline();
        let guards = default_guards();
        let clean = clean_reconciliation();
        let now = Utc::now();

        let result = pipeline.run(
            1, &guards, &clean,
            35.0, 20.0, now, &[], // VIX too high
            &[make_candidate()],
            100_000.0, 10.0, 0.0,
        );

        assert!(matches!(result, EntryPipelineResult::Skipped(EntrySkipReason::MacroGateDenied { .. })));
    }

    #[test]
    fn test_skip_on_no_chain_selected() {
        let pipeline = default_pipeline();
        let guards = default_guards();
        let clean = clean_reconciliation();
        let now = Utc::now();

        let result = pipeline.run(
            1, &guards, &clean,
            20.0, 20.0, now, &[],
            &[], // No candidates
            100_000.0, 10.0, 0.0,
        );

        assert!(matches!(result, EntryPipelineResult::Skipped(EntrySkipReason::NoChainSelected { .. })));
    }

    #[test]
    fn test_skip_on_sizing_failure() {
        let pipeline = default_pipeline();
        let guards = default_guards();
        let clean = clean_reconciliation();
        let now = Utc::now();

        let result = pipeline.run(
            1, &guards, &clean,
            20.0, 20.0, now, &[],
            &[make_candidate()],
            100_000.0, 0.0, 0.0, // stop_distance = 0 → sizing fails
        );

        assert!(matches!(result, EntryPipelineResult::Skipped(EntrySkipReason::SizingFailed { .. })));
    }

    #[test]
    fn test_full_pipeline_success() {
        let pipeline = default_pipeline();
        let guards = default_guards();
        let clean = clean_reconciliation();
        let now = Utc::now();

        let result = pipeline.run(
            1, &guards, &clean,
            20.0, 20.0, now, &[],
            &[make_candidate()],
            100_000.0, 10.0, 0.0,
        );

        match result {
            EntryPipelineResult::Initiated(entry) => {
                assert_eq!(entry.position_id, 1);
                assert_eq!(entry.symbol, "QQQ");
                assert_eq!(entry.contracts, 2); // floor(100000 * 0.01 / (10 * 0.45 * 100)) = 2
                assert_eq!(entry.limit_price, 5.10); // ask price
                assert_eq!(entry.stage, EntryStage::Stage1);
            }
            EntryPipelineResult::Skipped(reason) => {
                panic!("Expected Initiated, got Skipped: {:?}", reason);
            }
        }
    }
}
