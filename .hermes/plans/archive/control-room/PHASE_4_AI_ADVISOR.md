# Phase 4 — AI Trading Advisor (Revised)

**Goal**: An LLM-powered daily briefing that ingests the live features the dashboard
already shows (the "Feature Inspector" surface), the model's predictions, recent
sentiment, and macro/calendar context — then produces a verbal digest with
justifications for entries/exits and explicit warnings. The advisor is strictly
advisory (zero execution authority). It also answers conversational follow-up
questions with the same context envelope.

**Estimated effort**: ~2 weeks (incl. 4.0 sentiment prerequisite)
**Can deploy independently**: Yes — additive overlay, opt-in via config flag.
**Depends on**:
- Phase 0 (DB tables, API module tree) — **met**
- Phase 1 (AppState, WS) — **met**
- Phase 2 (Strategy Lab) — **met** (`engine/src/strategy_lab/`, `views/StrategyLab.svelte`)
- **4.0 Sentiment (Finnhub wiring) — NEW prerequisite**, must land first

---

## 4.0 Sentiment Prerequisite (MUST land before §4.1)

The current `engine/src/data/sentiment.rs` is a stub returning 0.5. Without real
sentiment the advisor cannot produce the "novel information" we want — it would
only echo the model's own features back at us.

### Changes
1. Replace `fetch_sentiment` body with a Finnhub `news_sentiment` call:
   `GET https://finnhub.io/api/v1/news-sentiment?symbol={code}&token={FINNHUB_API_KEY}`
2. Parse `sentiment.bullishPercent` (0–1) and `buzz.articlesInLastWeek` (volume).
   Compute a weighted score: `score = bullishPercent * log1p(buzz)` normalized to [-1, 1].
3. Persist into the existing `sentiment_cache` table (already in schema).
4. Add a `Finnhub` client module (`engine/src/data/finnhub.rs`) — separates concerns
   and makes the LLM advisor's data source testable in isolation.
5. Required env var: `FINNHUB_API_KEY=d9nluo9r01qvumgasr6gd9nluo9r01qvumgasr70`. Without it, sentiment silently falls back
   to the stub (advisor labels it `sentiment_source: "stub"` in the briefing).
6. Add a sentiment time-series endpoint so the Advisor view can chart it alongside
   the features: `GET /api/sentiment/history?symbol=QQQ&days=30` returns
   `{date, score, buzz}[]`.

### Why Finnhub
Free tier 60 req/min covers our daily cadence. One API gives us sentiment + news
text + earnings calendar (used in §4.2 macro context). Adding a second provider
later is cheap — the advisor only needs `score` and `source_label`.

### Test
- Unit: parse a canned Finnhub response → score in [-1, 1].
- Unit: missing `FINNHUB_API_KEY` → falls back to stub, no panic.
- Integration: `GET /api/sentiment/history` returns 30 rows after a backfill.

---

## 4.1 Advisor Module (`engine/src/advisor/`)

### Design
- Promoted from a single file (`advisor.rs`) to a module so we can split concerns
  cleanly: `mod.rs`, `prompt.rs`, `context.rs`, `briefing.rs`, `chat.rs`.
- Reuses the `LlmRegimeConfig` + reqwest pattern from `features/llm.rs`.
- **Three modes**, one shared context builder:
  1. **Pre-market briefing** — generated once per weekday at 13:00 UTC (08:00 ET),
     well before the 13:30 UTC NYSE open. Cached, surfaced via WS to the dashboard.
  2. **Conversational chat** — on-demand, streamed via SSE. Has 5-turn history.
  3. **Disabled** — config flag off, or no API key → endpoints return 503 with
     `{"enabled": false, "reason": "..."}` so the UI shows a clear panel.

### Module structure
```
engine/src/advisor/
  mod.rs              — module root, public types, AdvisorState
  prompt.rs           — compile_prompt(), parse_response()
  context.rs          — build_context() from DB; defines AdvisorContext
  briefing.rs         — generate_briefing(), briefing loop, scheduling
  chat.rs             — handle chat requests, multi-turn history

engine/src/api/
  advisor.rs          — HTTP handlers
```

