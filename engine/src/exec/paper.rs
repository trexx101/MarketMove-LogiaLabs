use anyhow::Result;
use tracing::info;

use crate::api::ws::{TelemetryEvent, TelemetrySender};
use crate::db::{self, DbPool};
use crate::exec::{FillResult, TradeSide};
use crate::strategy::Position;

/// Default inverse-ETF symbol used to express a short position in QQQ.
/// PSQ (ProShares Short QQQ) tracks -1x the Nasdaq-100, so buying PSQ
/// approximates a short without borrowing/locating shares.
pub const DEFAULT_SHORT_SYMBOL: &str = "PSQ";

pub struct PaperExecutor {
    pool: DbPool,
    fee_rate: f64,
    /// Symbol traded for long positions (e.g. "QQQ").
    primary_symbol: String,
    /// Symbol traded for short positions via the inverse ETF (e.g. "PSQ").
    short_symbol: String,
    /// Model id (registry UUID, or "bootstrap-default") for §8 telemetry attribution.
    model_id: String,
    /// Canonical "PRIMARY/INVERSE" label, e.g. "QQQ/PSQ".
    pair: String,
    current_position: Position,
    entry_price: f64,
    qty: f64,
    tx: Option<TelemetrySender>,
}

impl PaperExecutor {
    /// Construct a paper executor for a single primary symbol (no short symbol).
    ///
    /// Shorts (if ever produced by the strategy) are recorded against the
    /// primary symbol. Prefer [`PaperExecutor::new_for_symbol`] when shorting is
    /// enabled so the inverse ETF symbol is attributed correctly.
    pub fn new(pool: DbPool, fee_rate: f64, tx: Option<TelemetrySender>) -> Self {
        Self::new_for_symbol(pool, fee_rate, "QQQ", DEFAULT_SHORT_SYMBOL, tx)
    }

    /// Construct a paper executor with explicit primary and short (inverse ETF)
    /// symbols. `short_symbol` is the instrument recorded/“bought” when the
    /// strategy targets `Position::Short`.
    pub fn new_for_symbol(
        pool: DbPool,
        fee_rate: f64,
        primary_symbol: &str,
        short_symbol: &str,
        tx: Option<TelemetrySender>,
    ) -> Self {
        Self {
            pool,
            fee_rate,
            primary_symbol: primary_symbol.to_string(),
            short_symbol: short_symbol.to_string(),
            // §8 back-compat: existing callers (legacy single-model tests,
            // bootstrap path) don't supply model_id/pair. Stamp a default
            // synthetic id and derive pair from the symbols.
            model_id: "legacy".to_string(),
            pair: format!("{}/{}", primary_symbol.to_uppercase(), short_symbol.to_uppercase()),
            current_position: Position::Flat,
            entry_price: 0.0,
            qty: 1.0,
            tx,
        }
    }

    /// Construct a paper executor bound to a specific model from the
    /// registry (§8). Used by the per-model bootstrap loop in `main()`.
    pub fn new_for_model(
        pool: DbPool,
        fee_rate: f64,
        model_id: &str,
        primary_symbol: &str,
        inverse_symbol: &str,
        tx: Option<TelemetrySender>,
    ) -> Self {
        Self {
            pool,
            fee_rate,
            primary_symbol: primary_symbol.to_string(),
            short_symbol: inverse_symbol.to_string(),
            model_id: model_id.to_string(),
            pair: format!("{}/{}", primary_symbol.to_uppercase(), inverse_symbol.to_uppercase()),
            current_position: Position::Flat,
            entry_price: 0.0,
            qty: 1.0,
            tx,
        }
    }

    /// Read-only accessor used by tests + the WS publisher.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Read-only accessor used by tests + the WS publisher.
    pub fn pair(&self) -> &str {
        &self.pair
    }

    /// Restore position state from the DB after a restart.
    ///
    /// `PaperExecutor` is constructed `Flat` every boot; without this call a
    /// restart loses the open position and the next state transition silently
    /// skips its exit leg (engine thinks Flat while `signal_state` says
    /// Long/Short — the 2026-08-26/27 ghost-position bug). Position comes from
    /// `signal_state` (authoritative, written by the scheduler); entry price
    /// comes from the most recent buy fill of the held instrument.
    pub async fn sync_from_db(&mut self) -> Result<()> {
        let pos = db::load_position(&self.pool, &self.model_id).await?;
        let position = Position::from_i64(pos);
        self.current_position = position;
        if position != Position::Flat {
            let held = Self::symbol_for(position, &self.primary_symbol, &self.short_symbol)
                .to_string();
            self.entry_price = db::fetch_equity_entry_trade_price(&self.pool, &held)
                .await?
                .unwrap_or(0.0);
            info!(
                model_id = %self.model_id,
                position = ?position,
                held_symbol = %held,
                entry_price = self.entry_price,
                "executor state restored from DB"
            );
        } else {
            info!(model_id = %self.model_id, "executor state restored: flat");
        }
        Ok(())
    }

