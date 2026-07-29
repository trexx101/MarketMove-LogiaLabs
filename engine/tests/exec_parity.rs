//! Execution layer parity tests.
//!
//! Validates PaperExecutor PnL math against hand-computed fixtures
//! and verifies KrakenExecutor signing against the documented scheme.

use engine::exec::paper::PaperExecutor;
use engine::exec::TradeSide;
use engine::strategy::Position;
use sqlx::sqlite::SqlitePoolOptions;

async fn test_pool() -> engine::db::DbPool {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory pool");
    for stmt in engine::db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(stmt).execute(&pool).await.expect("DDL");
    }
    pool
}

// ---------------------------------------------------------------------------
// Paper PnL hand-computed fixture
// ---------------------------------------------------------------------------
//
// Sequence:
//   t=0: Flat → Long  at 50000  (buy, fee = 50000 * 0.0015 = 75.0)
//   t=1: Long  → Short at 51000  (sell close: pnl = (51000-50000)*1 - 76.5 = 923.5)
//                                  (sell open:  fee = 51000 * 0.0015 = 76.5)
//   t=2: Short → Flat  at 49500  (buy close:  pnl = (51000-49500)*1 - 74.25 = 1425.75)
//
// Total realized PnL = 923.5 + 1425.75 = 2349.25
// Total fees = 75.0 + 76.5 + 76.5 + 74.25 = 302.25

#[tokio::test]
async fn paper_pnl_hand_computed_fixture() {
    let pool = test_pool().await;
    let mut exec = PaperExecutor::new(pool, 0.0015, None);

    // t=0: Flat → Long at 50000
    let fills = exec.set_target_position(Position::Long, 50_000.0, 1000).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Buy);
    assert!((fills[0].fee - 75.0).abs() < 1e-9);
    assert!((fills[0].realized_pnl - 0.0).abs() < 1e-9);

    // t=1: Long → Short at 51000
    // Under the PSQ inverse-ETF remap, a short is opened by BUYING PSQ (not a
    // traditional short-sell). The close-long leg is unchanged (sell QQQ).
    let fills = exec.set_target_position(Position::Short, 51_000.0, 2000).await.unwrap();
    assert_eq!(fills.len(), 2);

    // Close long (sell QQQ).
    assert_eq!(fills[0].side, TradeSide::Sell);
    assert_eq!(fills[0].symbol, "QQQ");
    assert!((fills[0].price - 51_000.0).abs() < 1e-9);
    assert!((fills[0].fee - 76.5).abs() < 1e-9);
    // PnL = (51000 - 50000) * 1.0 - 76.5 = 923.5
    assert!((fills[0].realized_pnl - 923.5).abs() < 1e-9);

    // Open short via inverse ETF (buy PSQ).
    assert_eq!(fills[1].side, TradeSide::Buy);
    assert_eq!(fills[1].symbol, "PSQ");
    assert!((fills[1].price - 51_000.0).abs() < 1e-9);
    assert!((fills[1].fee - 76.5).abs() < 1e-9);
    assert!((fills[1].realized_pnl - 0.0).abs() < 1e-9);

    // t=2: Short → Flat at 49500 (sell PSQ to close the short).
    let fills = exec.set_target_position(Position::Flat, 49_500.0, 3000).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Sell);
    assert_eq!(fills[0].symbol, "PSQ");
    assert!((fills[0].price - 49_500.0).abs() < 1e-9);
    assert!((fills[0].fee - 74.25).abs() < 1e-9);
    // PnL = (entry - exit) * qty - fee = (51000 - 49500) * 1.0 - 74.25 = 1425.75
    assert!((fills[0].realized_pnl - 1425.75).abs() < 1e-9);
}

#[tokio::test]
async fn paper_short_entry_and_exit() {
    let pool = test_pool().await;
    let mut exec = PaperExecutor::new(pool, 0.001, None); // 0.1% fee

    // Flat → Short at 60000 (buy PSQ to open the short).
    let fills = exec.set_target_position(Position::Short, 60_000.0, 100).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Buy);
    assert_eq!(fills[0].symbol, "PSQ");
    assert!((fills[0].fee - 60.0).abs() < 1e-9);

    // Short → Flat at 59000 (sell PSQ to close; profitable short).
    let fills = exec.set_target_position(Position::Flat, 59_000.0, 200).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Sell);
    assert_eq!(fills[0].symbol, "PSQ");
    assert!((fills[0].realized_pnl - 941.0).abs() < 1e-9);
}

#[tokio::test]
async fn paper_losing_trade() {
    let pool = test_pool().await;
    let mut exec = PaperExecutor::new(pool, 0.0015, None);

    // Flat → Long at 50000
    exec.set_target_position(Position::Long, 50_000.0, 100).await.unwrap();

    // Long → Flat at 48000 (losing trade)
    let fills = exec.set_target_position(Position::Flat, 48_000.0, 200).await.unwrap();
    assert_eq!(fills.len(), 1);
    // PnL = (48000 - 50000) * 1.0 - (48000 * 0.0015) = -2000 - 72 = -2072.0
    assert!((fills[0].realized_pnl - (-2072.0)).abs() < 1e-9);
}

#[tokio::test]
async fn paper_zero_fee() {
    let pool = test_pool().await;
    let mut exec = PaperExecutor::new(pool, 0.0, None);

    exec.set_target_position(Position::Long, 50_000.0, 100).await.unwrap();
    let fills = exec.set_target_position(Position::Flat, 51_000.0, 200).await.unwrap();
    assert!((fills[0].fee - 0.0).abs() < 1e-9);
    // PnL = (51000 - 50000) * 1.0 - 0 = 1000.0
    assert!((fills[0].realized_pnl - 1000.0).abs() < 1e-9);
}