### Public types (`advisor/mod.rs`)
```rust
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// What the LLM sees as context. Built fresh per briefing/chat.
#[derive(Debug, Clone, Serialize)]
pub struct AdvisorContext {
    pub as_of: DateTime<Utc>,
    pub symbol: String,                          // e.g. "QQQ"
    pub market_session: String,                  // "Closed" | "PreMarket" | "Regular" | "AfterHours"
    pub next_open_utc: Option<i64>,
    pub next_close_utc: Option<i64>,
    pub is_trading_day: bool,
    pub holiday_name: Option<String>,

    // Predictions (from equity_predictions table)
    pub pred_1d: Option<f64>,                   // decimal return, e.g. 0.003 = +0.30%
    pub pred_5d: Option<f64>,
    pub pred_21d: Option<f64>,
    pub pred_1d_horizon_ts: Option<i64>,

    // Live feature vector (from latest equity_features row)
    // KEYS MATCH FeatureInspector.svelte exactly so the briefing and
    // the dashboard show the same labels.
    pub features: FeatureSnapshot,

    // Sentiment
    pub sentiment_score: Option<f64>,           // [-1, 1], None if unavailable
    pub sentiment_buzz: Option<f64>,            // articles/week, for context
    pub sentiment_source: String,               // "finnhub" | "stub"

    // Macro / calendar (next 7 days)
    pub macro: MacroContext,

    // Position state
    pub position_side: String,                  // "long" | "flat" | "short"
    pub position_qty: Option<f64>,
    pub entry_price: Option<f64>,
    pub entry_ts: Option<i64>,
    pub unrealized_pnl: Option<f64>,
    pub realized_pnl_session: Option<f64>,

    // Recent closed trades (last 5) for "are entries/exits justified" framing
    pub recent_trades: Vec<RecentTrade>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureSnapshot {
    pub trend_slope: f64,
    pub trend_adx: f64,
    pub rsi_14: f64,
    pub vix_regime: f64,                        // encoded level
    pub tlt_corr_20d: f64,
    pub rvol_20d: f64,
    pub gap_pct: f64,
    pub drawdown_from_50d_high: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MacroContext {
    pub ust_10y_latest: Option<f64>,            // FRED $UST10Y
    pub ust_10y_prev: Option<f64>,
    pub dxy_latest: Option<f64>,                // FRED $DXY
    pub dxy_prev: Option<f64>,
    pub vix_latest: Option<f64>,                // CBOE $VIX (most recent bar)
    pub earnings_in_next_7d: Vec<EarningsEvent>,
    pub macro_releases_in_next_7d: Vec<MacroRelease>,  // FOMC/CPI/NFP from Finnhub calendar
}

#[derive(Debug, Clone, Serialize)]
pub struct EarningsEvent {
    pub date: String,                           // "2026-08-05"
    pub hour_et: Option<String>,                // "BMO" | "AMC" | "DMH"
}

#[derive(Debug, Clone, Serialize)]
pub struct MacroRelease {
    pub date: String,
    pub event: String,                          // "CPI", "FOMC", "NFP"
    pub time_et: Option<String>,
    pub consensus: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentTrade {
    pub side: String,
    pub entry_ts: i64,
    pub exit_ts: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl: f64,
    pub bars_held: i64,
}

/// The LLM's response. **Always** parsed from a fenced ```json block,
/// not from raw response text. Falls back to a distinct error struct
/// (not a fake "hold") on parse failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorBriefing {
    pub model_used: String,
    pub as_of: DateTime<Utc>,
    pub for_date: String,                       // "YYYY-MM-DD" — the trading day this briefing covers
    /// Plain prose digest. Multiple short paragraphs.
    pub digest: String,
    /// Section headings the LLM is required to use, in this order.
    /// Missing sections → the digest is rejected as malformed.
    pub sections: BriefingSections,
    /// Warnings surface separately so the UI can render them as banners.
    pub warnings: Vec<String>,
    /// Optional structured metadata the LLM may include. NOT required.
    pub suggested_action: Option<String>,        // "hold_long" | "exit_long" | "enter_long" | ...
    pub suggested_confidence: Option<f64>,       // 0.0–1.0
    pub suggested_params: Option<serde_json::Value>,
    pub parse_status: String,                    // "ok" | "partial" | "failed"
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingSections {
    pub regime: String,                         // 1–2 sentences: what regime we are in
    pub predictions: String,                    // 1–2 sentences: what the model expects
    pub features: String,                       // 2–4 sentences: which features support / contradict
    pub sentiment: String,                      // 1–2 sentences: bullish/bearish + source
    pub macro: String,                          // 1–3 sentences: yields/DXY/VIX + upcoming events
    pub position_advice: String,                // 2–5 sentences: justified entries/exits + warnings
}

