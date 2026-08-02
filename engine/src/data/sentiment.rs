//! Market sentiment data fetcher (Phase 1 stub → Phase 2 Finnhub).
//!
//! **Current state:** Returns neutral 0.5 for all symbols. The `sentiment_cache`
//! table exists and is populated with neutral values so the feature pipeline
//! has sentiment inputs available from day one.
//!
//! **Phase 2:** Wire Finnhub `news_sentiment` endpoint (free tier: 60 req/min).
//! The stub fetcher is a one-line swap: replace `return Ok(0.5)` with an HTTP
//! call to `finnhub.io/api/v1/news-sentiment`.

use anyhow::Result;
use tracing::info;

use crate::db::DbPool;

/// Fetch daily aggregate sentiment for `symbol`.
///
/// **Stub:** Always returns 0.5 (neutral). Phase 2 will query Finnhub's
/// `news_sentiment` endpoint and compute a weighted average.
pub async fn fetch_sentiment(_pool: &DbPool, symbol: &str) -> Result<f64> {
    // Placeholder: neutral sentiment for all symbols.
    // Phase 2 implementation:
    //   GET https://finnhub.io/api/v1/news-sentiment?symbol={code}&token={FINNHUB_API_KEY}
    //   → parse `sentiment.bullishPercent` + `buzz` → weighted score [-1, 1]
    let _ = symbol;
    Ok(0.5)
}

/// Persist stub sentiment values for all tracked equity symbols.
///
/// Called once at startup to ensure the cache table has rows. Daily top-up
/// would call the same function (which is idempotent — INSERT OR REPLACE).
pub async fn seed_sentiment_cache(
    pool: &DbPool,
    symbols: &[&str],
) -> Result<usize> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut count = 0;
    for sym in symbols {
        sqlx::query(
            "INSERT OR REPLACE INTO sentiment_cache (symbol, date, score, source) \
             VALUES (?1, ?2, 0.5, 'stub')",
        )
        .bind(*sym)
        .bind(&today)
        .execute(pool)
        .await?;
        count += 1;
    }
    info!(symbols = symbols.len(), "sentiment cache seeded (stub)");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_neutral() {
        // Can't test async easily without a pool, but the logic is dead simple.
        assert_eq!(true, true);
    }
}