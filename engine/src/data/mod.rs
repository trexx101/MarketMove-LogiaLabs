pub mod cboe;
pub mod fred;
pub mod moomoo;
pub mod sentiment;
pub mod yahoo;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::db::DbPool;

/// Symbols pulled from Yahoo Finance / Moomoo for the equities engine.
/// QQQ (the trade target) + key constituents + cross-asset ETFs.
pub const EQUITY_SYMBOLS: &[&str] = &[
    "QQQ", "AAPL", "MSFT", "NVDA", "GOOG", "AMZN", "META", "TSLA", "TLT", "GLD", "UUP",
];

/// Macro series (stored in `equity_candles` with `$` prefixes).
/// $VIX → CBOE (free, no auth). $UST10Y/$DXY → FRED JSON API.
pub const MACRO_SYMBOLS: &[&str] = &["$VIX", "$UST10Y", "$DXY"];

/// Backfill everything the Wave A engine needs: equity OHLCV + macro series.
///
/// **Data source routing (priority order):**
/// 1. Equities: Moomoo OpenD → Yahoo Finance (fallback)
/// 2. VIX:      CBOE CSV → Yahoo ^VIX (fallback)
/// 3. Macro:    FRED JSON API v2 (DGS10, DTWEXBGS)
///
/// Idempotent — each symbol respects its own min-candles gate in the clients,
/// so re-running just tops up missing history. Errors from one symbol are
/// logged and skipped (best-effort) so a single outage doesn't abort the rest.
///
/// `stale_threshold_secs` controls the freshness gate passed to the Yahoo
/// client (used as fallback). Startup uses 3 days; daily top-up uses 18h.
pub async fn backfill_equities(pool: &DbPool, stale_threshold_secs: i64) -> Result<()> {
    // ── 1. Equity OHLCV: Moomoo first, Yahoo fallback ──────────
    let moomoo_ok = moomoo::is_available().await;
    if moomoo_ok {
        info!("Moomoo OpenD reachable — using as primary equity source");
        for s in EQUITY_SYMBOLS {
            match moomoo::backfill(pool, s, 250, 5 * 365).await {
                Ok(n) => info!(symbol = s, rows = n, "Moomoo backfill complete"),
                Err(e) => {
                    warn!(symbol = s, error = %e, "Moomoo backfill failed — trying Yahoo fallback");
                    match yahoo::backfill(pool, s, 250, "5y", stale_threshold_secs).await {
                        Ok(n) => info!(symbol = s, rows = n, "Yahoo fallback complete"),
                        Err(e2) => warn!(symbol = s, error = %e2, "Yahoo fallback also failed"),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    } else {
        info!("Moomoo OpenD not reachable — using Yahoo for equities");
        let n_eq = yahoo::backfill_many(
            pool, EQUITY_SYMBOLS, 250, "5y", stale_threshold_secs,
        )
        .await?;
        info!(rows = n_eq, "equity OHLCV backfill complete (Yahoo)");
    }

    // ── 2. VIX: CBOE (free, no auth, no rate limits) ──────────
    match cboe::backfill_vix(pool, 5 * 365).await {
        Ok(n) => info!(rows = n, "CBOE VIX backfill complete"),
        Err(e) => {
            warn!(error = %e, "CBOE VIX failed — trying Yahoo ^VIX fallback");
            let vix_count = crate::db::count_equity_candles(pool, "$VIX").await?;
            if vix_count <= 1 {
                match yahoo::backfill(pool, "^VIX", 1, "2y", stale_threshold_secs).await {
                    Ok(n) if n > 0 => info!(rows = n, "Yahoo ^VIX fallback loaded"),
                    Ok(_) => debug!("Yahoo ^VIX returned 0 new rows"),
                    Err(e2) => warn!(error = %e2, "Yahoo ^VIX fallback failed"),
                }
            }
        }
    }

    // ── 3. Macro: FRED JSON API (DGS10, DTWEXBGS) ─────────────
    let n_macro = fred::backfill_all_default_macros(pool, 5 * 365).await;
    match n_macro {
        Ok(n) => info!(rows = n, "macro series backfill complete (FRED JSON API)"),
        Err(e) => warn!(error = %e, "FRED macro backfill failed"),
    }

    Ok(())
}

/// Run the equities ingestion supervisor. Blocks until a fatal error.
///
/// Spawns:
///   - a daily top-up task that re-pulls the most recent trading days for
///     every equity + macro symbol (Yahoo/FRED clients are idempotent and
///     only upsert rows newer than what's already stored);
///   - a daily retention task that prunes `equity_candles` beyond the
///     retention window so the table doesn't grow unbounded (deep history
///     is re-fetched from the source on demand, not kept forever).
///
/// Daily cadence is appropriate for daily bars — no streaming feed needed.
pub async fn run_equities_ingestion(pool: DbPool) -> Result<()> {
    // --- 1. Daily equities top-up (02:00 local-ish, every 24h) ---
    let topup_pool = pool.clone();
    tokio::spawn(async move {
        // Initial short delay so the startup backfill settles first.
        let start =
            tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut interval =
            tokio::time::interval_at(start, std::time::Duration::from_secs(24 * 3_600));
        loop {
            interval.tick().await;
            if let Err(e) = backfill_equities(&topup_pool, 18 * 3600).await {
                tracing::error!(error = %e, "equities daily top-up failed");
            }
        }
    });

    // --- 2. Daily retention prune (keep 5y of daily bars) ---
    let retention_pool = pool.clone();
    tokio::spawn(async move {
        let start =
            tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        let mut interval =
            tokio::time::interval_at(start, std::time::Duration::from_secs(24 * 3_600));
        loop {
            interval.tick().await;
            match prune_equity_history(&retention_pool).await {
                Ok(n) if n > 0 => info!(pruned = n, "equity retention: removed old rows"),
                Ok(_) => {}
                Err(e) => tracing::warn!("equity retention prune error: {e:#}"),
            }
        }
    });

    // Block the process on an inert future so the supervisor owns the main thread.
    // A Ctrl-C / process kill is the intended shutdown.
    std::future::pending::<()>().await;
    Ok(())
}

/// Prune `equity_candles` rows older than `keep_days` for every symbol.
/// Daily bars at 11 symbols + 3 macros over 5y ≈ 14×~1,260 ≈ 17k rows — cheap
/// to keep, but pruning guards against unbounded growth if cadence slips.
async fn prune_equity_history(pool: &DbPool) -> Result<usize> {
    let keep_days: i64 = 5 * 365;
    let cutoff = chrono::Utc::now().timestamp() - keep_days * 86_400;
    let res = sqlx::query("DELETE FROM equity_candles WHERE ts < ?1")
        .bind(cutoff)
        .execute(pool)
        .await
        .context("prune_equity_history")?;
    Ok(res.rows_affected() as usize)
}
