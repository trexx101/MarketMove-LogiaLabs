//! Write-ahead intent log for options trading
//!
//! Persists stage transitions BEFORE every order send/modification.
//! One row per transition. Enables crash recovery and audit trail.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Exit stage for intent logging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[serde(rename_all = "snake_case")]
pub enum ExitStage {
    Stage1,
    Stage2,
    Stage3,
    Complete,
}

impl std::fmt::Display for ExitStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitStage::Stage1 => write!(f, "stage_1"),
            ExitStage::Stage2 => write!(f, "stage_2"),
            ExitStage::Stage3 => write!(f, "stage_3"),
            ExitStage::Complete => write!(f, "complete"),
        }
    }
}

/// Intent log entry
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct IntentLogEntry {
    pub id: i64,
    pub position_id: i64,
    pub stage: String,
    pub order_id: Option<String>,
    pub limit_price: f64,
    pub quantity: f64,
    pub timestamp: DateTime<Utc>,
}

/// Write-ahead intent logger
pub struct IntentLogger {
    pool: sqlx::SqlitePool,
}

impl IntentLogger {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    /// Log intent BEFORE sending order
    pub async fn log_intent(
        &self,
        position_id: i64,
        stage: ExitStage,
        limit_price: f64,
        quantity: f64,
    ) -> Result<i64, sqlx::Error> {
        let stage_str = stage.to_string();
        let result = sqlx::query(
            r#"
            INSERT INTO exit_intent_log (position_id, stage, limit_price, quantity, timestamp)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(position_id)
        .bind(&stage_str)
        .bind(limit_price)
        .bind(quantity)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Update intent with order ID after order is sent
    pub async fn update_order_id(&self, intent_id: i64, order_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE exit_intent_log
            SET order_id = ?
            WHERE id = ?
            "#,
        )
        .bind(order_id)
        .bind(intent_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get latest intent for a position
    pub async fn get_latest_intent(&self, position_id: i64) -> Result<Option<IntentLogEntry>, sqlx::Error> {
        let entry = sqlx::query_as::<_, IntentLogEntry>(
            r#"
            SELECT id, position_id, stage, order_id, limit_price, quantity, timestamp
            FROM exit_intent_log
            WHERE position_id = ?
            ORDER BY timestamp DESC
            LIMIT 1
            "#,
        )
        .bind(position_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(entry)
    }

    /// Get all intents for a position
    pub async fn get_position_intents(&self, position_id: i64) -> Result<Vec<IntentLogEntry>, sqlx::Error> {
        let entries = sqlx::query_as::<_, IntentLogEntry>(
            r#"
            SELECT id, position_id, stage, order_id, limit_price, quantity, timestamp
            FROM exit_intent_log
            WHERE position_id = ?
            ORDER BY timestamp ASC
            "#,
        )
        .bind(position_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_stage_display() {
        assert_eq!(ExitStage::Stage1.to_string(), "stage_1");
        assert_eq!(ExitStage::Stage2.to_string(), "stage_2");
        assert_eq!(ExitStage::Stage3.to_string(), "stage_3");
        assert_eq!(ExitStage::Complete.to_string(), "complete");
    }

    #[test]
    fn test_exit_stage_serialization() {
        let stage = ExitStage::Stage2;
        let json = serde_json::to_string(&stage).unwrap();
        assert_eq!(json, "\"stage2\"");

        let deserialized: ExitStage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ExitStage::Stage2);
    }
}