#[derive(Debug, Clone)]
pub enum BriefingError {
    ParseFailed { raw: String, reason: String },
    LlmUnavailable { status: u16, body: String },
    Disabled(String),
    RateLimited,
}

/// Cached briefing, keyed by (trading_date, model).
#[derive(Debug, Clone)]
pub struct CachedBriefing {
    pub briefing: AdvisorBriefing,
    pub context_hash: String,                   // sha256 of canonicalized AdvisorContext
}
```

### Context builder (`advisor/context.rs`)

```rust
/// Build the AdvisorContext from current DB state.
/// Pure read; never mutates anything.
pub async fn build_context(
    pool: &DbPool,
    symbol: &str,
) -> Result<AdvisorContext> {
    // 1. Latest equity_predictions row for `symbol` → pred_1d / 5d / 21d
    // 2. Latest equity_features row → FeatureSnapshot
    //    (the same row the FeatureInspector.svelte component fetches;
    //     if FeatureInspector sees X, the briefing sees X.)
    // 3. Latest sentiment_cache row → sentiment_score, buzz, source
    // 4. Macro: latest equity_candles row for $UST10Y, $DXY, $VIX
    // 5. Earnings + macro calendar from Finnhub cache (or omit if absent)
    // 6. Position: latest open position from positions table (long/flat/short)
    // 7. Last 5 closed trades from equity_trades where status='closed'
    // 8. Market session via market_hours::current_state()
}
```

**Critical**: the feature fetch in step 2 MUST hit the same endpoint as
`FeatureInspector.svelte` (`/api/equity/features?symbol=QQQ&limit=1`) or call the
same DB query it wraps. If the briefing's `trend_adx` is 25 and the dashboard's
is 27, the user immediately loses trust in the advisor. Implementation: factor the
fetch into a single `db::fetch_latest_feature_snapshot()` shared by both.

### Prompt design (`advisor/prompt.rs`)

The prompt is a **prose-digest** task, not a structured-action task. Action/confidence
metadata is optional and secondary.

```rust
pub fn compile_system_prompt() -> &'static str {
    // ~50 lines. Key rules:
    //  - Use the section headings verbatim: REGIME / PREDICTIONS / FEATURES /
    //    SENTIMENT / MACRO / POSITION ADVICE. Each section is 1-5 sentences.
    //  - REGIME: one of {Bullish, Bearish, Neutral, Volatile, Crash-risk}.
    //    Define each in 1 line (e.g. "Bullish: SMA200 slope > 0 AND ADX > 25").
    //  - FEATURES: call out which named features SUPPORT the model's prediction
    //    and which CONTRADICT it. Use the labels from the context verbatim
    //    (trend_slope, trend_adx, rsi_14, vix_regime, tlt_corr_20d, rvol_20d,
    //    gap_pct, drawdown_from_50d_high).
    //  - SENTIMENT: state the score, the source (finnhub or stub), and whether
    //    it agrees with the prediction. If source is "stub", say so explicitly.
    //  - MACRO: include $UST10Y, $DXY, $VIX levels and any earnings/FOMC/CPI
    //    in the next 7 days.
    //  - POSITION ADVICE: justify the current position or recommend an action.
    //    Explicitly call out warnings (e.g. earnings in 2 days, VIX > 30,
    //    feature conflict, sentiment stubbed, model stale >24h).
    //  - Output must be a JSON block wrapped in ```json ... ``` fences.
    //  - Do NOT recommend specific entry/exit prices. Do NOT suggest
    //    parameters outside the documented set.
}

