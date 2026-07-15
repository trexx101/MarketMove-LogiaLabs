pub mod rest;
pub mod ws;

use anyhow::Result;
use tracing::info;

use crate::db::DbPool;

/// Backfill + retention constants.
/// Keep 250 candles — comfortably above the 200-SMA + rolling-window requirement.
pub const RETENTION_CANDLES: usize = 250;

/// Run the REST backfill to ensure ≥ `min_candles` rows in the DB.
///
/// This is split out from the live-WS path so callers can `await` it
/// synchronously before spawning the scheduler. If the scheduler runs before
/// the DB has enough history, the first inference tick will see `seq_len=1`
/// and every downstream computation will be wrong.
pub async fn backfill(pool: DbPool, symbol: &str, min_candles: usize) -> Result<()> {
    let seeded = rest::backfill(&pool, symbol, min_candles).await?;
    info!(seeded, "REST backfill complete");
    Ok(())
}

/// Spawn the hourly retention task and run the persistent WebSocket loop.
///
/// The retention task is a background job that prunes old rows every hour.
/// The WS loop never returns under normal operation (reconnects on failure).
pub async fn run_ws_and_retention(pool: DbPool, symbol: &str) -> Result<()> {
    // --- 1. Retention task (every 1 h) ---
    let retention_pool = pool.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(3_600));
        loop {
            interval.tick().await;
            match crate::db::prune_old(&retention_pool, RETENTION_CANDLES).await {
                Ok(n) if n > 0 => info!(pruned = n, "retention: removed old candles"),
                Ok(_) => {}
                Err(e) => tracing::warn!("retention prune error: {e:#}"),
            }
        }
    });

    // --- 2. WebSocket loop ---
    ws::run_loop(&pool, symbol).await
}
