//! D4 (DEFERRED to cheap/fast LLM): OpenRouter hourly-cached regime feature.
//!
//! The real adapter wakes hourly, fetches a chart image / recent news, calls an
//! OpenRouter model, parses a `llm_bull_prob` in [0,1], and writes it to an
//! atomic cache the feature pipeline reads. It must have a timeout + stale-cache
//! fallback and is NEVER in the per-bar latency path.
//!
//! For the scaffold, `read_cached_bull_prob` returns the last cached value (or
//! `0.5` neutral if none), and the background task is filled in by the D4 pass.

use std::sync::RwLock;

/// Process-wide cache for the latest LLM regime probability.
/// `0.5` = neutral/absent (no signal yet).
static LLM_BULL_PROB: RwLock<f64> = RwLock::new(0.5);

/// Read the most recently cached LLM bull probability in [0,1].
pub fn read_cached_bull_prob() -> f64 {
    match LLM_BULL_PROB.read() {
        Ok(g) => *g,
        Err(_) => 0.5,
    }
}

/// Update the cached LLM bull probability.
pub fn write_cached_bull_prob(p: f64) {
    if let Ok(mut g) = LLM_BULL_PROB.write() {
        *g = p.clamp(0.0, 1.0);
    }
}

/// TODO(D4): spawn an hourly task that calls OpenRouter, parses the regime
/// probability, and calls `write_cached_bull_prob`. Falls back to the last
/// cached value (or `0.5`) on timeout/error.
pub async fn spawn_regime_cache_task(_cache_ttl_seconds: u64) {
    // Intentionally a no-op stub until the D4 reasoning pass implements it.
}