pub fn compile_user_prompt(ctx: &AdvisorContext) -> String {
    // Serializes the AdvisorContext into the user message.
    // Explicit units on every numeric field:
    //   "pred_1d: +0.30% (positive = bullish)"
    //   "rsi_14: 58.2 (0-100 scale, >70 overbought, <30 oversold)"
    //   "vix_regime: 1 (encoded level; 0=calm, 1=normal, 2=elevated, 3=panic)"
    //   "sentiment_score: +0.42 (range -1 to +1)"
    //   "tlt_corr_20d: -0.31 (Pearson, -1 to +1)"
    //   "drawdown_from_50d_high: -0.04 (decimal, negative = below high)"
    //   "$UST10Y: 4.32% (latest vs prev 4.28%)"
    //   "$VIX: 18.4 (latest)"
    // No "feature_names: [...]" parallel array — names are inline with values.
}
```

**Why prose, not JSON action**: you explicitly said "verbal digest ... giving
justification ... warnings based on data and current macros." That is a writing
task, not a classification task. Trying to coerce it into `{action: ...,
confidence: 0.7}` lost the reasoning. We still capture `suggested_action` /
`suggested_confidence` as optional metadata, derived from the prose by the LLM
itself (e.g. "Suggested action: hold_long. Confidence: medium.").

### Response parsing (`advisor/prompt.rs`)

```rust
pub fn parse_response(raw: &str, model: &str, as_of: DateTime<Utc>,
                      for_date: String) -> Result<AdvisorBriefing, BriefingError> {
    // 1. Extract the first ```json ... ``` fenced block. If none, try the
    //    whole response as JSON. If still no JSON, return ParseFailed.
    // 2. serde_json::from_str into a temp struct that REQUIRES all 6 section
    //    fields to be present and non-empty. Missing section → ParseFailed.
    // 3. Validate `for_date` is a valid YYYY-MM-DD and matches a trading day
    //    (else ParseFailed — model is hallucinating).
    // 4. If suggested_params is present, validate every key is in the
    //    documented parameter set (see §4.6). Unknown key → strip it, set
    //    warnings. Don't reject.
    // 5. Return AdvisorBriefing with parse_status="ok".
    //
    // On any failure: return BriefingError::ParseFailed with raw + reason.
    // NEVER silently substitute "hold" with confidence 0.
}
```

### Caching strategy (`advisor/briefing.rs`)

```rust
pub struct AdvisorState {
    pub cfg: AdvisorConfig,
    pub api_key: String,
    pub model: String,
    pub api_base: String,
    pub cache: Arc<RwLock<HashMap<String, CachedBriefing>>>,  // key: for_date
    pub notify: Arc<Notify>,                                  // wake loop early
}
```

- Cache key is `for_date` (the trading day the briefing covers), not `as_of`.
  One briefing per trading day, regenerated only if invalidated.
- Cache TTL: 6 hours default. After that the cache is stale but still served
  with a `parse_status` warning so the UI can render "stale (Xh)".
- The briefing loop is `tokio::select!` over `interval.tick()` and
  `notify.notified()`. The scheduler signals `notify` on a new prediction so a
  pre-market briefing can refresh mid-day if predictions shift.
- A new market session (PreMarket → Regular) does NOT auto-invalidate. The
  briefing covers the whole day; position advice at 13:00 UTC may differ from
  18:00 UTC but the briefing as a unit is still useful.
- The LLM call is NEVER in the per-bar latency path. Confirmed by design —
  background task only.

### Config additions (`engine/src/config.rs`)

```rust
pub advisor_enabled: bool,                        // default false
pub advisor_model: String,                        // "openrouter/google/gemini-3.1-pro-preview"
pub advisor_api_key: String,                      // OPENROUTER_API_KEY
pub advisor_api_base: String,                     // OpenRouter endpoint
pub advisor_fallback_model: String,               // "openrouter/anthropic/claude-3.5-sonnet"
pub advisor_briefing_hour_utc: u32,               // 13 = 08:00 ET pre-market
pub advisor_chat_history_turns: usize,            // default 5
pub advisor_chat_rate_limit_per_min: u32,         // default 10
```

Disabled states:
- `advisor_enabled=false` → endpoints return `{"enabled": false}` with 200.
- `advisor_enabled=true` but `OPENROUTER_API_KEY` empty → log warning, same 200 response with reason.

---

## 4.2 API Endpoints (`engine/src/api/advisor.rs`)

### `GET /api/advisor/briefing?date=YYYY-MM-DD`

- `date` defaults to today (in ET, so a 22:00 UTC request gets the next trading day).
- Returns the cached briefing for that date. If none:
  - If `advisor_enabled=false` or no API key → `200 {"enabled": false, "reason": "..."}`.
  - Else → `503 {"error": "briefing_pending", "next_attempt_utc": ...}`.
- Response shape matches `AdvisorBriefing` from §4.1.

### `POST /api/advisor/ask`

```json
// Request
{ "question": "Why did the model exit the long position on Friday?",
  "date": "2026-08-04" }
