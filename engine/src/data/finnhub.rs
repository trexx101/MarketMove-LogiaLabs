//! Finnhub API client — news sentiment + earnings calendar.
//!
//! Free tier: 60 req/min. We cache aggressively (daily for sentiment, hourly for
//! calendar) so we stay well under the limit.
//!
//! Endpoints:
//!   - news-sentiment: GET finnhub.io/api/v1/news-sentiment?symbol=QQQ&token=...
//!   - earnings-calendar: GET finnhub.io/api/v1/calendar/earnings?from=...&to=...&token=...

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tracing::debug;

const FINNHUB_BASE: &str = "https://finnhub.io/api/v1";

/// Aggregate sentiment for a symbol from the last 24h of news.
#[derive(Debug, Clone, Deserialize)]
pub struct NewsSentiment {
    #[serde(default)]
    pub buzz: Option<SentimentBuzz>,
    #[serde(default)]
    pub sentiment: Option<SentimentStats>,
    #[serde(default)]
    pub symbol: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SentimentBuzz {
    #[serde(default)]
    #[serde(rename = "articlesInLastWeek")]
    pub articles_in_last_week: i64,
    #[serde(default)]
    #[serde(rename = "weeklyAverage")]
    pub weekly_average: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SentimentStats {
    #[serde(default)]
    pub bullish: Option<f64>,
    #[serde(default)]
    pub bearish: Option<f64>,
}

/// Compute a single sentiment score in [-1, 1] from the Finnhub response.
///
/// Uses `bullishPercent` weighted by `log1p(buzz)` to avoid over-weighting
/// low-volume signals. Returns 0.0 (neutral) if the payload is empty.
pub fn compute_score(sentiment: &NewsSentiment) -> f64 {
    let bullish_pct = sentiment
        .sentiment
        .as_ref()
        .and_then(|s| s.bullish)
        .unwrap_or(0.5);

    let buzz = sentiment
        .buzz
        .as_ref()
        .map(|b| b.articles_in_last_week)
        .unwrap_or(0);

    // Weight: higher buzz → more confidence in the score. log1p so it doesn't
    // dominate. At 0 articles → weight 0 → score 0. At 50 articles → weight ~3.9.
    let weight = (buzz as f64 + 1.0).ln();

    // Map bullishPercent from [0,1] to [-1,1] with buzz weighting.
    if weight > 0.0 {
        let raw = (bullish_pct - 0.5) * 2.0; // [-1, 1] centred at 0
        (raw * weight / (weight + 1.0)).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Fetch news sentiment for a symbol from Finnhub.
///
/// Returns `None` if `FINNHUB_API_KEY` is not set. On API error, returns
/// the error — let the caller decide whether to fall back to stub.
pub async fn fetch_news_sentiment(
    api_key: &str,
    symbol: &str,
) -> Result<NewsSentiment> {
    if api_key.is_empty() {
        bail!("FINNHUB_API_KEY not set");
    }

    let url = format!(
        "{}/news-sentiment?symbol={}&token={}",
        FINNHUB_BASE, symbol, api_key
    );
    debug!(symbol, "fetching Finnhub news sentiment");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building reqwest client for Finnhub")?;

    let resp = client
        .get(&url)
        .send()
        .await
        .context("Finnhub news-sentiment request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Finnhub returned {status}: {body}");
    }

    let sentiment: NewsSentiment = resp
        .json()
        .await
        .context("parsing Finnhub news-sentiment response")?;

    Ok(sentiment)
}

/// Fetch earnings calendar for a symbol in the given date range.
///
/// Returns empty vec if no earnings in the window.
#[derive(Debug, Clone, Deserialize)]
pub struct EarningsCalendarEntry {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub hour: String,
    #[serde(default)]
    #[serde(rename = "epsEstimate")]
    pub eps_estimate: Option<f64>,
    #[serde(default)]
    #[serde(rename = "epsActual")]
    pub eps_actual: Option<f64>,
}

pub async fn fetch_earnings_calendar(
    api_key: &str,
    symbol: &str,
    from: &str, // YYYY-MM-DD
    to: &str,
) -> Result<Vec<EarningsCalendarEntry>> {
    if api_key.is_empty() {
        bail!("FINNHUB_API_KEY not set");
    }

    let url = format!(
        "{}/calendar/earnings?from={}&to={}&symbol={}&token={}",
        FINNHUB_BASE, from, to, symbol, api_key
    );
    debug!(symbol, from, to, "fetching Finnhub earnings calendar");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building reqwest client for Finnhub")?;

    let resp = client
        .get(&url)
        .send()
        .await
        .context("Finnhub earnings-calendar request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Finnhub earnings calendar returned {status}: {body}");
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .context("parsing Finnhub earnings-calendar response")?;

    // Finnhub returns { "earningsCalendar": [...] }
    let entries = body["earningsCalendar"].as_array().cloned().unwrap_or_default();
    let parsed: Vec<EarningsCalendarEntry> = serde_json::from_value(serde_json::Value::Array(entries))
        .unwrap_or_default();

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_score_empty() {
        let s = NewsSentiment {
            buzz: None,
            sentiment: None,
            symbol: "QQQ".into(),
        };
        let score = compute_score(&s);
        assert!((score - 0.0).abs() < 1e-9, "empty payload → neutral");
    }

    #[test]
    fn compute_score_bullish() {
        let s = NewsSentiment {
            buzz: Some(SentimentBuzz {
                articles_in_last_week: 50,
                weekly_average: 40.0,
            }),
            sentiment: Some(SentimentStats {
                bullish: Some(0.8),
                bearish: Some(0.1),
            }),
            symbol: "QQQ".into(),
        };
        let score = compute_score(&s);
        // bullishPercent=0.8 → raw=0.6, weight=ln(51)≈3.93, score≈0.6*3.93/4.93≈0.48
        assert!(score > 0.3, "high buzz + bullish → positive score, got {score}");
        assert!(score <= 1.0);
    }

    #[test]
    fn compute_score_bearish() {
        let s = NewsSentiment {
            buzz: Some(SentimentBuzz {
                articles_in_last_week: 30,
                weekly_average: 25.0,
            }),
            sentiment: Some(SentimentStats {
                bullish: Some(0.2),
                bearish: Some(0.6),
            }),
            symbol: "QQQ".into(),
        };
        let score = compute_score(&s);
        // bullishPercent=0.2 → raw=-0.6, weight=ln(31)≈3.43, score≈-0.6*3.43/4.43≈-0.46
        assert!(score < -0.2, "high buzz + bearish → negative score, got {score}");
        assert!(score >= -1.0);
    }

    #[test]
    fn compute_score_no_buzz() {
        let s = NewsSentiment {
            buzz: Some(SentimentBuzz {
                articles_in_last_week: 0,
                weekly_average: 0.0,
            }),
            sentiment: Some(SentimentStats {
                bullish: Some(0.9),
                bearish: Some(0.0),
            }),
            symbol: "QQQ".into(),
        };
        let score = compute_score(&s);
        // No buzz → weight=0 → score=0 regardless of sentiment
        assert!((score - 0.0).abs() < 1e-9);
    }
}