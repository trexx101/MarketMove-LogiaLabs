//! Options paper executor
//!
//! Extends paper execution semantics to options: entry creates a real
//! `option_positions` row + `option_fills` row + a lifecycle event;
//! exits use the staged ladder and fill against observed bid/ask from
//! the tape.
//!
//! All `position_id` parameters are the UUID TEXT primary key of
//! `option_positions` (not an integer).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::Row;
use tracing::info;

use crate::db::DbPool;
use crate::options::staged_ladder::{ExitStage, StagedExitLadder};

/// Paper executor for options positions
pub struct OptionsPaperExecutor {
    pool: DbPool,
    /// Current staged ladder for active exit
    current_ladder: Option<StagedExitLadder>,
}

/// Parameters for a paper entry (B1).
#[derive(Debug, Clone)]
pub struct EntryEntryParams {
    /// Underlying as stored by the tape recorder (e.g. "US.QQQ").
    pub underlying: String,
    /// Contract code from the tape (e.g. "QQQ  260930C 62500").
    pub contract_code: String,
    /// Strategy version that triggered the entry (NOT NULL in DDL).
    pub strategy_version_id: String,
    /// Last close of the underlying at entry time.
    pub entry_underlying_price: f64,
    /// Ask (per-contract premium).
    pub ask: f64,
    /// Bid (per-contract).
    pub bid: f64,
    /// Contract count from the position sizer.
    pub contracts: u32,
    /// Delta at selection time.
    pub delta: f64,
    /// Days to expiry at entry.
    pub dte: i64,
    /// Slippage budget (per-contract).
    pub slippage_budget: f64,
}

/// Outcome of a paper entry attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryOutcome {
    /// Position created, ENTRY fill recorded, event published.
    Opened {
        position_id: String,
        /// Per-contract fill price (the ask).
        fill_price: f64,
    },
    /// Entry was skipped without side effects.
    Skipped(EntrySkipReason),
}

/// Reason an entry was skipped.
#[derive(Debug, Clone, PartialEq)]
pub enum EntrySkipReason {
    /// One open position per underlying — an OPEN position already exists.
    DuplicateOpenPosition {
        existing_position_id: String,
    },
}

