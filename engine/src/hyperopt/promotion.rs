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
#[derive(Debug, Clone)]
pub struct PromotionEvidence {
    pub n_trades: usize,
    pub ic: f64,
    pub sharpe: f64,
    pub days_observed: usize,
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
        Self {
            candidate_to_paper: GateRequirement {
                min_trades: 100,
                min_ic: 0.03,
                min_sharpe: 1.0,
                min_days: 0,
            },
            paper_to_micro: GateRequirement {
                min_trades: 30,
                min_ic: 0.03,
                min_sharpe: 1.0,
                min_days: 14,
            },
            micro_to_live: GateRequirement {
                min_trades: 50,
                min_ic: 0.04,
                min_sharpe: 1.5,
                min_days: 30,
            },
        }
    }
}

impl PromotionPipeline {
    pub fn new() -> Self {
        Self::default()
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

    /// Promote a candidate (DB-backed)
    ///
    /// Reads evidence from the candidate snapshot, checks gates, and updates status if promotion passes.
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

        let current_stage = match snapshot.status {
            CandidateStatus::New | CandidateStatus::Stable => PromotionStage::Candidate,
            CandidateStatus::Paper => PromotionStage::Paper,
            CandidateStatus::Micro => PromotionStage::Micro,
            CandidateStatus::Live => PromotionStage::Live,
            CandidateStatus::Unstable | CandidateStatus::Retired => PromotionStage::Rejected,
        };

        let result = self.check_promotion(current_stage, evidence);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_promote_candidate_to_paper_pass() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 150,
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 0,
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
        };

        let result = pipeline.check_promotion(PromotionStage::Candidate, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("IC too low"));
    }

    #[test]
    fn test_promote_paper_to_micro_fail_days() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 50,
            ic: 0.05,
            sharpe: 1.5,
            days_observed: 7, // < 14
        };

        let result = pipeline.check_promotion(PromotionStage::Paper, &evidence);
        assert!(!result.promoted);
        assert!(result.reason.contains("Insufficient observation days"));
    }

    #[test]
    fn test_promote_micro_to_live_pass() {
        let pipeline = PromotionPipeline::new();
        let evidence = PromotionEvidence {
            n_trades: 60,
            ic: 0.05,
            sharpe: 2.0,
            days_observed: 45,
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
        assert_eq!(req.min_days, 14);
        
        assert!(pipeline.get_requirements(PromotionStage::Live).is_none());
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
        };

        let result = pipeline.promote(&store, "nonexistent", &evidence).await.unwrap();
        assert!(!result.promoted);
        assert!(result.reason.contains("not found"));
    }
}
