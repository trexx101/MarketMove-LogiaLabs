# Phase 4 — AI Trading Advisor

**Goal**: Build an LLM-powered trading advisor that consumes the model's predictions, live
features, position, and PnL — then produces a daily briefing and answers conversational
questions. The advisor is strictly advisory (zero execution authority). Suggested parameter
changes can be one-click ported to the Strategy Lab for backtesting.

**Estimated effort**: ~2 weeks
**Can deploy independently**: Yes — the advisor is an additive overlay.
**Depends on**: Phase 0 (DB tables, API module tree), Phase 1 (AppState, WS), Phase 2 (Strategy Lab for param suggestion integration)

---

## 4.1 Advisor Module (`engine/src/advisor.rs`)

### Design
- Reuses the existing `engine/src/features/llm.rs` OpenRouter client pattern.
- Two interaction modes:
  1. **Hourly briefing** — automated, cached in `RwLock`, stored in `advisor_log`.
  2. **Conversational chat** — on-demand, streamed via SSE.
- The advisor is completely decoupled from the execution layer. It can only suggest, never act.

### Module structure
```
engine/src/
  advisor.rs          — LLM advisor: briefing generation, chat, prompt compilation
  api/
    advisor.rs        — HTTP handlers: GET /api/advisor/briefing, POST /api/advisor/ask
```

### Core types
```rust
// engine/src/advisor.rs
use serde::{Serialize, Deserialize};

/// System state snapshot sent to the LLM as context.
pub struct AdvisorContext {
    pub regime: String,           // "Bullish" | "Bearish" | "Unknown"
    pub position: String,         // "long" | "flat" | "short"
    pub pred_1d: Option<f64>,
    pub pred_5d: Option<f64>,
    pub pred_21d: Option<f64>,
    pub features: [f64; 8],       // raw feature values
    pub feature_names: [&'static str; 8],
    pub realized_pnl: f64,
    pub unrealized_pnl: Option<f64>,
    pub entry_price: Option<f64>,
    pub current_close: Option<f64>,
}

/// Structured LLM response (parsed from JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorResponse {
    pub action: String,           // "hold" | "exit" | "alert" | "enter_long" | "enter_short"
    pub confidence: f64,          // 0.0–1.0
    pub reasoning: String,
    pub suggested_params: Option<serde_json::Value>,  // e.g. {"entry_threshold": 0.004}
}

/// Cached briefing.
pub struct CachedBriefing {
    pub timestamp: i64,
    pub response: AdvisorResponse,
    pub model_used: String,
}
```

### Prompt template
```
You are an expert quantitative advisor analyzing the MarketMarkovNet daily equities model.

CURRENT STATE:
- Market Regime: {regime}
- Current Position: {position}
- Model Predictions: 1D: {pred_1d}, 5D: {pred_5d}, 21D: {pred_21d}
- Live Feature Vector:
  [Slope: {f0}, ADX: {f1}, RSI: {f2}, VIX Regime: {f3},
   TLT Corr: {f4}, RVol: {f5}, Gap: {f6}, DD: {f7}]
- Realized PnL: {realized_pnl}
- Unrealized PnL: {unrealized_pnl}
- Entry Price: {entry_price}
- Current Close: {current_close}

Analyze the features against the predictions. Does the volatility context (VIX) or
momentum (RSI) contradict the model's directional forecast?

Output your response in valid JSON matching this schema:
{
  "action": "hold" | "exit" | "alert" | "enter_long" | "enter_short",
  "confidence": 0.0-1.0,
  "reasoning": "...",
  "suggested_params": {}
}
```

### Briefing generation flow
```rust
/// Generate a briefing by calling OpenRouter. Called by a background task
/// on an hourly timer (like llm.rs). Result is cached + logged.
pub async fn generate_briefing(
    context: &AdvisorContext,
    api_key: &str,
    model: &str,
    api_base: &str,
) -> Result<AdvisorResponse> {
    let prompt = compile_prompt(context);
    let response = call_openrouter(api_key, model, api_base, &prompt).await?;
    let parsed: AdvisorResponse = serde_json::from_str(&response)
        .unwrap_or(AdvisorResponse {
            action: "hold".into(),
            confidence: 0.0,
            reasoning: "Failed to parse LLM response".into(),
            suggested_params: None,
        });
    Ok(parsed)
}
```

### Caching strategy
- Process-wide `RwLock<Option<CachedBriefing>>` — same pattern as `llm.rs`.
- Background task wakes every hour (configurable `ADVISOR_CACHE_TTL_SECONDS`, default 3600).
- On new prediction (scheduler publishes `PredictionUpdate`), the cache is invalidated
  and a new briefing is generated on the next tick.