```

- Streams the LLM response via SSE: `data: {"token": "..."}` per token.
- Final event: `data: {"done": true, "response": {"full_prose": "...", "warnings": [...]}}`.
- Errors: emit `data: {"error": "..."}` then `data: {"done": true}`. Never leave
  the stream hanging.
- The chat history (last `advisor_chat_history_turns` Q/A pairs) is loaded from
  `advisor_chat_log` and prepended to the prompt. Multi-turn context, but bounded.
- **Rate limit**: per-IP token bucket, `advisor_chat_rate_limit_per_min`. Exceeded
  → `429` before streaming starts.

### `GET /api/sentiment/history?symbol=QQQ&days=30`
Already added in §4.0. Listed here so the frontend knows where to fetch it.

### Route registration (`engine/src/api/mod.rs`)

```rust
.route("/api/advisor/briefing", get(advisor::handle_get_briefing))
.route("/api/advisor/ask",     post(advisor::handle_ask))
.route("/api/sentiment/history", get(advisor::handle_sentiment_history))
```

### Files to create/modify
- **CREATE** `engine/src/advisor/{mod,prompt,context,briefing,chat}.rs`
- **CREATE** `engine/src/data/finnhub.rs` (prerequisite §4.0)
- **CREATE** `engine/src/api/advisor.rs`
- **MODIFY** `engine/src/api/mod.rs` — add `mod advisor;`, register routes
- **MODIFY** `engine/src/lib.rs` — add `pub mod advisor;`
- **MODIFY** `engine/src/config.rs` — add advisor config fields
- **MODIFY** `engine/src/main.rs` — spawn advisor background task
- **MODIFY** `engine/src/db.rs` — add `advisor_chat_log` table + helpers

---

## 4.3 Briefing Background Task

### In `main.rs` (after scheduler spawn)

```rust
if cfg.advisor_enabled {
    let advisor_state = AdvisorState::new(cfg.clone());
    tokio::spawn(advisor::briefing::run_briefing_loop(
        advisor_state,
        pool.clone(),
        cfg.symbol.clone(),
        scheduler_notify.clone(),  // forward PredictionUpdate notifications
    ));
    info!(model = %cfg.advisor_model, "advisor background task started");
}
```

### Briefing loop

```rust
pub async fn run_briefing_loop(
    state: AdvisorState,
    pool: DbPool,
    symbol: String,
    scheduler_notify: Arc<Notify>,
) {
    // 1. Wait until next briefing hour (or run immediately if past).
    let mut next_run = compute_next_briefing_ts(state.cfg.advisor_briefing_hour_utc);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(next_run) => {}
            _ = state.notify.notified() => {
                // Manual refresh — re-run briefing for today now.
                next_run = compute_next_briefing_ts(state.cfg.advisor_briefing_hour_utc);
                continue;
            }
        }

        // 2. Skip if today is not a trading day.
        let today = trading_date_et();
        if !market_hours::is_trading_day(today.year(), today.month(), today.day()) {
            tracing::info!(date = %today, "non-trading day; skipping briefing");
            next_run = compute_next_briefing_ts(state.cfg.advisor_briefing_hour_utc);
            continue;
        }

        // 3. Build context.
        let context = match build_context(&pool, &symbol).await {
            Ok(c) => c,
            Err(e) => { warn!(error = %e, "advisor context build failed"); continue; }
        };

        // 4. Call LLM (primary, then fallback on failure).
        let (raw, model_used) = match call_llm_with_fallback(&state, &context).await {
            Ok(r) => r,
            Err(e) => { warn!(error = %e, "advisor LLM call failed"); continue; }
        };

        // 5. Parse.
        let briefing = match parse_response(&raw, &model_used, Utc::now(), today.to_string()) {
            Ok(b) => b,
            Err(BriefingError::ParseFailed { raw, reason }) => {
                // Log the failure but still cache a degraded entry.
                let degraded = AdvisorBriefing {
                    parse_status: "failed".into(),
                    parse_error: Some(reason.clone()),
                    digest: raw,  // show the raw model output
                    ..default_failed_briefing()
                };
                state.cache.write().await.insert(today.to_string(), CachedBriefing {
                    briefing: degraded,
                    context_hash: hash_context(&context),
                });
                warn!(reason = %reason, "advisor parse failed; cached degraded briefing");
                continue;
            }
            Err(e) => { warn!(error = ?e, "advisor parse failed"); continue; }
        };

        // 6. Cache + log + broadcast.
        state.cache.write().await.insert(today.to_string(), CachedBriefing {
            briefing: briefing.clone(),
            context_hash: hash_context(&context),
        });
        db::insert_advisor_log(&pool, "briefing", &context, &briefing).await.ok();

        // 7. Push to dashboard via WS (TelemetryEvent::AdvisorBriefing).
        let _ = state.tx.send(TelemetryEvent::AdvisorBriefing {
            for_date: today.to_string(),
            briefing: briefing.clone(),
        });

        next_run = compute_next_briefing_ts(state.cfg.advisor_briefing_hour_utc);
    }
}
```

### Skipping non-trading days
You said: "do nothing, especially on weekends, if the market is closed." The loop
explicitly checks `is_trading_day` and skips. No briefing is generated on
weekends or holidays. On Monday morning, the loop fires at 13:00 UTC and
generates the Monday briefing.

---

## 4.4 DB Additions (`engine/src/db.rs`)

### `advisor_briefing_log` (one row per successful or attempted briefing)
```sql
CREATE TABLE IF NOT EXISTS advisor_briefing_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    for_date     TEXT    NOT NULL,
    model_used   TEXT    NOT NULL,
    context_hash TEXT    NOT NULL,
    context_json TEXT    NOT NULL,
    briefing_json TEXT   NOT NULL,
    parse_status TEXT    NOT NULL,        -- 'ok' | 'partial' | 'failed'
    latency_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS advisor_briefing_log_date_idx
    ON advisor_briefing_log (for_date DESC);
