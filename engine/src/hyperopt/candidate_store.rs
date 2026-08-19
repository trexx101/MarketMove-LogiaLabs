//! Candidate store — versioned parameter snapshots with backtest reports
//!
//! DB-backed storage for optimization candidates. Each candidate gets a
//! unique version ID and stores parameters + backtest metrics, scoped by equity.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;

use crate::db::DbPool;

/// Candidate status in the store
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateStatus {
    /// Newly optimized, not yet validated
    New,
    /// Passed stability check
    Stable,
    /// Failed stability check
    Unstable,
    /// Running in paper mode
    Paper,
    /// Running in micro mode
    Micro,
    /// Promoted to live
    Live,
    /// Retired (replaced or underperformed)
    Retired,
}

impl CandidateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateStatus::New => "NEW",
            CandidateStatus::Stable => "STABLE",
            CandidateStatus::Unstable => "UNSTABLE",
            CandidateStatus::Paper => "PAPER",
            CandidateStatus::Micro => "MICRO",
            CandidateStatus::Live => "LIVE",
            CandidateStatus::Retired => "RETIRED",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "NEW" => Some(CandidateStatus::New),
            "STABLE" => Some(CandidateStatus::Stable),
            "UNSTABLE" => Some(CandidateStatus::Unstable),
            "PAPER" => Some(CandidateStatus::Paper),
            "MICRO" => Some(CandidateStatus::Micro),
            "LIVE" => Some(CandidateStatus::Live),
            "RETIRED" => Some(CandidateStatus::Retired),
            _ => None,
        }
    }
}

/// Candidate snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSnapshot {
    pub version_id: String,
    pub equity: String,
    pub strategy_family: String,
    pub params: HashMap<String, f64>,
    pub status: CandidateStatus,
    pub mean_ic: f64,
    pub std_ic: f64,
    pub n_trades: usize,
    pub fold_ics: Vec<f64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Candidate store (DB-backed)
pub struct CandidateStore {
    pool: DbPool,
    counter: std::sync::atomic::AtomicU64,
}

impl CandidateStore {
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Store a new candidate
    pub async fn store(
        &self,
        equity: &str,
        strategy_family: &str,
        params: HashMap<String, f64>,
        mean_ic: f64,
        std_ic: f64,
        n_trades: usize,
        fold_ics: Vec<f64>,
    ) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let seq = self.counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let version_id = format!("v{}_{}_{}", equity, now, seq);

        let params_json = serde_json::to_string(&params)?;
        let fold_ics_json = serde_json::to_string(&fold_ics)?;
        let promotion_metadata = serde_json::json!({
            "mean_ic": mean_ic,
            "std_ic": std_ic,
            "n_trades": n_trades,
            "fold_ics": fold_ics,
            "strategy_family": strategy_family,
        });
        let promotion_metadata_json = serde_json::to_string(&promotion_metadata)?;

        sqlx::query(
            r#"
            INSERT INTO strategy_versions (id, equity, family, params_json, status, promotion_metadata_json, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&version_id)
        .bind(equity)
        .bind(strategy_family)
        .bind(&params_json)
        .bind(CandidateStatus::New.as_str())
        .bind(&promotion_metadata_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("inserting candidate")?;

        Ok(version_id)
    }

    /// Get candidate by version ID
    pub async fn get(&self, version_id: &str) -> Result<Option<CandidateSnapshot>> {
        let row = sqlx::query(
            r#"
            SELECT id, equity, family, params_json, status, promotion_metadata_json, created_at, updated_at
            FROM strategy_versions
            WHERE id = ?
            "#,
        )
        .bind(version_id)
        .fetch_optional(&self.pool)
        .await
        .context("fetching candidate")?;

        match row {
            Some(row) => Ok(Some(self.row_to_snapshot(row)?)),
            None => Ok(None),
        }
    }

    /// Update candidate status
    pub async fn update_status(&self, version_id: &str, status: CandidateStatus) -> Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let result = sqlx::query(
            r#"
            UPDATE strategy_versions
            SET status = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status.as_str())
        .bind(now)
        .bind(version_id)
        .execute(&self.pool)
        .await
        .context("updating candidate status")?;

        Ok(result.rows_affected() > 0)
    }

    /// List all candidates
    pub async fn list(&self) -> Result<Vec<CandidateSnapshot>> {
        let rows = sqlx::query(
            r#"
            SELECT id, equity, family, params_json, status, promotion_metadata_json, created_at, updated_at
            FROM strategy_versions
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("listing candidates")?;

        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(self.row_to_snapshot(row)?);
        }
        Ok(snapshots)
    }

    /// List candidates by equity
    pub async fn list_by_equity(&self, equity: &str) -> Result<Vec<CandidateSnapshot>> {
        let rows = sqlx::query(
            r#"
            SELECT id, equity, family, params_json, status, promotion_metadata_json, created_at, updated_at
            FROM strategy_versions
            WHERE equity = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(equity)
        .fetch_all(&self.pool)
        .await
        .context("listing candidates by equity")?;

        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(self.row_to_snapshot(row)?);
        }
        Ok(snapshots)
    }

    /// List candidates by status
    pub async fn list_by_status(&self, status: CandidateStatus) -> Result<Vec<CandidateSnapshot>> {
        let rows = sqlx::query(
            r#"
            SELECT id, equity, family, params_json, status, promotion_metadata_json, created_at, updated_at
            FROM strategy_versions
            WHERE status = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await
        .context("listing candidates by status")?;

        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(self.row_to_snapshot(row)?);
        }
        Ok(snapshots)
    }