    /// Read-only accessor: current position (used by sync tests).
    pub fn position(&self) -> Position {
        self.current_position
    }

    /// Resolve the instrument symbol for a given position.
    fn symbol_for<'a>(pos: Position, primary: &'a str, short: &'a str) -> &'a str {
        match pos {
            Position::Short => short,
            _ => primary,
        }
    }

    pub async fn set_target_position(
        &mut self,
        target: Position,
        close: f64,
        ts: i64,
    ) -> Result<Vec<FillResult>> {
        // Transition safety: never jump Long <-> Short directly. If the previous
        // position is opposite to the target, the exit leg below flattens first
        // (current_position != target and != Flat → close to Flat), then the
        // entry leg opens the new instrument. This guarantees two clean fills:
        //   Long -> Short : sell QQQ, then buy PSQ
        //   Short -> Long : sell PSQ, then buy QQQ
        if target == self.current_position {
            return Ok(Vec::new());
        }

        let mut fills = Vec::new();

        // --- Exit leg: close the currently-held position (if any). ---
        if self.current_position != Position::Flat {
            // Both a long (QQQ) and a short (PSQ) are closed by SELLING the
            // held instrument. The symbol (QQQ vs PSQ) is resolved separately.
            let exit_side = TradeSide::Sell;
            let exit_symbol = Self::symbol_for(
                self.current_position,
                &self.primary_symbol,
                &self.short_symbol,
            );
            let fee = self.qty * close * self.fee_rate;
            let pnl = match self.current_position {
                Position::Long => (close - self.entry_price) * self.qty - fee,
                Position::Short => (self.entry_price - close) * self.qty - fee,
                Position::Flat => unreachable!(),
            };
            let side_str = "sell";
            info!(
                symbol = exit_symbol,
                side = side_str,
                qty = self.qty,
                price = close,
                fee = fee,
                pnl = pnl,
                "closing position"
            );
            db::insert_equity_trade(
                &self.pool, exit_symbol, ts, side_str, self.qty, close, fee, pnl, &self.model_id,
            )
            .await?;
            let fill = FillResult {
                side: exit_side,
                symbol: exit_symbol.to_string(),
                qty: self.qty,
                price: close,
                fee,
                realized_pnl: pnl,
                ts,
            };
            self.publish_fill(side_str, exit_symbol, &fill);
            fills.push(fill);
        }

        // --- Entry leg: open the target position (if non-Flat). ---
        if target != Position::Flat {
            // Both a long (QQQ) and a short (PSQ) are opened by BUYING the
            // instrument. A short is expressed by buying the inverse ETF (PSQ),
            // not by short-selling the primary — no borrow/locate needed.
            let entry_side = TradeSide::Buy;
            let entry_symbol = Self::symbol_for(target, &self.primary_symbol, &self.short_symbol);
            // Idempotency: never open a second lot for the same candle. A
            // restart + replayed signal must not duplicate the entry (the
            // 2026-08-25/27 "two XLF lots" bug).
            if db::has_open_entry_trade(&self.pool, entry_symbol, ts).await? {
                info!(
                    symbol = entry_symbol,
                    ts,
                    "entry already recorded for this candle — skipping duplicate buy"
                );
            } else {
                let fee = self.qty * close * self.fee_rate;
                let side_str = "buy";
                info!(
                    symbol = entry_symbol,
                    side = side_str,
                    qty = self.qty,
                    price = close,
                    fee = fee,
                    "opening position"
                );
                db::insert_equity_trade(
                    &self.pool, entry_symbol, ts, side_str, self.qty, close, fee, 0.0,
                    &self.model_id,
                )
                .await?;
                self.entry_price = close;
                let fill = FillResult {
                    side: entry_side,
                    symbol: entry_symbol.to_string(),
                    qty: self.qty,
                    price: close,
                    fee,
                    realized_pnl: 0.0,
                    ts,
                };
                self.publish_fill(side_str, entry_symbol, &fill);
                fills.push(fill);
            }
        }

        self.current_position = target;
        Ok(fills)
    }