```

### `advisor_chat_log` (multi-turn history source)
```sql
CREATE TABLE IF NOT EXISTS advisor_chat_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    question     TEXT    NOT NULL,
    response     TEXT    NOT NULL,
    model_used   TEXT    NOT NULL,
    latency_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS advisor_chat_log_ts_idx
    ON advisor_chat_log (ts DESC);
```

### Helpers
```rust
pub async fn insert_advisor_briefing_log(...) -> Result<i64>;
pub async fn insert_advisor_chat_log(...) -> Result<i64>;
pub async fn fetch_recent_chat_turns(pool: &DbPool, n: usize) -> Result<Vec<ChatTurn>>;
```

Add to the same migration file as the existing schema (`db.rs` line ~22).
Do NOT create a one-off migration file.

---

## 4.5 Frontend: Advisor View (`frontend/src/views/Advisor.svelte`)

### Layout
```
+------------------------------------------------------------------+
|  AI Advisor                                [Briefing for: 2026-08-04]
+------------------------------------------------------------------+
|  Latest Briefing                                  [refresh] [open in lab]
+------------------------------------------------------------------+
|  REGIME        |  Bullish: SMA200 slope positive, ADX 31.        |
|                |  Confirmed by VIX in normal range (18.4).       |
+----------------+-------------------------------------------------+
|  PREDICTIONS   |  Model: +0.30% 1D, +1.20% 5D, +3.10% 21D.       |
+----------------+-------------------------------------------------+
|  FEATURES      |  trend_slope: +0.18 (supports long)              |
|                |  trend_adx: 31.0 (trending, supports long)      |
|                |  rsi_14: 58.2 (neutral)                          |
|                |  vix_regime: 1 (normal)                          |
|                |  drawdown_from_50d_high: -0.04 (slight pullback) |
|                |  CONFLICT: gap_pct +1.2% suggests gap-up but     |
|                |  rvol_20d 0.8 shows below-avg participation.     |
+----------------+-------------------------------------------------+
|  SENTIMENT     |  Finnhub: +0.42 (bullish, 47 articles/wk).       |
|                |  Agrees with model direction.                    |
+----------------+-------------------------------------------------+
|  MACRO         |  $UST10Y 4.32% (+4bp wow). $DXY 104.1 (-0.3 wow).|
|                |  $VIX 18.4 (normal).                             |
|                |  UPCOMING: FOMC Wed, NVDA earnings Thu AMC.      |
+----------------+-------------------------------------------------+
|  POSITION ADVICE  Hold current long. Entry justified by trend +  |
|                  sentiment agreement. Watch for FOMC volatility   |
|                  Wed — consider tightening stop if VIX > 25       |
|                  intraday. Suggested params: tighter stop, no     |
|                  change to entry_threshold.                       |
+------------------------------------------------------------------+
|  WARNINGS  [FOMC in 2 days]  [gap_pct conflicts with rvol]       |
+------------------------------------------------------------------+

  Ask a follow-up:
  +--------------------------------------------------------------+
  | Why did the model exit the long position?                    |
  |                                                              |
  |                                                       [Send] |
  +--------------------------------------------------------------+
  Response (streaming):                                          |
  "The model exited the long position because pred_1d fell..."   |