impl OptionsPaperExecutor {
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            current_ladder: None,
        }
    }

    /// Paper entry: atomically creates one OPEN `option_positions` row,
    /// one ENTRY `option_fills` row, and one `options::position_opened`
    /// event.
    ///
    /// Paper semantics: the 2-stage entry ladder is a no-op — the order
    /// fills immediately at the ask (plan risk #3: ladder state is
    /// in-memory and only matters for live limit orders in Phase D).
    ///
    /// Returns `Skipped(DuplicateOpenPosition)` if the underlying already
    /// has an OPEN position (one position per underlying per plan risk #4).
    /// Returns an error if `strategy_version_id` is empty (defends the
    /// NOT NULL attribution chain that Phase E builds on).
    pub async fn initiate_entry(
        &self,
        p: &EntryEntryParams,
    ) -> Result<EntryOutcome> {
        // Guard: one open position per underlying.
        let existing: Option<(String,)> =
            sqlx::query_as(
                "SELECT id FROM option_positions \
                 WHERE underlying = ?1 AND status = 'OPEN' LIMIT 1",
            )
            .bind(&p.underlying)
            .fetch_optional(&self.pool)
            .await
            .context("initiate_entry: duplicate check")?;
        if let Some((id,)) = existing {
            return Ok(EntryOutcome::Skipped(EntrySkipReason::DuplicateOpenPosition {
                existing_position_id: id,
            }));
        }

        // Guard: strategy attribution must be present (NOT NULL in DDL,
        // and the only link between a position and the promotion gate).
        if p.strategy_version_id.trim().is_empty() {
            anyhow::bail!(
                "initiate_entry: strategy_version_id is required (got empty) for underlying {}",
                p.underlying
            );
        }

        let position_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        // Per-contract premium = ask; total premium = ask * 100 * qty
        // (computed at PnL time, see exit pipeline).
        let entry_premium = p.ask;
        let entry_spread = p.ask - p.bid;

        sqlx::query(
            r#"
            INSERT INTO option_positions (
                id, underlying, contract_code, strategy_version_id,
                entry_underlying_price, entry_premium, entry_spread,
                entry_slippage_budget, qty, qty_filled_residual, status,
                dte_at_entry, delta_at_entry, realized_pnl, closed_at,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'OPEN', ?, ?, NULL, NULL, ?, ?)
            "#,
        )
        .bind(&position_id)
        .bind(&p.underlying)
        .bind(&p.contract_code)
        .bind(&p.strategy_version_id)
        .bind(p.entry_underlying_price)
        .bind(entry_premium)
        .bind(entry_spread)
        .bind(p.slippage_budget)
        .bind(p.contracts as i64)
        .bind(p.contracts as i64)
        .bind(p.dte)
        .bind(p.delta)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("initiate_entry: option_positions insert")?;

        sqlx::query(
            r#"
            INSERT INTO option_fills (
                position_id, stage, price, quantity, timestamp
            ) VALUES (?, 'ENTRY', ?, ?, ?)
            "#,
        )
        .bind(&position_id)
        .bind(p.ask)
        .bind(p.contracts as f64)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("initiate_entry: option_fills insert")?;

        // Lifecycle event so the position is visible in the Events tab.
        let payload = serde_json::json!({
            "position_id": position_id,
            "contract_code": p.contract_code,
            "strategy_version_id": p.strategy_version_id,
            "underlying": p.underlying,
            "delta": p.delta,
            "dte": p.dte,
            "ask": p.ask,
            "contracts": p.contracts,
        });
        let _ = crate::db::insert_event(
            &self.pool,
            "trade",
            "info",
            "paper",
            "options::position_opened",
            &format!(
                "POSITION_OPENED {}: {} @ {:.2} x{} (delta {:.2}, dte {})",
                p.underlying, p.contract_code, p.ask, p.contracts, p.delta, p.dte
            ),
            &payload.to_string(),
            Some(&p.underlying),
        )
        .await;

        info!(
            %position_id,
            underlying = %p.underlying,
            contract = %p.contract_code,
            ask = p.ask,
            contracts = p.contracts,
            "paper entry filled"
        );

        Ok(EntryOutcome::Opened {
            position_id,
            fill_price: p.ask,
        })
    }

    /// Initiate an exit using the staged ladder
    pub async fn initiate_exit(
        &mut self,
        position_id: &str,
        current_bid: f64,
        tick_size: f64,
    ) -> Result<ExitStage> {
        let mut ladder = StagedExitLadder::new(position_id);
        ladder.start_stage_1(current_bid, tick_size);

        self.current_ladder = Some(ladder.clone());

        info!(
            %position_id,
            stage = ?ladder.current_stage(),
            limit_price = ladder.current_limit_price(),
            "initiated staged exit"
        );

        Ok(ladder.current_stage())
    }

    /// Attempt to fill at the current ladder price
    pub async fn try_fill(
        &mut self,
        position_id: &str,
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
            position_id: position_id.to_string(),
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
            %position_id,
            stage = ?stage,
            price = fill_price,
            "filled exit"
        );

        Ok(Some(fill))
    }

    /// Advance the ladder to the next stage
    pub fn advance_ladder(
        &mut self,
        position_id: &str,
        current_bid: f64,
    ) -> Result<ExitStage> {
        let ladder = match &mut self.current_ladder {
            Some(l) if l.position_id() == position_id => l,
            _ => return Ok(ExitStage::Complete),
        };

        ladder.advance(current_bid);

        info!(
            %position_id,
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
        .bind(&fill.position_id)
        .bind(format!("{:?}", fill.stage))
        .bind(fill.price)
        .bind(fill.quantity)
        .bind(fill.timestamp.timestamp())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get the current ladder state for a position
    pub fn get_ladder(&self, position_id: &str) -> Option<&StagedExitLadder> {
        self.current_ladder
            .as_ref()
            .filter(|l| l.position_id() == position_id)
    }

    /// Cancel an active exit
    pub fn cancel_exit(&mut self, position_id: &str) {
        if let Some(ladder) = &self.current_ladder {
            if ladder.position_id() == position_id {
                self.current_ladder = None;
                info!(%position_id, "cancelled exit");
            }
        }
    }
}

/// Result of filling an option exit
#[derive(Debug, Clone)]
pub struct FillResult {
    pub position_id: String,
    pub stage: ExitStage,
    pub price: f64,
    pub quantity: f64,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        // Create tables — mirrors production DDL in db.rs
        // (option_fills.position_id is TEXT; it matches the
        // option_positions UUID primary key).
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS option_positions (
                id TEXT PRIMARY KEY,
                underlying TEXT NOT NULL,
                contract_code TEXT NOT NULL,
                strategy_version_id TEXT NOT NULL,
                entry_underlying_price REAL NOT NULL,
                entry_premium REAL NOT NULL DEFAULT 0.0,
                entry_spread REAL NOT NULL,
                entry_slippage_budget REAL NOT NULL,
                qty INTEGER NOT NULL,
                qty_filled_residual INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'OPEN',
                dte_at_entry INTEGER NOT NULL,
                delta_at_entry REAL NOT NULL,
                realized_pnl REAL,
                closed_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS option_fills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                position_id TEXT NOT NULL,
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

    fn entry_params() -> EntryEntryParams {
        EntryEntryParams {
            underlying: "US.QQQ".to_string(),
            contract_code: "QQQ  260930C 62500".to_string(),
            strategy_version_id: "sv-test".to_string(),
            entry_underlying_price: 620.0,
            ask: 8.5,
            bid: 8.25,
            contracts: 2,
            delta: 0.45,
            dte: 33,
            slippage_budget: 0.01,
        }
    }

    // ── initiate_entry ──────────────────────────────────────────────

    #[tokio::test]
    async fn initiate_entry_creates_position_and_fill() {
        let pool = test_pool().await;
        let executor = OptionsPaperExecutor::new(pool.clone());

        let outcome = executor
            .initiate_entry(&entry_params())
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
        let parsed = uuid::Uuid::parse_str(&position_id).expect("position id is a UUID");
        assert_eq!(parsed.get_variant(), uuid::Variant::RFC4122);

        // option_positions row: every field populated
        let row: (
            String,
            String,
            String,
            f64,
            f64,
            f64,
            i64,
            i64,
            String,
            i64,
            f64,
        ) = sqlx::query_as(
            "SELECT id, underlying, contract_code, entry_underlying_price, \
             entry_premium, entry_spread, qty, qty_filled_residual, status, \
             dte_at_entry, delta_at_entry FROM option_positions",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, position_id);
        assert_eq!(row.1, "US.QQQ");
        assert_eq!(row.2, "QQQ  260930C 62500");
        assert_eq!(row.3, 620.0);
        assert_eq!(row.4, 8.5); // per-contract premium = ask
        assert_eq!(row.5, 0.25); // spread = ask - bid
        assert_eq!(row.6, 2);
        assert_eq!(row.7, 2); // paper fills immediately
        assert_eq!(row.8, "OPEN");
        assert_eq!(row.9, 33);
        assert_eq!(row.10, 0.45);

        // option_fills row: ENTRY at the ask
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

        // lifecycle event published
        let event: (String, String) = sqlx::query_as(
            "SELECT source, message FROM engine_events WHERE source = 'options::position_opened'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(event.1.contains("POSITION_OPENED US.QQQ"));
    }

    #[tokio::test]
    async fn initiate_entry_skips_when_open_position_exists() {
        let pool = test_pool().await;
        let executor = OptionsPaperExecutor::new(pool.clone());

        // Seed an existing OPEN position for the same underlying
        sqlx::query(
            "INSERT INTO option_positions (id, underlying, contract_code, strategy_version_id, \
             entry_underlying_price, entry_premium, entry_spread, entry_slippage_budget, \
             qty, qty_filled_residual, status, dte_at_entry, delta_at_entry, created_at, updated_at) \
             VALUES ('existing-1', 'US.QQQ', 'QQQ  260918C 62000', 'sv-x', \
             610.0, 7.0, 0.2, 0.01, 1, 1, 'OPEN', 33, 0.45, 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = executor
            .initiate_entry(&entry_params())
            .await
            .unwrap();

        assert_eq!(
            outcome,
            EntryOutcome::Skipped(EntrySkipReason::DuplicateOpenPosition {
                existing_position_id: "existing-1".to_string(),
            })
        );

        // No new rows
        let n_pos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM option_positions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let n_fills: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM option_fills")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(n_pos, 1);
        assert_eq!(n_fills, 0);
    }

    #[tokio::test]
    async fn initiate_entry_rejects_empty_strategy_version_id() {
        let pool = test_pool().await;
        let executor = OptionsPaperExecutor::new(pool.clone());

        let mut params = entry_params();
        params.strategy_version_id = String::new();

        let result = executor.initiate_entry(&params).await;
        assert!(
            result.is_err(),
            "empty strategy_version_id must be rejected"
        );

        let n_pos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM option_positions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n_pos, 0);
    }

    // ── staged exit ladder (pre-existing, migrated to string ids) ──

    #[tokio::test]
    async fn initiate_exit_creates_ladder() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        let stage = executor
            .initiate_exit("pos-1", 5.0, 0.05)
            .await
            .unwrap();

        assert_eq!(stage, ExitStage::Stage1);
        assert!(executor.get_ladder("pos-1").is_some());
    }

    #[tokio::test]
    async fn fill_at_stage1_when_bid_meets_limit() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        executor.initiate_exit("pos-1", 5.0, 0.05).await.unwrap();

        // Stage 1 limit = 5.0 + 2*0.05 = 5.10
        // Observed bid = 5.15 (meets limit)
        let now = Utc::now();
        let fill = executor
            .try_fill("pos-1", 5.15, 5.20, now)
            .await
            .unwrap();

        assert!(fill.is_some());
        let fill = fill.unwrap();
        assert_eq!(fill.stage, ExitStage::Stage1);
        assert_eq!(fill.price, 5.10); // Filled at limit
        assert_eq!(fill.position_id, "pos-1");
        assert!(executor.get_ladder("pos-1").is_none()); // Ladder cleared
    }

    #[tokio::test]
    async fn no_fill_when_bid_below_limit() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        executor.initiate_exit("pos-1", 5.0, 0.05).await.unwrap();

        // Stage 1 limit = 5.10
        // Observed bid = 5.05 (below limit)
        let now = Utc::now();
        let fill = executor
            .try_fill("pos-1", 5.05, 5.10, now)
            .await
            .unwrap();

        assert!(fill.is_none());
        assert!(executor.get_ladder("pos-1").is_some()); // Ladder still active
    }

    #[tokio::test]
    async fn cancel_exit_clears_ladder() {
        let pool = test_pool().await;
        let mut executor = OptionsPaperExecutor::new(pool);

        executor.initiate_exit("pos-1", 5.0, 0.05).await.unwrap();
        assert!(executor.get_ladder("pos-1").is_some());

        executor.cancel_exit("pos-1");
        assert!(executor.get_ladder("pos-1").is_none());
    }
}
