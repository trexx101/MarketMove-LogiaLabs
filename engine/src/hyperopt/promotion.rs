//! Promotion pipeline — state machine with evidence gates
//!
//! CANDIDATE → PAPER → MICRO → LIVE
//! Each transition requires evidence gates to be met.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::candidate_store::{CandidateSnapshot, CandidateStatus, CandidateStore};

/// Promotion stage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromotionStage {
    Candidate,
    Paper,
    Micro,
    Live,
    Rejected,
}

/// Evidence gate requirement
#[derive(Debug, Clone)]
pub struct GateRequirement {
    pub min_trades: usize,
    pub min_ic: f64,
    pub min_sharpe: f64,
    pub min_days: usize,
}

impl Default for GateRequirement {
    fn default() -> Self {
        Self {
            min_trades: 30,
            min_ic: 0.03,
            min_sharpe: 1.0,
            min_days: 14,
        }
    }
}

/// Promotion result
#[derive(Debug, Clone)]
pub struct PromotionResult {
    pub promoted: bool,
    pub from_stage: PromotionStage,
    pub to_stage: PromotionStage,
    pub reason: String,
}

/// Promotion evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionEvidence {
    pub n_trades: usize,
    pub ic: f64,
    pub sharpe: f64,
    pub days_observed: usize,
    /// Per-fold walk-forward ICs at candidate creation time. Defaulted on
    /// deserialize so evidence JSON queued before the 2026-08-25 fold-
    /// consistency gate (docs/triage/2026-08-25-options-negative-ic.md)
    /// still loads — an absent list skips the gate rather than failing it.
    #[serde(default)]
    pub fold_ics: Vec<f64>,
}

/// Promotion pipeline
pub struct PromotionPipeline {
    /// Gates for each transition
    pub candidate_to_paper: GateRequirement,
    pub paper_to_micro: GateRequirement,
    pub micro_to_live: GateRequirement,
}

impl Default for PromotionPipeline {
    fn default() -> Self {
        Self::with_gates(crate::config::PromotionGatesConfig::default())
    }
}

