//! Options paper executor
//!
//! Extends paper execution semantics to options: uses the staged exit ladder
//! and fills against observed bid/ask from the tape.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::info;

use crate::options::staged_ladder::{ExitStage, StagedExitLadder};

/// Paper executor for options positions
pub struct OptionsPaperExecutor {
    pool: SqlitePool,
    /// Current staged ladder for active exit
    current_ladder: Option<StagedExitLadder>,
}

impl OptionsPaperExecutor {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            current_ladder: None,
        }
    }

    /// Initiate an exit using the staged ladder
    pub async fn initiate_exit(
        &mut self,
        position_id: i64,
        current_bid: f64,
        tick_size: f64,
    ) -> Result<ExitStage> {
        let mut ladder = StagedExitLadder::new(position_id);
        ladder.start_stage_1(current_bid, tick_size);

        self.current_ladder = Some(ladder.clone());

        info!(
            position_id = position_id,
            stage = ?ladder.current_stage(),
            limit_price = ladder.current_limit_price(),
            "initiated staged exit"
        );

        Ok(ladder.current_stage())
    }

    /// Attempt to fill at the current ladder price
    pub async fn try_fill(
        &mut self,
        position_id: i64,
        observed_bid: f64,
        observed_ask: f64,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<FillResult>> {
        let ladder = match &self.current_ladder {
            Some(l) if l.position_id() == position_id => l,
            _ => return Ok(None), // No active exit for this position
        };

        let limit_price = ladder.current_limit_price();
        let stage = ladder.current_stage();

        // Check if we can fill: observed bid must be >= limit price
        let can_fill = observed_bid >= limit_price;

        if !can_fill {
            // Check if we should advance to next stage
            if ladder.should_advance(timestamp) {
                self.advance_ladder(position_id, observed_bid)?;
            }
            return Ok(None);
        }

        // Fill at the limit price (or observed bid if better)
        let fill_price = observed_bid.min(limit_price);

        let fill = FillResult {
            position_id,
            stage,
            price: fill_price,
            quantity: 1.0, // TODO: track actual position size
            timestamp,
        };

        // Record the fill
        self.record_fill(&fill).await?;

        // Clear the ladder
        self.current_ladder = None;

        info!(
            position_id = position_id,
            stage = ?stage,
            price = fill_price,
            "filled exit"
        );

        Ok(Some(fill))
    }

    /// Advance the ladder to the next stage
    fn advance_ladder(
        &mut self,
        position_id: i64,
        current_bid: f64,
    ) -> Result<ExitStage> {
        let ladder = match &mut self.current_ladder {
            Some(l) if l.position_id() == position_id => l,
            _ => return Ok(ExitStage::Complete),
        };

        ladder.advance(current_bid);

        info!(
            position_id = position_id,
            stage = ?ladder.current_stage(),
            limit_price = ladder.current_limit_price(),
            "advanced exit ladder"
        );

        Ok(ladder.current_stage())
    }

    /// Record a fill in the database
    async fn record_fill(&self, fill: &FillResult) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO option_fills (
                position_id, stage, price, quantity, timestamp
            ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(fill.position_id)
        .bind(format!("{:?}", fill.stage))
        .bind(fill.price)
        .bind(fill.quantity)
        .bind(fill.timestamp.timestamp())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get the current ladder state for a position
    pub fn get_ladder(&self, position_id: i64) -> Option<&StagedExitLadder> {
        self.current_ladder
            .as_ref()
            .filter(|l| l.position_id() == position_id)
    }

    /// Cancel an active exit
    pub fn cancel_exit(&mut self, position_id: i64) {
        if let Some(ladder) = &self.current_ladder {
            if ladder.position_id() == position_id {
                self.current_ladder = None;
                info!(position_id = position_id, "cancelled exit");
            }
        }
    }
}

/// Result of filling an option exit
#[derive(Debug, Clone)]
pub struct FillResult {
    pub position_id: i64,
    pub stage: ExitStage,
    pub price: f64,
    pub quantity: f64,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        // Create tables
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS option_fills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                position_id INTEGER NOT NULL,
                stage TEXT NOT NULL,
                price REAL NOT NULL,
                quantity REAL NOT NULL,
                timestamp INTEGER NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[tokio::test]
    async fn initiate_exit_creates_ladder() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        let stage = executor.initiate_exit(1, 5.0, 0.05).await.unwrap();

        assert_eq!(stage, ExitStage::Stage1);
        assert!(executor.get_ladder(1).is_some());
    }

    #[tokio::test]
    async fn fill_at_stage1_when_bid_meets_limit() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        executor.initiate_exit(1, 5.0, 0.05).await.unwrap();

        // Stage 1 limit = 5.0 + 2*0.05 = 5.10
        // Observed bid = 5.15 (meets limit)
        let now = Utc::now();
        let fill = executor.try_fill(1, 5.15, 5.20, now).await.unwrap();

        assert!(fill.is_some());
        let fill = fill.unwrap();
        assert_eq!(fill.stage, ExitStage::Stage1);
        assert_eq!(fill.price, 5.10); // Filled at limit
        assert!(executor.get_ladder(1).is_none()); // Ladder cleared
    }

    #[tokio::test]
    async fn no_fill_when_bid_below_limit() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        executor.initiate_exit(1, 5.0, 0.05).await.unwrap();

        // Stage 1 limit = 5.10
        // Observed bid = 5.05 (below limit)
        let now = Utc::now();
        let fill = executor.try_fill(1, 5.05, 5.10, now).await.unwrap();

        assert!(fill.is_none());
        assert!(executor.get_ladder(1).is_some()); // Ladder still active
    }

    #[tokio::test]
    async fn cancel_exit_clears_ladder() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        executor.initiate_exit(1, 5.0, 0.05).await.unwrap();
        assert!(executor.get_ladder(1).is_some());

        executor.cancel_exit(1);
        assert!(executor.get_ladder(1).is_none());
    }
}