```

### Components
```
frontend/src/
  views/Advisor.svelte
  lib/components/
    AdvisorBriefing.svelte       # Sectioned prose digest, warnings banner
    AdvisorSection.svelte        # Single REGIME/PREDICTIONS/... block
    WarningsBanner.svelte        # Yellow/red list of warnings
    AdvisorChat.svelte           # Question input + SSE streaming
    SuggestedParamsActions.svelte # "Test in Strategy Lab" + "Copy params"
    AdvisorDisabledState.svelte  # "Advisor disabled — set OPENROUTER_API_KEY"
```

### SSE streaming
```js
async function askAdvisor(question) {
    const controller = new AbortController();
    const response = await fetch('/api/advisor/ask', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ question, date: today }),
        signal: controller.signal,
    });

    // User navigates away → abort. Otherwise the LLM call keeps billing.
    cleanup(() => controller.abort());

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop();
        for (const line of lines) {
            if (!line.startsWith('data: ')) continue;
            try {
                const data = JSON.parse(line.slice(6));
                if (data.token) streamingText.update(t => t + data.token);
                if (data.error) chatError.set(data.error);
                if (data.done) finalResponse.set(data.response);
            } catch (e) { /* skip malformed line, don't kill stream */ }
        }
    }
}
```

### Strategy Lab integration
- "Test in Strategy Lab →" button: sets a Svelte store
  `suggestedParamsFromAdvisor` and `goto('/strategy-lab')`.
- `StrategyLab.svelte` already exists; on mount, if the store is non-null, prefill
  the params form and scroll to it.
- If `parse_status === 'failed'` or `suggested_params` is null, hide the button.

### Disabled state
When `GET /api/advisor/briefing` returns `{enabled: false}`:
- Render `AdvisorDisabledState` with the `reason` string.
- Hide the chat input.
- Hide the refresh button.

### Feature parity with FeatureInspector
Both the dashboard FeatureInspector and the AdvisorBriefing MUST display the
same feature labels (trend_slope, trend_adx, ...). Source of truth: the
`FEATURE_DEFS` array currently in `FeatureInspector.svelte`. Move that to
`frontend/src/lib/feature_defs.js` and import from both. If we ever rename a
feature, both update together.

---

## 4.6 Suggested Parameters (`AdvisorBriefing.suggested_params`)

The LLM may only suggest parameters that exist in `EquityStrategyParams`. The
documented schema is included in the system prompt:

```
AVAILABLE STRATEGY PARAMETERS (suggest only these — others will be silently dropped):
- entry_threshold: f64, default 0.003. Trigger magnitude for new long entries.
- exit_threshold:  f64, default -0.001. Trigger for closing longs.
- short_entry_threshold: f64, default -0.004. Trigger magnitude for new shorts.
- short_exit_threshold:  f64, default 0.001. Trigger for closing shorts.
- sma_window: int, default 40. SMA lookback for trend regime.
- enable_shorting: bool, default false. Allow short-side entries.
- pred_5d_filter: bool, default false. Require pred_5d to agree with pred_1d before entry.
```

The parser strips unknown keys silently. The UI shows what survived with a
tooltip explaining why the rest were dropped.

---

## 4.7 Test Requirements

### Backend
- Unit: `build_context` produces a struct with all fields populated when DB has
  full data.
- Unit: `build_context` handles missing sentiment, missing macro, no recent
  trades, missing predictions (returns `None` for optional fields, never panics).
- Unit: `compile_system_prompt` contains the six required section headings.
- Unit: `parse_response` succeeds on a canned well-formed response.
- Unit: `parse_response` returns `ParseFailed` on missing section.
- Unit: `parse_response` returns `ParseFailed` on invalid `for_date`.
- Unit: `parse_response` strips unknown param keys but keeps the briefing.
- Integration: briefing generation flow end-to-end with mocked reqwest (no real
  OpenRouter call).
- Integration: `GET /api/advisor/briefing` returns the cached entry after one
  is written to `AdvisorState.cache`.
- Integration: `GET /api/advisor/briefing?date=YYYY-MM-DD` returns 503 when no
  briefing cached and `enabled=true`.
- Integration: `GET /api/advisor/briefing` returns `{enabled: false, reason}`
  when key missing.
- Integration: chat SSE emits token events and a final `done` event.
- Integration: chat rate limit returns 429 after the per-minute threshold.
- Integration: non-trading day → briefing loop skips without writing a row.

### Frontend
- Manual: Briefing sections render in order; warnings render in red banner.
- Manual: Disabled state renders when API key absent.
- Manual: Chat streaming shows tokens incrementally; abort works on nav-away.
- Manual: "Test in Strategy Lab" pre-fills the lab form.
- Manual: Feature labels in briefing match FeatureInspector labels exactly.

---

## 4.8 Risk Notes

### LLM hallucination of facts
The LLM may invent macro levels, sentiment scores, or dates. Mitigation:
- **Numeric facts in the user prompt are wrapped in quoted lines the LLM is told
  not to repeat from memory** ("quote the value EXACTLY as given").
- Warnings array is required to mention any context field marked `source: "stub"`
  (so the user sees "sentiment_source: stub" in the briefing).
- Failed parse → briefing cached with `parse_status: "failed"` and the UI shows
  a "Briefing could not be parsed — last attempt [timestamp]" banner. The
  advisor is never silently wrong.

### LLM hallucination of parameters
Mitigated by the documented parameter set in §4.6. Unknown keys are stripped
before the briefing is cached.

### Stale features
If `equity_features` latest row is older than 2 hours during market hours, the
advisor context includes `features_staleness_secs`. The system prompt instructs
the model to mention staleness in the FEATURES or POSITION ADVICE section.

### Cost (revised math)
Pre-market briefing: ~800 tokens input context + ~400 tokens output. At
`openrouter/google/gemini-3.1-pro-preview` rates (~$0.50/$2.00 per M tokens
estimated), one briefing = ~$0.002. 252 trading days = ~$0.50/year.
Chat: user-controlled, ~5 turns/day × ~600 tokens = $0.06/day if everyone uses
it constantly. Negligible.

### SSE through reverse proxy
Nginx/Caddy MUST disable buffering for `/api/advisor/ask`:
```
location /api/advisor/ask {
    proxy_buffering off;
    proxy_cache off;
    proxy_set_header Connection '';
    proxy_http_version 1.1;
}
```
Caddy: `reverse_proxy localhost:8080 { flush_interval -1 }` on that path.

### Finnhub rate limits (60 req/min free)
Both the sentiment fetch (§4.0) and the earnings/macro calendar fetch (§4.1
MacroContext) hit Finnhub. Cache both aggressively:
- Sentiment: refresh once per trading day, after the morning briefing.
- Calendar: refresh once per hour (events don't change frequently).
- 2 calls/min sustained, well under 60 limit.

### Config flag absent on existing deployments
`advisor_enabled` defaults to `false`. Existing deployments that don't set
`ADVISOR_ENABLED=true` continue to work unchanged. No migration is required.

### Feature drift between dashboard and advisor
If a future change renames a feature in `FeatureInspector` but not in
`AdvisorContext`, the briefing labels go stale silently. Mitigation:
- The shared `lib/feature_defs.js` (frontend) and the shared
  `db::fetch_latest_feature_snapshot` (backend) make drift impossible by
  construction.
- Add a unit test: `FEATURE_DEFS.length === FEATURE_DIM` and the keys match
  the `FeatureRow` struct fields exactly.