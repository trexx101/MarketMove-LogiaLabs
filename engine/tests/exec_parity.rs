//! Execution layer parity tests.
//!
//! Validates PaperExecutor PnL math against hand-computed fixtures
//! and verifies KrakenExecutor signing against the documented scheme.

use engine::exec::paper::PaperExecutor;
use engine::exec::kraken::KrakenExecutor;
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
    let mut exec = PaperExecutor::new(pool, 0.0015);

    // t=0: Flat → Long at 50000
    let fills = exec.set_target_position(Position::Long, 50_000.0, 1000).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Buy);
    assert!((fills[0].fee - 75.0).abs() < 1e-9);
    assert!((fills[0].realized_pnl - 0.0).abs() < 1e-9);

    // t=1: Long → Short at 51000
    let fills = exec.set_target_position(Position::Short, 51_000.0, 2000).await.unwrap();
    assert_eq!(fills.len(), 2);

    // Close long
    assert_eq!(fills[0].side, TradeSide::Sell);
    assert!((fills[0].price - 51_000.0).abs() < 1e-9);
    assert!((fills[0].fee - 76.5).abs() < 1e-9);
    // PnL = (51000 - 50000) * 1.0 - 76.5 = 923.5
    assert!((fills[0].realized_pnl - 923.5).abs() < 1e-9);

    // Open short
    assert_eq!(fills[1].side, TradeSide::Sell);
    assert!((fills[1].price - 51_000.0).abs() < 1e-9);
    assert!((fills[1].fee - 76.5).abs() < 1e-9);
    assert!((fills[1].realized_pnl - 0.0).abs() < 1e-9);

    // t=2: Short → Flat at 49500
    let fills = exec.set_target_position(Position::Flat, 49_500.0, 3000).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Buy);
    assert!((fills[0].price - 49_500.0).abs() < 1e-9);
    assert!((fills[0].fee - 74.25).abs() < 1e-9);
    // PnL = (51000 - 49500) * 1.0 - 74.25 = 1425.75
    assert!((fills[0].realized_pnl - 1425.75).abs() < 1e-9);
}

#[tokio::test]
async fn paper_short_entry_and_exit() {
    let pool = test_pool().await;
    let mut exec = PaperExecutor::new(pool, 0.001); // 0.1% fee

    // Flat → Short at 60000
    let fills = exec.set_target_position(Position::Short, 60_000.0, 100).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Sell);
    assert!((fills[0].fee - 60.0).abs() < 1e-9);

    // Short → Flat at 59000 (profitable short)
    let fills = exec.set_target_position(Position::Flat, 59_000.0, 200).await.unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].side, TradeSide::Buy);
    // PnL = (60000 - 59000) * 1.0 - 59.0 = 941.0
    assert!((fills[0].realized_pnl - 941.0).abs() < 1e-9);
}

#[tokio::test]
async fn paper_losing_trade() {
    let pool = test_pool().await;
    let mut exec = PaperExecutor::new(pool, 0.0015);

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
    let mut exec = PaperExecutor::new(pool, 0.0);

    exec.set_target_position(Position::Long, 50_000.0, 100).await.unwrap();
    let fills = exec.set_target_position(Position::Flat, 51_000.0, 200).await.unwrap();
    assert!((fills[0].fee - 0.0).abs() < 1e-9);
    // PnL = (51000 - 50000) * 1.0 - 0 = 1000.0
    assert!((fills[0].realized_pnl - 1000.0).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Kraken signature verification
// ---------------------------------------------------------------------------

#[test]
fn kraken_sign_request_produces_64_byte_signature() {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let secret_b64 = "kQH5HW/8p1uGOVjbgWA7FunAmGO8lsSUXFYsuR2BHIc=";
    let exec = KrakenExecutor::new("test-key", secret_b64, "BTC/USD").unwrap();

    let sig = exec
        .sign_request(
            "/0/private/AddOrder",
            "1700000000000000",
            "nonce=1700000000000000&pair=XBTUSD&type=buy&ordertype=market&volume=0.01",
        )
        .unwrap();

    let decoded = STANDARD.decode(&sig).unwrap();
    assert_eq!(decoded.len(), 64, "HMAC-SHA512 must produce 64 bytes");
    assert_eq!(sig.len(), 88, "base64 of 64 bytes must be 88 chars");
}

#[test]
fn kraken_different_nonces_produce_different_signatures() {
    let secret_b64 = "kQH5HW/8p1uGOVjbgWA7FunAmGO8lsSUXFYsuR2BHIc=";
    let exec = KrakenExecutor::new("test-key", secret_b64, "BTC/USD").unwrap();

    let sig1 = exec
        .sign_request("/0/private/Balance", "1000", "nonce=1000")
        .unwrap();
    let sig2 = exec
        .sign_request("/0/private/Balance", "2000", "nonce=2000")
        .unwrap();

    assert_ne!(sig1, sig2, "different nonces must produce different signatures");
}

#[test]
fn kraken_different_paths_produce_different_signatures() {
    let secret_b64 = "kQH5HW/8p1uGOVjbgWA7FunAmGO8lsSUXFYsuR2BHIc=";
    let exec = KrakenExecutor::new("test-key", secret_b64, "BTC/USD").unwrap();

    let sig1 = exec
        .sign_request("/0/private/Balance", "1000", "nonce=1000")
        .unwrap();
    let sig2 = exec
        .sign_request("/0/private/AddOrder", "1000", "nonce=1000")
        .unwrap();

    assert_ne!(sig1, sig2, "different paths must produce different signatures");
}