- The LLM call is NEVER in the per-bar latency path.

### Config additions (`engine/src/config.rs`)
```rust
pub struct Config {
    // ... existing fields ...
    pub advisor_enabled: bool,           // default false
    pub advisor_model: String,           // "openrouter/deepseek/deepseek-v4-flash"
    pub advisor_api_key: String,         // OPENROUTER_API_KEY
    pub advisor_api_base: String,        // OpenRouter endpoint
    pub advisor_cache_ttl: u64,          // 3600 seconds
    pub advisor_fallback_model: String,  // "openrouter/anthropic/claude-3.5-sonnet"
}
```

---

## 4.2 API Endpoints (`engine/src/api/advisor.rs`)

### `GET /api/advisor/briefing`
```json
// Response:
{
  "timestamp": 1718920000,
  "action": "hold",
  "confidence": 0.8,
  "reasoning": "The model predicts a modest 1D return of 0.003...",
  "suggested_params": { "entry_threshold": 0.004 },
  "model_used": "deepseek-v4-flash"
}
```
- Returns the cached briefing. If no briefing exists yet, returns 503.

### `POST /api/advisor/ask`
```json
// Request:
{ "question": "Why did the model exit the long position?" }

// Response: Server-Sent Events stream
data: {"token": "The "}
data: {"token": "model "}
data: {"token": "exited "}
data: {"token": "because "}
...
data: {"done": true, "response": {"action": "hold", "confidence": 0.7, "reasoning": "..."}}
```
- Streams the LLM response token-by-token via SSE.
- The full response is parsed into `AdvisorResponse` JSON at the end.
- Both the question and response are logged to `advisor_log`.

### Route registration
```rust
.route("/api/advisor/briefing", get(advisor::handle_get_briefing))
.route("/api/advisor/ask", post(advisor::handle_ask))
```

### Files to create/modify
- **CREATE** `engine/src/advisor.rs`
- **CREATE** `engine/src/api/advisor.rs`
- **MODIFY** `engine/src/api/mod.rs` — add `mod advisor;`, register routes
- **MODIFY** `engine/src/lib.rs` — add `pub mod advisor;`
- **MODIFY** `engine/src/config.rs` — add advisor config fields
- **MODIFY** `engine/src/main.rs` — spawn advisor background task if enabled

---

## 4.3 Advisor Background Task

### In `main.rs` (after scheduler spawn)
```rust
if cfg.advisor_enabled {
    let advisor_state = advisor::AdvisorState::new(
        cfg.advisor_api_key.clone(),
        cfg.advisor_model.clone(),
        cfg.advisor_api_base.clone(),
        cfg.advisor_cache_ttl,
    );
    tokio::spawn(advisor::run_briefing_loop(
        advisor_state,
        pool.clone(),
        cfg.symbol.clone(),
        cfg.sma_window,
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
    sma_window: usize,
) {
    loop {
        // 1. Gather current context from DB
        let context = build_context(&pool, &symbol, sma_window).await;

        // 2. Call LLM
        match generate_briefing(&context, &state.api_key, &state.model, &state.api_base).await {
            Ok(response) => {
                // 3. Cache it
                state.set_cached(CachedBriefing {
                    timestamp: Utc::now().timestamp(),
                    response: response.clone(),
                    model_used: state.model.clone(),
                });

                // 4. Log to advisor_log
                db::insert_advisor_log(&pool, "briefing", &context, &response, &state.model).await;

                // 5. Broadcast via WS (optional — push briefing to dashboard)
                // state.tx.send(TelemetryEvent::AdvisorBriefing { ... });
            }
            Err(e) => warn!(error = %e, "advisor briefing failed"),
        }

        // 6. Sleep until next cycle
        tokio::time::sleep(Duration::from_secs(state.cache_ttl)).await;
    }
}
```

---

## 4.4 Frontend: Advisor View (`frontend/src/views/Advisor.svelte`)

### Layout
```
+------------------------------------------------------------------+
| AI Trading Advisor                                               |
+------------------------------------------------------------------+
| Latest Briefing (updated hourly)                                 |
+------------------------------------------------------------------+
| Action: HOLD        Confidence: 80%                              |
| Model: DeepSeek V4 Flash                                        |
| Timestamp: 2026-07-28 14:00 UTC                                 |
+------------------------------------------------------------------+
| "The model predicts a modest 1D return of 0.003 with a bullish  |
| SMA200 regime. VIX is elevated at 22 but within normal range.    |
| RSI at 58 shows no overbought condition. Recommend holding the   |
| current long position. Consider tightening entry_threshold to    |
| 0.004 to reduce false entries in this volatility regime."       |
+------------------------------------------------------------------+
| Suggested Parameters:                                            |
|   entry_threshold: 0.004                                         |
|   [Test in Strategy Lab →]  ← one-click port to backtest        |
+------------------------------------------------------------------+
| Ask the Advisor:                                                 |
| +--------------------------------------------------------------+ |
| | Why did the model exit the long position?                    | |
| |                                                              | |
| | [Send]                                                       | |
| +--------------------------------------------------------------+ |
|                                                                  |
| Response (streaming):                                           |
| "The model exited the long position because pred_1d fell to    |
| -0.002, below the exit_threshold of -0.001. This coincided..." |
+------------------------------------------------------------------+
```

