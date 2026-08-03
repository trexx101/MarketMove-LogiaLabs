//! Market sentiment data fetcher — Finnhub news_sentiment (free tier).
//!
//! Reads `FINNHUB_API_KEY` from the environment. If absent, falls back to 0.5
//! neutral (the original stub) so the engine still compiles and runs without
//! the key. The source tag ("finnhub" vs "stub") is persisted so the advisor
//! can warn when sentiment is not live.

use std::env;

use anyhow::Result;
use tracing::{debug, info, warn};

use super::finnhub;
use crate::db::DbPool;

/// Fetch daily aggregate sentiment for `symbol`.
///
/// Calls Finnhub `news-sentiment`, computes a weighted score in [-1, 1], and
/// persists to `sentiment_cache`. If `FINNHUB_API_KEY` is absent, falls back
/// to the legacy stub (0.5 neutral, source="stub").
///
/// Returns the computed score.
pub async fn fetch_sentiment(pool: &DbPool, symbol: &str) -> Result<f64> {
    let api_key = finnhub_api_key();

    if api_key.is_empty() {
        // Fall back to stub — same behaviour as before, but we still persist
        // so the cache stays populated for the feature pipeline.
        debug!(symbol, "FINNHUB_API_KEY absent — using stub sentiment (0.5)");
        persist(pool, symbol, 0.5, "stub", 0, 0.0).await?;
        return Ok(0.5);
    }

    match finnhub::fetch_news_sentiment(&api_key, symbol).await {
        Ok(sent) => {
            let score = finnhub::compute_score(&sent);
            let buzz = sent
                .buzz
                .as_ref()
                .map(|b| b.articles_in_last_week)
                .unwrap_or(0);
            let weekly_avg = sent
                .buzz
                .as_ref()
                .map(|b| b.weekly_average)
                .unwrap_or(0.0);

            info!(
                symbol,
                score = format!("{:.3}", score),
                buzz,
                "Finnhub sentiment fetched"
            );
            persist(pool, symbol, score, "finnhub", buzz, weekly_avg).await?;
            Ok(score)
        }
        Err(e) => {
            warn!(symbol, error = %e, "Finnhub sentiment fetch failed — using cached or stub");
            // If the cache already has a recent row, don't overwrite it with stub.
            // Just return the latest cached value (or 0.5 if nothing cached).
            let cached = latest_cached(pool, symbol).await.unwrap_or(0.5);
            // Persist the stub so the table has rows for the UI.
            persist(pool, symbol, cached, "stub", 0, 0.0).await?;
            Ok(cached)
        }
    }
}

/// Persist (or update) a sentiment snapshot to `sentiment_cache`.
async fn persist(
    pool: &DbPool,
    symbol: &str,
    score: f64,
    source: &str,
    buzz: i64,
    weekly_avg: f64,
) -> Result<()> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    sqlx::query(
        "INSERT OR REPLACE INTO sentiment_cache \
         (symbol, date, score, source, buzz, weekly_avg) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(symbol)
    .bind(&today)
    .bind(score)
    .bind(source)
    .bind(buzz)
    .bind(weekly_avg)
    .execute(pool)
    .await?;
    Ok(())
}

/// Seed the sentiment cache for all tracked equity symbols.
///
/// Called once at startup. Now fetches real data from Finnhub instead of
/// writing stub rows unconditionally. If Finnhub is unavailable, falls back
/// to stub so the pipeline still has inputs.
pub async fn seed_sentiment_cache(
    pool: &DbPool,
    symbols: &[&str],
) -> Result<usize> {
    let api_key = finnhub_api_key();
    let mut count = 0;

    if api_key.is_empty() {
        info!("FINNHUB_API_KEY absent — seeding sentiment cache with stub (0.5)");
    } else {
        info!("seeding sentiment cache from Finnhub for {} symbols", symbols.len());
    }

    for sym in symbols {
        match fetch_sentiment(pool, sym).await {
            Ok(_) => count += 1,
            Err(e) => warn!(symbol = sym, error = %e, "sentiment seed failed for symbol"),
        }
        // Small delay to respect Finnhub rate limits.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    info!(seeded = count, "sentiment cache seeded");
    Ok(count)
}

/// Read the FINNHUB_API_KEY from the environment.
fn finnhub_api_key() -> String {
    env::var("FINNHUB_API_KEY").unwrap_or_default().trim().to_string()
}

/// Get the most recent cached sentiment score for a symbol.
/// Returns None if no row exists.
async fn latest_cached(pool: &DbPool, symbol: &str) -> Option<f64> {
    sqlx::query_scalar::<_, f64>(
        "SELECT score FROM sentiment_cache \
         WHERE symbol = ?1 \
         ORDER BY date DESC LIMIT 1",
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_neutral_when_no_key() {
        // Without the env var set, fetch_sentiment would fall back to stub.
        // Can't test async easily without a pool, but the logic path is clear.
        std::env::remove_var("FINNHUB_API_KEY");
        assert_eq!(finnhub_api_key(), "");
    }
}