impl PromotionPipeline {
    /// Build the pipeline from configured gates.
    ///
    /// NOTE (2026-08-28): min_sharpe stays DISABLED (0.0) on every stage —
    /// no options executor exists yet, so there is no per-candidate returns
    /// data to compute Sharpe from, and fabricating one is forbidden.
    /// min_days IS active for PAPER→MICRO and MICRO→LIVE: its source of
    /// truth is the candidate's own observation clock (`updated_at`, stamped
    /// on every status flip — see `live_days_observed`). CANDIDATE→PAPER
    /// keeps min_days=0 because observation starts only at PAPER.
    pub fn with_gates(gates: crate::config::PromotionGatesConfig) -> Self {
        Self {
            candidate_to_paper: GateRequirement {
                min_trades: 100,
                min_ic: 0.03,
                min_sharpe: 0.0,
                min_days: 0, // observation clock starts at PAPER
            },
            paper_to_micro: GateRequirement {
                min_trades: 30,
                min_ic: 0.03,
                min_sharpe: 0.0, // disabled — no returns source yet
                min_days: gates.min_days_paper_to_micro,
            },
            micro_to_live: GateRequirement {
                min_trades: 50,
                min_ic: 0.04,
                min_sharpe: 0.0, // disabled — no returns source yet
                min_days: gates.min_days_micro_to_live,
            },
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Days a candidate has been under observation in its CURRENT stage.
    ///
    /// Source of truth: `strategy_versions.updated_at`, which
    /// `CandidateStore::update_status` stamps on every status flip. So for a
    /// PAPER candidate this is "days since promoted to PAPER". Live-computed
    /// at check/apply time — never taken from queue-time evidence, which is
    /// stale by definition for a days gate.
    pub fn live_days_observed(snapshot: &CandidateSnapshot) -> usize {
        let now = chrono::Utc::now().timestamp();
        let elapsed_secs = now.saturating_sub(snapshot.updated_at);
        (elapsed_secs / 86_400) as usize
    }

    /// Check if promotion is allowed
    pub fn check_promotion(&self, current: PromotionStage, evidence: &PromotionEvidence) -> PromotionResult {
        let (gate, next_stage) = match current {
            PromotionStage::Candidate => (&self.candidate_to_paper, PromotionStage::Paper),
            PromotionStage::Paper => (&self.paper_to_micro, PromotionStage::Micro),
            PromotionStage::Micro => (&self.micro_to_live, PromotionStage::Live),
            PromotionStage::Live | PromotionStage::Rejected => {
                return PromotionResult {
                    promoted: false,
                    from_stage: current,
                    to_stage: current,
                    reason: "Terminal stage".to_string(),
                };
            }
        };

        // Fold-consistency gate (2026-08-25 triage,
        // docs/triage/2026-08-25-options-negative-ic.md): a candidate whose
        // mean IC clears the bar on the strength of a subset of folds — with
        // any test fold negative — is "carried by one bad fold" and does not
        // earn promotion. Applied at every transition; the strongest form of
        // the rule (min fold > 0) guards the first hop where all walk-forward
        // evidence lives.
        if !evidence.fold_ics.is_empty() && evidence.fold_ics.iter().any(|&ic| ic <= 0.0) {
            return PromotionResult {
                promoted: false,
                from_stage: current,
                to_stage: current,
                reason: format!(
                    "Fold inconsistency: all walk-forward folds must be positive, got {:?}",
                    evidence.fold_ics.iter().map(|ic| format!("{ic:+.4}")).collect::<Vec<_>>()
                ),
            };
        }

        // Check each gate
        if evidence.n_trades < gate.min_trades {
            return PromotionResult {
                promoted: false,
                from_stage: current,
                to_stage: current,
                reason: format!(
                    "Insufficient trades: {} < {}",
                    evidence.n_trades, gate.min_trades
                ),
            };
        }

        if evidence.ic < gate.min_ic {
            return PromotionResult {
                promoted: false,
                from_stage: current,
                to_stage: current,
                reason: format!("IC too low: {:.4} < {:.4}", evidence.ic, gate.min_ic),
            };
        }

        if evidence.sharpe < gate.min_sharpe {
            return PromotionResult {
                promoted: false,
                from_stage: current,
                to_stage: current,
                reason: format!(
                    "Sharpe too low: {:.2} < {:.2}",
                    evidence.sharpe, gate.min_sharpe
                ),
            };
        }

        if evidence.days_observed < gate.min_days {
            return PromotionResult {
                promoted: false,
                from_stage: current,
                to_stage: current,
                reason: format!(
                    "Insufficient observation days: {} < {}",
                    evidence.days_observed, gate.min_days
                ),
            };
        }

        PromotionResult {
            promoted: true,
            from_stage: current,
            to_stage: next_stage,
            reason: "All gates passed".to_string(),
        }
    }

    /// Get current stage requirements
    pub fn get_requirements(&self, stage: PromotionStage) -> Option<&GateRequirement> {
        match stage {
            PromotionStage::Candidate => Some(&self.candidate_to_paper),
            PromotionStage::Paper => Some(&self.paper_to_micro),
            PromotionStage::Micro => Some(&self.micro_to_live),
            _ => None,
        }
    }

    /// Map a candidate's persisted status to its promotion stage.
    pub fn stage_for_status(&self, status: &CandidateStatus) -> PromotionStage {
        match status {
            CandidateStatus::New | CandidateStatus::Stable => PromotionStage::Candidate,
            CandidateStatus::Paper => PromotionStage::Paper,
            CandidateStatus::Micro => PromotionStage::Micro,
            CandidateStatus::Live => PromotionStage::Live,
            CandidateStatus::Unstable | CandidateStatus::Retired => PromotionStage::Rejected,
        }
    }

    /// Dry-run gate check for a snapshot without any DB writes (D13).
    /// Used by the promote endpoint to fail fast before queueing.
    ///
    /// days_observed is overwritten with the LIVE observation clock
    /// (`updated_at` → days in current stage). Queue-time or caller-supplied
    /// days values are stale by definition for a days gate and are ignored.
    pub fn check_snapshot(
        &self,
        snapshot: &CandidateSnapshot,
        evidence: &PromotionEvidence,
    ) -> PromotionResult {
        let mut live = evidence.clone();
        live.days_observed = Self::live_days_observed(snapshot);
        self.check_promotion(self.stage_for_status(&snapshot.status), &live)
    }

    /// Promote a candidate (DB-backed)
    ///
    /// Reads evidence from the candidate snapshot, checks gates, and updates status if promotion passes.
    /// days_observed is overwritten with the LIVE observation clock (see
    /// `check_snapshot`) — a candidate cannot satisfy the days gate with
    /// evidence numbers, only with real time spent in the current stage.
    pub async fn promote(
        &self,
        store: &CandidateStore,
        version_id: &str,
        evidence: &PromotionEvidence,
    ) -> Result<PromotionResult> {
        let snapshot = store.get(version_id).await?;
        let snapshot = match snapshot {
            Some(s) => s,
            None => {
                return Ok(PromotionResult {
                    promoted: false,
                    from_stage: PromotionStage::Rejected,
                    to_stage: PromotionStage::Rejected,
                    reason: format!("Candidate {} not found", version_id),
                });
            }
        };

        let current_stage = self.stage_for_status(&snapshot.status);

        let mut live = evidence.clone();
        live.days_observed = Self::live_days_observed(&snapshot);
        let result = self.check_promotion(current_stage, &live);

        if result.promoted {
            let new_status = match result.to_stage {
                PromotionStage::Paper => CandidateStatus::Paper,
                PromotionStage::Micro => CandidateStatus::Micro,
                PromotionStage::Live => CandidateStatus::Live,
                _ => snapshot.status,
            };
            store.update_status(version_id, new_status).await?;
        }

        Ok(result)
    }
}

/// D13 boundary applier: run at the daily candle close for an equity.
///
/// Applies queued promotion requests for that equity. Before flipping any
/// status it RE-CHECKS the mid-exit gate (open positions may have been
/// opened after queueing); positions present → request stays queued.
///
/// Returns (applied_count, skipped_count).
pub async fn apply_pending_promotions(
    pool: &crate::db::DbPool,
    equity: &str,
    mode: &str,
    pipeline: &PromotionPipeline,
    store: &CandidateStore,
) -> Result<(usize, usize)> {
    let mut applied = 0usize;
    let mut skipped = 0usize;

    for pending in crate::db::list_pending_promotions(pool).await? {
        if pending.equity != equity {
            continue;
        }

        // Mid-exit re-check: never promote while a position is open.
        let open = crate::db::list_option_positions(pool, Some(equity), Some("OPEN"), 1).await?;
        if !open.is_empty() {
            skipped += 1; // stays queued; retried at the next boundary
            continue;
        }

        // Use the evidence validated at queue time (persisted with the request).
        // Never fabricate evidence here. days_observed from the persisted
        // evidence is IGNORED by promote() — it recomputes the live
        // observation clock from strategy_versions.updated_at.
        let evidence: PromotionEvidence = match serde_json::from_str(&pending.evidence_json) {
            Ok(e) => e,
            Err(e) => {
                crate::db::mark_pending_promotion_applied(
                    pool,
                    &pending.version_id,
                    &format!("DENIED: corrupt evidence_json ({e})"),
                )
                .await?;
                continue;
            }
        };

        let result = pipeline.promote(store, &pending.version_id, &evidence).await?;
        let outcome = if result.promoted {
            applied += 1;
            format!("PROMOTED to {:?}", result.to_stage)
        } else {
            format!("DENIED: {}", result.reason)
        };
        crate::db::mark_pending_promotion_applied(pool, &pending.version_id, &outcome).await?;

        // Publish promotion outcome event for the Events tab (strategy category)
        let payload = serde_json::json!({
            "version_id": pending.version_id,
            "target_status": pending.target_status,
            "outcome": outcome,
        });
        if let Err(e) = crate::db::insert_event(
            pool,
            "strategy",
            if result.promoted { "info" } else { "warn" },
            mode,
            "hyperopt::promotion",
            &format!("PROMOTION {equity} {}: {outcome}", pending.version_id),
            &payload.to_string(),
            Some(equity),
        )
        .await
        {
            tracing::error!(equity, error = %e, "failed to record PROMOTION event");
        }
    }

    Ok((applied, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use crate::db::DbPool;

    /// Test pool with the three tables the D13 flow touches.
    async fn d13_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
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
            );
            CREATE TABLE IF NOT EXISTS option_positions (
                id                      TEXT    PRIMARY KEY,
                underlying              TEXT    NOT NULL,
                contract_code           TEXT    NOT NULL,
                strategy_version_id     TEXT    NOT NULL,
                entry_underlying_price  REAL    NOT NULL,
                entry_premium           REAL    NOT NULL DEFAULT 0.0,
                entry_spread            REAL    NOT NULL,
                entry_slippage_budget   REAL    NOT NULL,
                qty                     INTEGER NOT NULL,
                qty_filled_residual     INTEGER NOT NULL DEFAULT 0,
                status                  TEXT    NOT NULL DEFAULT 'OPEN',
                dte_at_entry            INTEGER NOT NULL,
                delta_at_entry          REAL    NOT NULL,
                realized_pnl            REAL,
                closed_at               INTEGER,
                created_at              INTEGER NOT NULL,
                updated_at              INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pending_promotions (
                id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                version_id         TEXT    NOT NULL,
                equity             TEXT    NOT NULL,
                target_status      TEXT    NOT NULL,
                evidence_json      TEXT    NOT NULL DEFAULT '{}',
                requested_at       INTEGER NOT NULL,
                applied_at         INTEGER,
                applied_result     TEXT,
                UNIQUE(version_id, equity)
            );
            CREATE TABLE IF NOT EXISTS engine_events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                ts           INTEGER NOT NULL,
                category     TEXT    NOT NULL,
                severity     TEXT    NOT NULL,
                mode         TEXT    NOT NULL,
                source       TEXT    NOT NULL,
                message      TEXT    NOT NULL,
                payload_json TEXT    NOT NULL DEFAULT '{}',
                equity       TEXT
            );
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_open_position(pool: &DbPool, underlying: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO option_positions
             (id, underlying, contract_code, strategy_version_id, entry_underlying_price,
              entry_premium, entry_spread, entry_slippage_budget, qty, status,
              dte_at_entry, delta_at_entry, created_at, updated_at)
             VALUES (?1, ?2, 'TEST_CONTRACT', 'v-test', 450.0, 3.5, 0.05, 0.10, 1, 'OPEN',
                     40, 0.45, ?3, ?3)",
        )
        .bind(format!("pos-{underlying}"))
        .bind(underlying)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Evidence that passes the Candidate→Paper gate, serialized as stored.
    fn passing_evidence_json() -> String {
        serde_json::to_string(&PromotionEvidence {
            n_trades: 150,
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![0.05, 0.06],
        })
        .unwrap()
    }

    #[tokio::test]
    async fn test_apply_pending_promotions_promotes_at_boundary() {
        let pool = d13_pool().await;
        let store = CandidateStore::new(pool.clone());
        let pipeline = PromotionPipeline::new();

        // Candidate with passing evidence (n_trades >= 100, ic >= 0.03)
        let version_id = store
            .store("QQQ", "momentum", HashMap::new(), 0.05, 0.01, 150, vec![0.04, 0.05, 0.06])
            .await
            .unwrap();
        crate::db::queue_pending_promotion(&pool, &version_id, "QQQ", "PAPER", &passing_evidence_json())
            .await
            .unwrap();

        // No open positions → promotion applies at the boundary
        let (applied, skipped) =
            apply_pending_promotions(&pool, "QQQ", "paper", &pipeline, &store).await.unwrap();
        assert_eq!(applied, 1);
        assert_eq!(skipped, 0);

        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.status, CandidateStatus::Paper);

        // Request marked applied, no longer pending
        assert!(crate::db::fetch_pending_promotion(&pool, &version_id).await.unwrap().is_none());

        // Promotion outcome event published via insert_event (category + mode searchable)
        let events = crate::db::search_events(&pool, Some("strategy"), Some("paper"), None, Some("QQQ"), None, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("PROMOTION QQQ"));
        assert!(events[0].message.contains("PROMOTED to Paper"));
    }

    #[tokio::test]
    async fn test_apply_pending_promotions_blocked_by_open_position() {
        let pool = d13_pool().await;
        let store = CandidateStore::new(pool.clone());
        let pipeline = PromotionPipeline::new();

        let version_id = store
            .store("QQQ", "momentum", HashMap::new(), 0.05, 0.01, 150, vec![0.04, 0.05, 0.06])
            .await
            .unwrap();
        crate::db::queue_pending_promotion(&pool, &version_id, "QQQ", "PAPER", &passing_evidence_json())
            .await
            .unwrap();

        // Open position → D13 mid-exit block: stays queued, status untouched
        insert_open_position(&pool, "QQQ").await;
        let (applied, skipped) =
            apply_pending_promotions(&pool, "QQQ", "paper", &pipeline, &store).await.unwrap();
        assert_eq!(applied, 0);
        assert_eq!(skipped, 1);

        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.status, CandidateStatus::New);
        assert!(crate::db::fetch_pending_promotion(&pool, &version_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_queue_pending_promotion_upsert_replaces_previous() {
        let pool = d13_pool().await;
        crate::db::queue_pending_promotion(&pool, "v-1", "QQQ", "PAPER", "{}").await.unwrap();
        crate::db::queue_pending_promotion(&pool, "v-1", "QQQ", "MICRO", "{}").await.unwrap();

        let pending = crate::db::list_pending_promotions(&pool).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].target_status, "MICRO");
    }

    #[tokio::test]
    async fn test_apply_pending_promotions_ignores_other_equities() {
        let pool = d13_pool().await;
        let store = CandidateStore::new(pool.clone());
        let pipeline = PromotionPipeline::new();

        let version_id = store
            .store("SMH", "momentum", HashMap::new(), 0.05, 0.01, 150, vec![0.04, 0.05, 0.06])
            .await
            .unwrap();
        crate::db::queue_pending_promotion(&pool, &version_id, "SMH", "PAPER", "{}")
            .await
            .unwrap();

        // Boundary for QQQ must not touch SMH's queue
        let (applied, skipped) =
            apply_pending_promotions(&pool, "QQQ", "paper", &pipeline, &store).await.unwrap();
        assert_eq!(applied, 0);
        assert_eq!(skipped, 0);
        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.status, CandidateStatus::New);
    }

    #[test]
    fn test_promote_candidate_to_paper_pass() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 150,
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Candidate, &evidence);
        assert!(result.promoted);
        assert_eq!(result.to_stage, PromotionStage::Paper);
    }

    #[test]
    fn test_promote_candidate_to_paper_fail_trades() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 12, // < 100
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Candidate, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("Insufficient trades"));
    }

    #[test]
    fn test_promote_candidate_to_paper_fail_ic() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 150,
            ic: 0.01, // < 0.03
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Candidate, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("IC too low"));
    }

    #[test]
    fn test_promote_paper_to_micro_fails_on_positive_trade_count_and_ic_bars() {
        let pipeline = PromotionPipeline::new();
        // Gates ACTIVE for paper->micro: min_trades + min_ic + fold
        // consistency + min_days (live observation clock, 2026-08-28).
        // min_sharpe remains disabled (0.0) — no returns source exists yet.
        let evidence = PromotionEvidence {
            // Below paper->micro min_trades (30) => must reject on trades
            // before the days gate is even consulted.
            n_trades: 1,
            ic: 0.05,
            sharpe: 0.0,
            days_observed: 7, // below the 14d gate too, but trades rejects first
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Paper, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("Insufficient trades"));
    }

    #[test]
    fn test_promote_paper_to_micro_rejects_when_days_short() {
        // min_days is ACTIVE (2026-08-28): a paper candidate with passing
        // IC/n_trades but fewer than the configured observation days must
        // NOT promote.
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 100,
            ic: 0.05,
            sharpe: 0.0, // ignored: min_sharpe=0 (disabled)
            days_observed: 7, // below the 14d default gate
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Paper, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("Insufficient observation days"));
    }

    #[test]
    fn test_promote_paper_to_micro_pass_when_days_satisfied() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 100,
            ic: 0.05,
            sharpe: 0.0,      // ignored: min_sharpe=0
            days_observed: 14, // meets the 14d gate
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Paper, &evidence);
        assert!(result.promoted);
    }

    #[test]
    fn test_promote_micro_to_live_pass() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 60,
            ic: 0.05,
            sharpe: 2.0,
            days_observed: 45,
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Micro, &evidence);
        assert!(result.promoted);
        assert_eq!(result.to_stage, PromotionStage::Live);
    }

    #[test]
    fn test_terminal_stage_no_promotion() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 1000,
            ic: 0.10,
            sharpe: 3.0,
            days_observed: 365,
            fold_ics: vec![],
        };

        let result = pipeline.check_promotion(PromotionStage::Live, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("Terminal stage"));
    }

    #[test]
    fn test_get_requirements() {
        let pipeline = PromotionPipeline::new();
        
        let req = pipeline.get_requirements(PromotionStage::Candidate).unwrap();
        assert_eq!(req.min_trades, 100);

        let req = pipeline.get_requirements(PromotionStage::Paper).unwrap();
        // min_days is ACTIVE (2026-08-28): 14-day observation clock sourced
        // from strategy_versions.updated_at at check/apply time.
        assert_eq!(req.min_days, 14);
        // min_sharpe stays disabled until an executor produces returns data.
        assert_eq!(req.min_sharpe, 0.0);
        
        assert!(pipeline.get_requirements(PromotionStage::Live).is_none());
    }

    #[test]
    fn test_with_gates_overrides_min_days() {
        let pipeline = PromotionPipeline::with_gates(crate::config::PromotionGatesConfig {
            min_days_paper_to_micro: 30,
            min_days_micro_to_live: 60,
        });
        assert_eq!(pipeline.paper_to_micro.min_days, 30);
        assert_eq!(pipeline.micro_to_live.min_days, 60);
        // Candidate→PAPER keeps days=0 regardless of config.
        assert_eq!(pipeline.candidate_to_paper.min_days, 0);
    }

    #[test]
    fn test_live_days_observed_from_updated_at() {
        // Snapshot promoted 10 days (+1h slack) ago → 10 observed days.
        // (+1h so elapsed floors cleanly to 10 full 86400s periods.)
        let ten_days_ago = chrono::Utc::now().timestamp() - (10 * 86_400 + 3600);
        let snap = CandidateSnapshot {
            version_id: "v1".into(),
            equity: "QQQ".into(),
            strategy_family: "m".into(),
            params: HashMap::new(),
            status: CandidateStatus::Paper,
            mean_ic: 0.05,
            std_ic: 0.01,
            n_trades: 10,
            fold_ics: vec![],
            created_at: ten_days_ago,
            updated_at: ten_days_ago,
        };
        assert_eq!(PromotionPipeline::live_days_observed(&snap), 10);

        // Future timestamp (clock skew) → saturates at 0, never negative.
        let mut snap2 = snap.clone();
        snap2.updated_at = chrono::Utc::now().timestamp() + 3600;
        assert_eq!(PromotionPipeline::live_days_observed(&snap2), 0);
    }

    #[tokio::test]
    async fn test_promote_db_backed_pass() {
        use sqlx::sqlite::SqlitePoolOptions;

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

        let store = CandidateStore::new(pool);
        let pipeline = PromotionPipeline::new();

        // Store a candidate
        let version_id = store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.05, 0.01, 150, vec![0.05])
            .await
            .unwrap();

        // Promote with sufficient evidence
        let evidence = PromotionEvidence {
            n_trades: 150,
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![],
        };

        let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
        assert!(result.promoted);
        assert_eq!(result.to_stage, PromotionStage::Paper);

        // Verify status was updated
        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.status, CandidateStatus::Paper);
    }

    #[tokio::test]
    async fn test_promote_db_backed_fail() {
        use sqlx::sqlite::SqlitePoolOptions;

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

        let store = CandidateStore::new(pool);
        let pipeline = PromotionPipeline::new();

        // Store a candidate
        let version_id = store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.05, 0.01, 12, vec![0.05])
            .await
            .unwrap();

        // Attempt to promote with insufficient trades
        let evidence = PromotionEvidence {
            n_trades: 12, // < 100
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![],
        };

        let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
        assert!(!result.promoted);
        assert!(result.reason.contains("Insufficient trades"));

        // Verify status was NOT updated
        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.status, CandidateStatus::New);
    }

    #[tokio::test]
    async fn test_promote_db_backed_not_found() {
        use sqlx::sqlite::SqlitePoolOptions;

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

        let store = CandidateStore::new(pool);
        let pipeline = PromotionPipeline::new();

        // Attempt to promote non-existent candidate
        let evidence = PromotionEvidence {
            n_trades: 150,
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
            fold_ics: vec![],
        };

        let result = pipeline.promote(&store, "nonexistent", &evidence).await.unwrap();
        assert!(!result.promoted);
        assert!(result.reason.contains("not found"));
    }
}