    /// Publish a `TradeFill` telemetry event if a broadcast sender is wired.
    /// Send errors are silently ignored — no subscribers is a normal state.
    fn publish_fill(&self, side_str: &str, symbol: &str, fill: &FillResult) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(TelemetryEvent::TradeFill {
                model_id: self.model_id.clone(),
                pair: self.pair.clone(),
                side: side_str.to_string(),
                symbol: symbol.to_string(),
                qty: fill.qty,
                price: fill.price,
                fee: fill.fee,
                realized_pnl: fill.realized_pnl,
                timestamp: fill.ts,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in crate::db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn flat_to_long_opens_position() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);

        let fills = exec
            .set_target_position(Position::Long, 50000.0, 1000)
            .await
            .unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Buy);
        assert_eq!(fills[0].symbol, "QQQ");
        assert!((fills[0].qty - 1.0).abs() < 1e-9);
        assert!((fills[0].price - 50000.0).abs() < 1e-9);
        assert!((fills[0].fee - 75.0).abs() < 1e-9);
        assert!((fills[0].realized_pnl - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn long_to_flat_closes_with_pnl() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);
        exec.current_position = Position::Long;
        exec.entry_price = 50000.0;

        let fills = exec
            .set_target_position(Position::Flat, 51000.0, 2000)
            .await
            .unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert_eq!(fills[0].symbol, "QQQ");
        assert!((fills[0].price - 51000.0).abs() < 1e-9);
        assert!((fills[0].fee - 76.5).abs() < 1e-9);
        assert!((fills[0].realized_pnl - 923.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn long_to_short_trades_qqq_then_psq() {
        // Strategic Long -> Short must flatten (sell QQQ) then open short
        // (buy PSQ). This is the paper-mode PSQ inverse-ETF remap.
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);
        exec.current_position = Position::Long;
        exec.entry_price = 50000.0;

        let fills = exec
            .set_target_position(Position::Short, 49000.0, 3000)
            .await
            .unwrap();

        assert_eq!(fills.len(), 2);
        // Exit leg: sell QQQ (close long).
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert_eq!(fills[0].symbol, "QQQ");
        assert!((fills[0].price - 49000.0).abs() < 1e-9);
        assert!((fills[0].fee - 73.5).abs() < 1e-9);
        assert!((fills[0].realized_pnl - (-1073.5)).abs() < 1e-9);

        // Entry leg: buy PSQ (open short via inverse ETF — recorded as 'buy').
        assert_eq!(fills[1].side, TradeSide::Buy);
        assert_eq!(fills[1].symbol, "PSQ");
        assert!((fills[1].price - 49000.0).abs() < 1e-9);
        assert!((fills[1].fee - 73.5).abs() < 1e-9);
        assert!((fills[1].realized_pnl - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn short_to_long_trades_psq_then_qqq() {
        // Strategic Short -> Long must flatten (sell PSQ) then open long (buy QQQ).
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);
        exec.current_position = Position::Short;
        exec.entry_price = 49000.0;

        let fills = exec
            .set_target_position(Position::Long, 51000.0, 3000)
            .await
            .unwrap();

        assert_eq!(fills.len(), 2);
        // Exit leg: sell PSQ (close short).
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert_eq!(fills[0].symbol, "PSQ");
        assert!((fills[0].price - 51000.0).abs() < 1e-9);
        // PnL = (entry - exit) * qty - fee = (49000 - 51000) - 76.5 = -2076.5
        assert!((fills[0].realized_pnl - (-2076.5)).abs() < 1e-9);

        // Entry leg: buy QQQ (open long).
        assert_eq!(fills[1].side, TradeSide::Buy);
        assert_eq!(fills[1].symbol, "QQQ");
        assert!((fills[1].price - 51000.0).abs() < 1e-9);
        assert!((fills[1].realized_pnl - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn flat_to_short_buys_psq() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);

        let fills = exec
            .set_target_position(Position::Short, 48000.0, 3000)
            .await
            .unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Buy);
        assert_eq!(fills[0].symbol, "PSQ");
        assert!((fills[0].price - 48000.0).abs() < 1e-9);
        assert!((fills[0].fee - 72.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn short_to_flat_sells_psq_with_pnl() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);
        exec.current_position = Position::Short;
        exec.entry_price = 49000.0;

        // PSQ falls (inverse ETF rises in value? here price is the ETF price;
        // exiting a short at a lower price is profitable).
        let fills = exec
            .set_target_position(Position::Flat, 48000.0, 4000)
            .await
            .unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert_eq!(fills[0].symbol, "PSQ");
        // PnL = (49000 - 48000) * 1 - 72 = 928.0
        assert!((fills[0].realized_pnl - 928.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn same_position_no_trade() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_symbol(pool, 0.0015, "QQQ", "PSQ", None);
        exec.current_position = Position::Long;
        exec.entry_price = 50000.0;

        let fills = exec
            .set_target_position(Position::Long, 51000.0, 4000)
            .await
            .unwrap();
        assert!(fills.is_empty());
    }

    // -----------------------------------------------------------------------
    // Restart state-loss fix (2026-08-27): sync_from_db + entry idempotency
    // + model_id attribution.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn sync_from_db_restores_long_position_and_entry_price() {
        let pool = test_pool().await;
        // Scheduler previously persisted: model "m1" is long, entered at 500.
        db::save_position(&pool, "m1", Position::Long.as_i64()).await.unwrap();
        db::insert_equity_trade(&pool, "QQQ", 100, "buy", 1.0, 500.0, 0.75, 0.0, "m1")
            .await
            .unwrap();

        // Engine restarts → fresh executor starts Flat.
        let mut exec = PaperExecutor::new_for_model(pool.clone(), 0.0015, "m1", "QQQ", "PSQ", None);
        assert_eq!(exec.position(), Position::Flat);

        exec.sync_from_db().await.unwrap();
        assert_eq!(exec.position(), Position::Long);
        assert!((exec.entry_price - 500.0).abs() < 1e-9);

        // The next transition must produce the EXIT leg (the bug: it was
        // skipped because the executor believed itself Flat).
        let fills = exec.set_target_position(Position::Flat, 520.0, 200).await.unwrap();
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert_eq!(fills[0].symbol, "QQQ");
    }

    #[tokio::test]
    async fn sync_from_db_restores_short_position_holding_inverse_etf() {
        let pool = test_pool().await;
        db::save_position(&pool, "m2", Position::Short.as_i64()).await.unwrap();
        db::insert_equity_trade(&pool, "PSQ", 100, "buy", 1.0, 48.0, 0.07, 0.0, "m2")
            .await
            .unwrap();

        let mut exec = PaperExecutor::new_for_model(pool.clone(), 0.0015, "m2", "QQQ", "PSQ", None);
        exec.sync_from_db().await.unwrap();
        assert_eq!(exec.position(), Position::Short);
        assert!((exec.entry_price - 48.0).abs() < 1e-9);

        // Cover: sells the inverse ETF, not the primary.
        let fills = exec.set_target_position(Position::Flat, 45.0, 200).await.unwrap();
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert_eq!(fills[0].symbol, "PSQ");
    }

    #[tokio::test]
    async fn sync_from_db_flat_model_stays_flat() {
        let pool = test_pool().await;
        db::save_position(&pool, "m3", Position::Flat.as_i64()).await.unwrap();
        let mut exec = PaperExecutor::new_for_model(pool.clone(), 0.0015, "m3", "QQQ", "PSQ", None);
        exec.sync_from_db().await.unwrap();
        assert_eq!(exec.position(), Position::Flat);
    }

    #[tokio::test]
    async fn duplicate_entry_same_candle_is_skipped() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_model(pool.clone(), 0.0015, "m1", "XLF", "FAZ", None);

        // First entry at candle ts=1000.
        let fills1 = exec.set_target_position(Position::Long, 58.0, 1000).await.unwrap();
        assert_eq!(fills1.len(), 1);

        // Restart scenario: fresh executor re-enters the same candle
        // (signal re-fires after boot). Must NOT insert a second lot.
        let mut exec2 = PaperExecutor::new_for_model(pool.clone(), 0.0015, "m1", "XLF", "FAZ", None);
        exec2.current_position = Position::Flat;
        let fills2 = exec2.set_target_position(Position::Long, 58.1, 1000).await.unwrap();
        assert!(fills2.is_empty(), "duplicate entry for same candle must be suppressed");

        // Exactly one buy row in equity_trades for XLF @ ts=1000.
        let n: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM equity_trades WHERE symbol='XLF' AND candle_ts=1000 AND side='buy'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n.0, 1);
    }

    #[tokio::test]
    async fn fills_carry_model_id_attribution() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new_for_model(pool.clone(), 0.0015, "smh-v1", "SMH", "SOXS", None);
        exec.set_target_position(Position::Long, 560.0, 100).await.unwrap();
        exec.set_target_position(Position::Flat, 570.0, 200).await.unwrap();

        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT model_id FROM equity_trades ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2); // buy + sell
        assert!(rows.iter().all(|r| r.0 == "smh-v1"));
    }
}
