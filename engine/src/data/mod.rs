pub mod rest;
pub mod ws;

use anyhow::Result;
use tracing::info;

use crate::db::DbPool;

/// Backfill + retention constants.
/// Keep 250 candles — comfortably above the 200-SMA + rolling-window requirement.
pub const RETENTION_CANDLES: usize = 250;

/// Run the full data pipeline:
///   1. REST backfill to ensure ≥ `min_candles` rows in the DB.
///   2. A background task that prunes old rows every hour.
///   3. A persistent WebSocket loop that ingests live candles and reconnects on failure.
///
/// This function does not return under normal operation.
pub async fn run(pool: DbPool, symbol: &str, min_candles: usize) -> Result<()> {
    // --- 1. REST backfill ---
    let seeded = rest::backfill(&pool, symbol, min_candles).await?;
    info!(seeded, "REST backfill complete");

    // --- 2. Retention task (every 1 h) ---
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

    // --- 3. WebSocket loop ---
    ws::run_loop(&pool, symbol).await
}