### Components
```
frontend/src/
  views/Advisor.svelte
  lib/components/
    BriefingCard.svelte       # Action badge, confidence, reasoning text
    SuggestedParams.svelte    # Param chips with "Test in Strategy Lab" button
    AdvisorChat.svelte        # Question input + SSE streaming response
```

### SSE streaming in Svelte
```js
async function askAdvisor(question) {
    const response = await fetch('/api/advisor/ask', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ question }),
    });

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        // Parse SSE events
        const lines = buffer.split('\n');
        buffer = lines.pop(); // keep incomplete line

        for (const line of lines) {
            if (line.startsWith('data: ')) {
                const data = JSON.parse(line.slice(6));
                if (data.token) {
                    // Append token to streaming response
                    streamingText.update(t => t + data.token);
                }
                if (data.done) {
                    // Final structured response
                    finalResponse.set(data.response);
                }
            }
        }
    }
}
```

### Strategy Lab integration
- The "Test in Strategy Lab" button navigates to `/strategy-lab` and pre-fills the
  parameter form with the advisor's suggested params.
- Uses Svelte stores to pass the params between views:
  ```js
  // stores.js
  export const suggestedParams = writable(null);
  ```
- `SuggestedParams.svelte` sets the store and navigates:
  ```js
  function testInStrategyLab() {
    suggestedParams.set(briefing.suggested_params);
    goto('/strategy-lab');
  }
  ```
- `StrategyLab.svelte` reads `suggestedParams` on mount and pre-fills the form.

---

## 4.5 DB functions for advisor_log (`engine/src/db.rs`)

```rust
pub async fn insert_advisor_log(
    pool: &DbPool,
    interaction_type: &str,
    context: &AdvisorContext,
    response: &AdvisorResponse,
    model: &str,
) -> Result<()>;

pub async fn fetch_advisor_logs(
    pool: &DbPool,
    limit: usize,
) -> Result<Vec<AdvisorLogRow>>;
```

---

## 4.6 Test requirements

### Backend
- Unit test: `compile_prompt` produces the expected text with all context fields filled.
- Unit test: `AdvisorResponse` deserialization from valid JSON.
- Unit test: `AdvisorResponse` deserialization fallback on invalid JSON (returns "hold" + 0.0).
- Integration test: `GET /api/advisor/briefing` returns 503 when no briefing is cached.
- Integration test: `GET /api/advisor/briefing` returns cached briefing after generation.
- Integration test: `POST /api/advisor/ask` streams SSE tokens.
- Integration test: advisor_log table receives entries for both briefings and chats.
- Mock test: LLM call is mocked (don't hit real API in tests) — use a test fixture response.

### Frontend
- Manual: Briefing card displays action, confidence, reasoning.
- Manual: "Test in Strategy Lab" button navigates and pre-fills params.
- Manual: Chat input sends a question, response streams token-by-token.
- Manual: When advisor is disabled (config), the Advisor view shows "disabled" state.

---

## 4.7 Risk notes

- **LLM hallucination**: The advisor may suggest nonsensical params. Mitigation: the
  advisor has ZERO execution authority. All suggestions must be backtested in the Strategy
  Lab before the user can apply them. The UI clearly labels suggestions as "unverified".
- **OpenRouter API key**: The existing `llm.rs` already uses `OPENROUTER_API_KEY`. The
  advisor reuses the same key. If the key is missing, the advisor gracefully disables.
- **SSE through reverse proxy**: Nginx/Caddy must buffer SSE responses correctly. Verify
  proxy config disables buffering for `/api/advisor/ask` (e.g. `proxy_buffering off`).
- **Cost**: DeepSeek V4 Flash at $0.14/$0.28 per M tokens, hourly briefings (~2K tokens
  per call) = ~$0.01/day. Negligible. Chat is on-demand and user-controlled.
- **Model availability**: DeepSeek V4 Flash may not always be available on OpenRouter.
  Mitigation: implement fallback to `claude-3.5-sonnet` (more expensive but reliable).
  The `advisor.rs` module should try the primary model, and on failure, try the fallback.
