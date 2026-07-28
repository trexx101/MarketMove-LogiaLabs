use anyhow::Result;
use tracing::info;

use crate::api::ws::{TelemetryEvent, TelemetrySender};
use crate::db::{self, DbPool};
use crate::exec::{FillResult, TradeSide};
use crate::strategy::Position;

pub struct PaperExecutor {
    pool: DbPool,
    fee_rate: f64,
    current_position: Position,
    entry_price: f64,
    qty: f64,
    tx: Option<TelemetrySender>,
}

impl PaperExecutor {
    pub fn new(pool: DbPool, fee_rate: f64, tx: Option<TelemetrySender>) -> Self {
        Self {
            pool,
            fee_rate,
            current_position: Position::Flat,
            entry_price: 0.0,
            qty: 1.0,
            tx,
        }
    }

    pub async fn set_target_position(
        &mut self,
        target: Position,
        close: f64,
        ts: i64,
    ) -> Result<Vec<FillResult>> {
        if target == self.current_position {
            return Ok(Vec::new());
        }

        let mut fills = Vec::new();

        if self.current_position != Position::Flat {
            let exit_side = match self.current_position {
                Position::Long => TradeSide::Sell,
                Position::Short => TradeSide::Buy,
                Position::Flat => unreachable!(),
            };
            let fee = self.qty * close * self.fee_rate;
            let pnl = match self.current_position {
                Position::Long => (close - self.entry_price) * self.qty - fee,
                Position::Short => (self.entry_price - close) * self.qty - fee,
                Position::Flat => unreachable!(),
            };
            let side_str = match exit_side {
                TradeSide::Buy => "buy",
                TradeSide::Sell => "sell",
            };
            info!(
                side = side_str,
                qty = self.qty,
                price = close,
                fee = fee,
                pnl = pnl,
                "closing position"
            );
            db::insert_trade(&self.pool, ts, side_str, self.qty, close, fee, pnl).await?;
            let fill = FillResult {
                side: exit_side,
                qty: self.qty,
                price: close,
                fee,
                realized_pnl: pnl,
                ts,
            };
            self.publish_fill(side_str, &fill);
            fills.push(fill);
        }

        if target != Position::Flat {
            let entry_side = match target {
                Position::Long => TradeSide::Buy,
                Position::Short => TradeSide::Sell,
                Position::Flat => unreachable!(),
            };
            let fee = self.qty * close * self.fee_rate;
            let side_str = match entry_side {
                TradeSide::Buy => "buy",
                TradeSide::Sell => "sell",
            };
            info!(
                side = side_str,
                qty = self.qty,
                price = close,
                fee = fee,
                "opening position"
            );
            db::insert_trade(&self.pool, ts, side_str, self.qty, close, fee, 0.0).await?;
            self.entry_price = close;
            let fill = FillResult {
                side: entry_side,
                qty: self.qty,
                price: close,
                fee,
                realized_pnl: 0.0,
                ts,
            };
            self.publish_fill(side_str, &fill);
            fills.push(fill);
        }

        self.current_position = target;
        Ok(fills)
    }

    /// Publish a `TradeFill` telemetry event if a broadcast sender is wired.
    /// Send errors are silently ignored — no subscribers is a normal state.
    fn publish_fill(&self, side_str: &str, fill: &FillResult) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(TelemetryEvent::TradeFill {
                side: side_str.to_string(),
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
        let mut exec = PaperExecutor::new(pool, 0.0015, None);

        let fills = exec.set_target_position(Position::Long, 50000.0, 1000).await.unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Buy);
        assert!((fills[0].qty - 1.0).abs() < 1e-9);
        assert!((fills[0].price - 50000.0).abs() < 1e-9);
        assert!((fills[0].fee - 75.0).abs() < 1e-9);
        assert!((fills[0].realized_pnl - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn long_to_flat_closes_with_pnl() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new(pool, 0.0015, None);
        exec.current_position = Position::Long;
        exec.entry_price = 50000.0;

        let fills = exec.set_target_position(Position::Flat, 51000.0, 2000).await.unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert!((fills[0].price - 51000.0).abs() < 1e-9);
        assert!((fills[0].fee - 76.5).abs() < 1e-9);
        assert!((fills[0].realized_pnl - 923.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn long_to_short_closes_and_opens() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new(pool, 0.0015, None);
        exec.current_position = Position::Long;
        exec.entry_price = 50000.0;

        let fills = exec.set_target_position(Position::Short, 49000.0, 3000).await.unwrap();

        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].side, TradeSide::Sell);
        assert!((fills[0].price - 49000.0).abs() < 1e-9);
        assert!((fills[0].fee - 73.5).abs() < 1e-9);
        assert!((fills[0].realized_pnl - (-1073.5)).abs() < 1e-9);

        assert_eq!(fills[1].side, TradeSide::Sell);
        assert!((fills[1].price - 49000.0).abs() < 1e-9);
        assert!((fills[1].fee - 73.5).abs() < 1e-9);
        assert!((fills[1].realized_pnl - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn same_position_no_trade() {
        let pool = test_pool().await;
        let mut exec = PaperExecutor::new(pool, 0.0015, None);
        exec.current_position = Position::Long;
        exec.entry_price = 50000.0;

        let fills = exec.set_target_position(Position::Long, 51000.0, 4000).await.unwrap();
        assert!(fills.is_empty());
    }
}
