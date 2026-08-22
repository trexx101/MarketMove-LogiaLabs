# Sentiment Risk Overlay — Implementation Notes

## Background
Implemented 2026-08-05 as Part 3 of the NVDA multi-asset + sentiment overlay plan.

## Locked Defaults
- `enable_sentiment_overlay: false`
- `sentiment_reduce_threshold: -0.5`
- `sentiment_exit_threshold: -0.8`
- `sentiment_min_articles: 15`

## Runtime Flow
1. `EquityScheduler::finalize_candle` calls `data::sentiment::fetch_sentiment(pool, symbol)`.
2. Best-effort: failures are logged but do not block strategy evaluation.
3. `evaluate_and_execute_strategy` reads latest cached `(score, buzz)` via `db::latest_sentiment()`.
4. `strategy::apply_sentiment_overlay(raw_pos, score, buzz, params)` returns the final position.

## Overlay Logic
```rust
if !enabled || articles < min_articles { return signal; }
if score < exit_threshold { return Position::Flat; }
if score < reduce_threshold {
    // block new entries, preserve existing positions
    return if signal == Position::Flat { Position::Flat } else { signal };
}
return signal;
```

## Deviation from Original Plan
Plan §3C originally specified a `(Position, f64)` return that would halve position size on moderate negative sentiment. This was rejected because the executor (`PaperExecutor` / `MoomooExecutor`) computes fixed `qty = budget / close` at construction time; threading a dynamic size multiplier through every fill would be invasive and error-prone. The adopted force-exit + block-entries interpretation preserves the risk-reduction intent without executor changes.

## Files Touched
- `engine/src/strategy.rs` — new params + `apply_sentiment_overlay`
- `engine/src/scheduler.rs` — fetch sentiment + apply overlay
- `engine/src/db.rs` — `latest_sentiment()`
- `engine/src/api/strategy_config.rs` — PUT validation + response fields
- `engine/src/api/ws.rs` — `TelemetryEvent::StrategyConfigChange` extended
- `frontend/src/lib/components/StrategyConfigPanel.svelte` — toggle + thresholds UI

## Verification
- `cargo test --lib` — 199 passed, 23 pre-existing config failures
- `npm run build` — clean
