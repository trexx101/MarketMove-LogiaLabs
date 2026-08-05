//! Conversational chat — on-demand, multi-turn, SSE-streamed.
//!
//! Wraps the same `AdvisorState` and `call_openrouter` as the briefing loop,
//! but adds session history (last N turns) to the prompt. Rate-limited per-IP.

use std::sync::Arc;
use std::time::Duration;

use tracing::warn;

use super::{AdvisorConfig, AdvisorState, BriefingError};
use super::briefing::call_openrouter;
use super::context::build_context;
use super::prompt::compile_user_prompt;

/// Generate a chat response given a question and the current context.
/// Returns the full LLM response text.
pub async fn generate_chat_response(
    state: &AdvisorState,
    pool: &crate::db::DbPool,
    symbol: &str,
    question: &str,
    history: &[(String, String)], // (question, response) pairs, oldest first
) -> Result<String, BriefingError> {
    let context = build_context(pool, symbol)
        .await
        .map_err(|e| BriefingError::Disabled(format!("context build: {e}")))?;

    let system = compile_chat_system_prompt();
    let user = compile_chat_user_prompt(&context, question, history);

    let raw = call_openrouter(
        &state.cfg.api_key,
        &state.cfg.model,
        &state.cfg.api_base,
        &system,
        &user,
        state.cfg.request_timeout_seconds,
    )
    .await?;

    Ok(raw)
}

/// System prompt for the conversational chat mode.
/// Simpler than the briefing prompt — no required sections, just context + answer.
fn compile_chat_system_prompt() -> String {
    "You are a quantitative trading advisor answering follow-up questions about \
    the daily briefing. You have access to the same live market data, predictions, \
    features, sentiment, and macro context as the morning briefing.\n\
    \n\
    Answer the user's question directly using only the data provided. If the \
    question requires information not in the context, say so explicitly. \
    Do not hallucinate dates, prices, or values.\n\
    \n\
    Keep answers concise — 2-5 sentences unless the question demands more detail. \
    If the user asks about a specific feature, quote its value and explain its \
    interpretation.\n\
    \n\
    Previous conversation history is provided for context. Refer to prior \
    answers if the user asks a follow-up."
    .to_string()
}

/// Compile the user prompt for chat, including conversation history.
fn compile_chat_user_prompt(
    ctx: &super::AdvisorContext,
    question: &str,
    history: &[(String, String)],
) -> String {
    let mut prompt = compile_user_prompt(ctx);

    // Prepend conversation history.
    if !history.is_empty() {
        prompt.push_str("\n\nPREVIOUS CONVERSATION:\n");
        for (i, (q, a)) in history.iter().enumerate() {
            prompt.push_str(&format!("Q{}: {}\nA{}: {}\n\n", i + 1, q, i + 1, a));
        }
    }

    prompt.push_str(&format!("\nNEW QUESTION: {}\n\nAnswer:", question));
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_system_prompt_not_empty() {
        let p = compile_chat_system_prompt();
        assert!(!p.is_empty());
        assert!(p.contains("quantitative trading advisor"));
    }

    #[test]
    fn chat_user_prompt_includes_history() {
        let ctx = super::super::AdvisorContext {
            as_of: chrono::Utc::now(),
            symbol: "QQQ".into(),
            market_session: "Regular".into(),
            next_open_utc: None,
            next_close_utc: None,
            is_trading_day: true,
            holiday_name: None,
            pred_1d: None,
            pred_5d: None,
            pred_21d: None,
            pred_ts: None,
            features: super::super::FeatureSnapshot::default(),
            sentiment_score: None,
            sentiment_buzz: None,
            sentiment_source: "stub".into(),
            macro_ctx: super::super::MacroSnapshot {
                ust_10y_latest: None,
                ust_10y_prev: None,
                dxy_latest: None,
                dxy_prev: None,
                vix_latest: None,
                earnings_in_next_7d: vec![],
                macro_releases_in_next_7d: vec![],
            },
            position_side: "flat".into(),
            entry_price: None,
            entry_ts: None,
            unrealized_pnl: None,
            realized_pnl_session: None,
            recent_trades: vec![],
        };
        let history = vec![
            ("What is the regime?".into(), "Bullish".into()),
        ];
        let prompt = compile_chat_user_prompt(&ctx, "Why?", &history);
        assert!(prompt.contains("PREVIOUS CONVERSATION"));
        assert!(prompt.contains("Q1: What is the regime?"));
        assert!(prompt.contains("A1: Bullish"));
        assert!(prompt.contains("NEW QUESTION: Why?"));
    }
}