    /// Get best candidate by mean IC for a given equity
    pub async fn get_best(&self, equity: &str) -> Result<Option<CandidateSnapshot>> {
        let row = sqlx::query(
            r#"
            SELECT id, equity, family, params_json, status, promotion_metadata_json, created_at, updated_at
            FROM strategy_versions
            WHERE equity = ?
            ORDER BY json_extract(promotion_metadata_json, '$.mean_ic') DESC
            LIMIT 1
            "#,
        )
        .bind(equity)
        .fetch_optional(&self.pool)
        .await
        .context("fetching best candidate")?;

        match row {
            Some(row) => Ok(Some(self.row_to_snapshot(row)?)),
            None => Ok(None),
        }
    }

    /// Convert DB row to CandidateSnapshot
    fn row_to_snapshot(&self, row: sqlx::sqlite::SqliteRow) -> Result<CandidateSnapshot> {
        let version_id: String = row.get("id");
        let equity: String = row.get("equity");
        let strategy_family: String = row.get("family");
        let params_json: String = row.get("params_json");
        let status_str: String = row.get("status");
        let promotion_metadata_json: String = row.get("promotion_metadata_json");
        let created_at: i64 = row.get("created_at");
        let updated_at: i64 = row.get("updated_at");

        let params: HashMap<String, f64> = serde_json::from_str(&params_json)?;
        let promotion_metadata: serde_json::Value = serde_json::from_str(&promotion_metadata_json)?;

        let mean_ic = promotion_metadata["mean_ic"].as_f64().unwrap_or(0.0);
        let std_ic = promotion_metadata["std_ic"].as_f64().unwrap_or(0.0);
        let n_trades = promotion_metadata["n_trades"].as_u64().unwrap_or(0) as usize;
        let fold_ics: Vec<f64> = promotion_metadata["fold_ics"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();

        let status = CandidateStatus::from_str(&status_str).unwrap_or(CandidateStatus::New);

        Ok(CandidateSnapshot {
            version_id,
            equity,
            strategy_family,
            params,
            status,
            mean_ic,
            std_ic,
            n_trades,
            fold_ics,
            created_at,
            updated_at,
        })
    }
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

        pool
    }

    #[tokio::test]
    async fn test_store_candidate() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool);

        let mut params = HashMap::new();
        params.insert("threshold".to_string(), 0.5);

        let version_id = store
            .store("QQQ", "ema_macd_breakout", params.clone(), 0.05, 0.01, 150, vec![0.04, 0.05, 0.06])
            .await
            .unwrap();

        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.equity, "QQQ");
        assert_eq!(snapshot.strategy_family, "ema_macd_breakout");
        assert_eq!(snapshot.params, params);
        assert_eq!(snapshot.mean_ic, 0.05);
        assert_eq!(snapshot.status, CandidateStatus::New);
    }

    #[tokio::test]
    async fn test_update_status() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool);

        let version_id = store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.05, 0.01, 150, vec![0.05])
            .await
            .unwrap();

        assert!(store.update_status(&version_id, CandidateStatus::Stable).await.unwrap());

        let snapshot = store.get(&version_id).await.unwrap().unwrap();
        assert_eq!(snapshot.status, CandidateStatus::Stable);
    }

    #[tokio::test]
    async fn test_list_by_equity() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool);

        store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.05, 0.01, 150, vec![0.05])
            .await
            .unwrap();
        store
            .store("SMH", "ema_macd_breakout", HashMap::new(), 0.06, 0.01, 150, vec![0.06])
            .await
            .unwrap();
        store
            .store("QQQ", "ichimoku", HashMap::new(), 0.07, 0.01, 150, vec![0.07])
            .await
            .unwrap();

        let qqq_candidates = store.list_by_equity("QQQ").await.unwrap();
        assert_eq!(qqq_candidates.len(), 2);

        let smh_candidates = store.list_by_equity("SMH").await.unwrap();
        assert_eq!(smh_candidates.len(), 1);
    }

    #[tokio::test]
    async fn test_get_best() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool);

        store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.05, 0.01, 150, vec![0.05])
            .await
            .unwrap();
        store
            .store("QQQ", "ichimoku", HashMap::new(), 0.08, 0.01, 150, vec![0.08])
            .await
            .unwrap();
        store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.06, 0.01, 150, vec![0.06])
            .await
            .unwrap();

        let best = store.get_best("QQQ").await.unwrap().unwrap();
        assert_eq!(best.mean_ic, 0.08);
    }

    #[tokio::test]
    async fn test_list_by_status() {
        let pool = test_pool().await;
        let store = CandidateStore::new(pool);

        let id1 = store
            .store("QQQ", "ema_macd_breakout", HashMap::new(), 0.05, 0.01, 150, vec![0.05])
            .await
            .unwrap();
        store
            .store("QQQ", "ichimoku", HashMap::new(), 0.06, 0.01, 150, vec![0.06])
            .await
            .unwrap();

        store.update_status(&id1, CandidateStatus::Stable).await.unwrap();

        let new_candidates = store.list_by_status(CandidateStatus::New).await.unwrap();
        assert_eq!(new_candidates.len(), 1);

        let stable_candidates = store.list_by_status(CandidateStatus::Stable).await.unwrap();
        assert_eq!(stable_candidates.len(), 1);
    }
}
