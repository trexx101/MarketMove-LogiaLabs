//! Prompt compilation and response parsing for the advisor.
//!
//! The prompt is a prose-digest task: the LLM writes a structured market
//! briefing with six required sections. We parse the response from a
//! fenced JSON block with required section fields.

use chrono::{DateTime, Utc};

use super::{AdvisorBriefing, AdvisorContext, BriefingError, BriefingSections};

/// The system prompt — sent once at the start of the conversation.
/// Instructs the LLM on tone, structure, and output format.
pub fn compile_system_prompt() -> &'static str {
    "You are a senior quantitative analyst writing a morning briefing for a \
    systematic equities trader. The strategy is a SMA-confirmed daily model \
    on QQQ (Nasdaq-100 ETF).\n\
    \n\
    Your briefing must have these sections, in this order, each 1-5 sentences:\n\
    1. REGIME — Current market state with supporting evidence.\n\
    2. PREDICTIONS — What the model expects for 1D/5D/21D. Flag contradictions.\n\
    3. FEATURES — Which named features support the prediction, which contradict. \
    Call out extremes.\n\
    4. SENTIMENT — News sentiment score and whether it agrees with the model.\n\
    5. MACRO — Yields, DXY, VIX. Note any upcoming earnings/FOMC/CPI in the next 7 days.\n\
    6. POSITION ADVICE — Justify the current position or recommend an action. \
    Include concrete warnings.\n\
    \n\
    REGIME definitions:\n\
    - Bullish: Close > SMA200 and SMA200 slope > 0\n\
    - Bearish: Close < SMA200 or SMA200 slope < 0\n\
    - Volatile: VIX regime >= 2 (elevated) or ADX > 30\n\
    - Crash-risk: VIX regime >= 3 (panic) or drawdown > 10%\n\
    \n\
    TONE: Direct, professional, like a sell-side morning note. No hedging \
    filler (\"it's worth noting that...\"). State positions clearly. \
    Every claim must cite a number from the context.\n\
    \n\
    OUTPUT: Respond with a single JSON object wrapped in ```json ... ``` fences. \
    The JSON must have these fields:\n\
    {\n\
      \"regime\": \"string (1-3 sentences)\",\n\
      \"predictions\": \"string (1-3 sentences)\",\n\
      \"features\": \"string (2-5 sentences)\",\n\
      \"sentiment\": \"string (1-2 sentences)\",\n\
      \"macro_section\": \"string (1-3 sentences)\",\n\
      \"position_advice\": \"string (2-5 sentences)\",\n\
      \"warnings\": [\"string (max 3)\"],\n\
      \"suggested_action\": \"string (optional, one of: hold_long, exit_long, enter_long, \
      hold_short, exit_short, enter_short, wait)\",\n\
      \"suggested_confidence\": \"number 0.0-1.0 (optional)\",\n\
      \"suggested_params\": { \"param_name\": value } (optional, only from the documented set)\n\
    }\n\
    \n\
    AVAILABLE STRATEGY PARAMETERS (suggest only these — others will be silently dropped):\n\
    - entry_threshold: f64, default 0.003. Trigger magnitude for new long entries.\n\
    - exit_threshold: f64, default -0.001. Trigger for closing longs.\n\
    - short_entry_threshold: f64, default -0.004. Trigger magnitude for new shorts.\n\
    - short_exit_threshold: f64, default 0.001. Trigger for closing shorts.\n\
    - sma_window: int, default 40. SMA lookback for trend regime.\n\
    - enable_shorting: bool, default false. Allow short-side entries.\n\
    - pred_5d_filter: bool, default false. Require pred_5d agreement before entry.\n\
    \n\
    WARNINGS RULES:\n\
    - Always include a warning if sentiment_source is \"stub\" or \"unavailable\".\n\
    - Always include a warning if features are stale (>2h during market hours).\n\
    - Always include a warning if earnings or FOMC/CPI/NFP are within 5 trading days.\n\
    - Max 3 warnings. If there are more than 3, pick the most material ones.\n\
    \n\
    Do NOT recommend specific entry/exit prices. Do NOT hallucinate dates or \
    values not in the context. Quote numeric values exactly as given."
}

/// Compile the user prompt — serializes the context into the message body.
pub fn compile_user_prompt(ctx: &AdvisorContext) -> String {
    let mut prompt = String::with_capacity(2048);

    prompt.push_str(&format!(
        "Today is {}. Market session: {}. {}",
        ctx.as_of.format("%Y-%m-%d %H:%M UTC"),
        ctx.market_session,
        if ctx.is_trading_day {
            ""
        } else {
            "The market is closed today."
        },
    ));
    if let Some(h) = &ctx.holiday_name {
        prompt.push_str(&format!(" Holiday: {h}."));
    }
    prompt.push_str("\n\n");

    // ── Predictions ──
    prompt.push_str("MODEL PREDICTIONS:\n");
    match (ctx.pred_1d, ctx.pred_5d, ctx.pred_21d) {
        (Some(p1), Some(p5), Some(p21)) => {
            prompt.push_str(&format!(
                "- 1D: {:.4} ({:.2}%)\n\
                 - 5D: {:.4} ({:.2}%)\n\
                 - 21D: {:.4} ({:.2}%)\n\
                 (positive = bullish, negative = bearish)\n\n",
                p1, p1 * 100.0, p5, p5 * 100.0, p21, p21 * 100.0
            ));
        }
        _ => prompt.push_str("- No predictions available.\n\n"),
    }

    // ── Features ──
    let f = &ctx.features;
    prompt.push_str("LIVE FEATURES (most recent bar):\n");
    prompt.push_str(&format!("- trend_slope: {:.4} (annualized, >0 = uptrend)\n", f.trend_slope));
    prompt.push_str(&format!("- trend_adx: {:.1} (0-100, >25 = trending, >30 = strong)\n", f.trend_adx));
    prompt.push_str(&format!("- rsi_14: {:.1} (0-100, >70 overbought, <30 oversold)\n", f.rsi_14));
    prompt.push_str(&format!("- vix_regime: {:.0} (0=calm, 1=normal, 2=elevated, 3=panic)\n", f.vix_regime));
    prompt.push_str(&format!("- tlt_corr_20d: {:.2} (Pearson, -1 to +1)\n", f.tlt_corr_20d));
    prompt.push_str(&format!("- rvol_20d: {:.2}x (relative to 20-day avg)\n", f.rvol_20d));
    prompt.push_str(&format!("- gap_pct: {:.4} ({:.2}%)\n", f.gap_pct, f.gap_pct * 100.0));
    prompt.push_str(&format!("- drawdown_from_50d_high: {:.4} ({:.2}%)\n\n", f.drawdown_from_50d_high, f.drawdown_from_50d_high * 100.0));

    // ── Sentiment ──
    prompt.push_str("SENTIMENT:\n");
    prompt.push_str(&format!("- source: {}\n", ctx.sentiment_source));
    match ctx.sentiment_score {
        Some(s) => prompt.push_str(&format!("- score: {:.3} (range -1 to +1)\n", s)),
        None => prompt.push_str("- score: unavailable\n"),
    }
    match ctx.sentiment_buzz {
        Some(b) => prompt.push_str(&format!("- buzz: {} articles in last week\n\n", b)),
        None => prompt.push_str("\n"),
    }

    // ── Macro ──
    let m = &ctx.macro_ctx;
    prompt.push_str("MACRO CONTEXT:\n");
    match (m.ust_10y_latest, m.ust_10y_prev) {
        (Some(l), Some(p)) => prompt.push_str(&format!("- $UST10Y: {:.2}% (prev: {:.2}%)\n", l, p)),
        (Some(l), _) => prompt.push_str(&format!("- $UST10Y: {:.2}%\n", l)),
        _ => prompt.push_str("- $UST10Y: unavailable\n"),
    }
    match (m.dxy_latest, m.dxy_prev) {
        (Some(l), Some(p)) => prompt.push_str(&format!("- $DXY: {:.1} (prev: {:.1})\n", l, p)),
        (Some(l), _) => prompt.push_str(&format!("- $DXY: {:.1}\n", l)),
        _ => prompt.push_str("- $DXY: unavailable\n"),
    }
    match m.vix_latest {
        Some(v) => prompt.push_str(&format!("- $VIX: {:.2}\n\n", v)),
        None => prompt.push_str("- $VIX: unavailable\n\n"),
    }

    // ── Position ──
    prompt.push_str("CURRENT POSITION:\n");
    prompt.push_str(&format!("- side: {}\n", ctx.position_side));
    if let Some(ep) = ctx.entry_price {
        prompt.push_str(&format!("- entry price: ${:.2}\n", ep));
    }
    if let Some(pnl) = ctx.unrealized_pnl {
        prompt.push_str(&format!("- unrealized PnL: {:.2}%\n", pnl * 100.0));
    }
    prompt.push_str("\n");

    // ── Recent trades ──
    if !ctx.recent_trades.is_empty() {
        prompt.push_str("RECENT CLOSED TRADES (last 5):\n");
        for t in &ctx.recent_trades {
            prompt.push_str(&format!(
                "- {}: entry ${:.2}, exit ${:.2}, PnL: ${:.2}\n",
                t.side, t.entry_price, t.exit_price, t.pnl
            ));
        }
        prompt.push_str("\n");
    }

    prompt
}

/// Parse the LLM response into an `AdvisorBriefing`.
///
/// Extracts the first ```json fenced block, validates required fields,
/// and returns a structured briefing. On failure, returns `BriefingError`
/// — NEVER silently substitutes a fake "hold" action.
pub fn parse_response(
    raw: &str,
    model_used: &str,
    as_of: DateTime<Utc>,
    for_date: String,
) -> Result<AdvisorBriefing, BriefingError> {
    // 1. Extract JSON from fenced block.
    let json_str = extract_json_block(raw)
        .or_else(|| {
            // Try the whole response as JSON.
            if raw.trim().starts_with('{') {
                Some(raw.trim().to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| BriefingError::ParseFailed {
            raw: raw.to_string(),
            reason: "no JSON block found in response".to_string(),
        })?;

    // 2. Parse into a temp struct.
    let parsed: TempResponse = serde_json::from_str(&json_str).map_err(|e| {
        BriefingError::ParseFailed {
            raw: json_str.clone(),
            reason: format!("JSON parse error: {e}"),
        }
    })?;

    // 3. Validate required sections are non-empty.
    let sections = BriefingSections {
        regime: require_nonempty(parsed.regime, "regime")?,
        predictions: require_nonempty(parsed.predictions, "predictions")?,
        features: require_nonempty(parsed.features, "features")?,
        sentiment: require_nonempty(parsed.sentiment, "sentiment")?,
        macro_section: require_nonempty(parsed.macro_section, "macro_section")?,
        position_advice: require_nonempty(parsed.position_advice, "position_advice")?,
    };

    // 4. Validate for_date is a valid date string.
    chrono::NaiveDate::parse_from_str(&for_date, "%Y-%m-%d").map_err(|_| {
        BriefingError::ParseFailed {
            raw: json_str.clone(),
            reason: format!("invalid for_date: {for_date}"),
        }
    })?;

    // 5. Strip unknown suggested params.
    let valid_params = [
        "entry_threshold", "exit_threshold", "short_entry_threshold",
        "short_exit_threshold", "sma_window", "enable_shorting", "pred_5d_filter",
    ];
    let clean_params = parsed.suggested_params.as_ref().map(|params| {
        let mut clean = serde_json::Map::new();
        if let Some(obj) = params.as_object() {
            for k in valid_params {
                if let Some(v) = obj.get(k) {
                    clean.insert(k.to_string(), v.clone());
                }
            }
        }
        serde_json::Value::Object(clean)
    });

    let digest = format!(
        "REGIME\n{}\n\nPREDICTIONS\n{}\n\nFEATURES\n{}\n\nSENTIMENT\n{}\n\nMACRO\n{}\n\nPOSITION ADVICE\n{}",
        sections.regime, sections.predictions, sections.features,
        sections.sentiment, sections.macro_section, sections.position_advice
    );

    Ok(AdvisorBriefing {
        model_used: model_used.to_string(),
        as_of,
        for_date,
        digest,
        sections,
        warnings: parsed.warnings.unwrap_or_default(),
        suggested_action: parsed.suggested_action,
        suggested_confidence: parsed.suggested_confidence,
        suggested_params: clean_params,
        parse_status: "ok".to_string(),
        parse_error: None,
    })
}

/// Extract the first ```json ... ``` fenced block from the response.
fn extract_json_block(raw: &str) -> Option<String> {
    let start = raw.find("```json")?;
    let start = start + "```json".len();
    let end = raw[start..].find("```")?;
    Some(raw[start..start + end].trim().to_string())
}

/// Require a field to be non-empty, returning a ParseFailed error otherwise.
fn require_nonempty(value: Option<String>, field: &str) -> Result<String, BriefingError> {
    match value {
        Some(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        _ => Err(BriefingError::ParseFailed {
            raw: String::new(),
            reason: format!("required section '{field}' is missing or empty"),
        }),
    }
}

// ── temp struct for deserialization ───────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct TempResponse {
    regime: Option<String>,
    predictions: Option<String>,
    features: Option<String>,
    sentiment: Option<String>,
    #[serde(rename = "macro_section")]
    macro_section: Option<String>,
    #[serde(rename = "position_advice")]
    position_advice: Option<String>,
    #[serde(default)]
    warnings: Option<Vec<String>>,
    #[serde(rename = "suggested_action")]
    suggested_action: Option<String>,
    #[serde(rename = "suggested_confidence")]
    suggested_confidence: Option<f64>,
    #[serde(rename = "suggested_params")]
    suggested_params: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_response() {
        let raw = r#"```json
{
  "regime": "Bullish: SMA200 slope positive",
  "predictions": "Model expects +0.30% 1D",
  "features": "trend_adx 31 supports, gap_pct +1.2% contradicts",
  "sentiment": "Finnhub +0.42 bullish",
  "macro_section": "$UST10Y 4.32%, $VIX 18.4",
  "position_advice": "Hold current long",
  "warnings": ["FOMC in 2 days"],
  "suggested_action": "hold_long",
  "suggested_confidence": 0.8
}
```"#;
        let result = parse_response(raw, "test-model", Utc::now(), "2026-08-04".to_string());
        assert!(result.is_ok(), "parse failed: {:?}", result.err());
        let briefing = result.unwrap();
        assert_eq!(briefing.parse_status, "ok");
        assert_eq!(briefing.warnings.len(), 1);
        assert_eq!(briefing.suggested_action, Some("hold_long".to_string()));
        assert!((briefing.suggested_confidence.unwrap() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn parse_missing_section_fails() {
        let raw = r#"```json
{
  "regime": "Bullish",
  "predictions": "ok",
  "features": "ok",
  "sentiment": "ok",
  "macro_section": "ok"
}
```"#;
        let result = parse_response(raw, "test", Utc::now(), "2026-08-04".to_string());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("position_advice"), "expected missing section error, got: {err}");
    }

    #[test]
    fn parse_no_json_block_fails() {
        let result = parse_response("just some text", "test", Utc::now(), "2026-08-04".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn extract_json_block_basic() {
        let raw = "some text ```json\n{\"key\":\"val\"}\n``` more text";
        let extracted = extract_json_block(raw);
        assert_eq!(extracted, Some("{\"key\":\"val\"}".to_string()));
    }

    #[test]
    fn extract_json_block_missing() {
        assert_eq!(extract_json_block("no json here"), None);
    }

    #[test]
    fn system_prompt_contains_required_sections() {
        let p = compile_system_prompt();
        assert!(p.contains("REGIME"));
        assert!(p.contains("PREDICTIONS"));
        assert!(p.contains("FEATURES"));
        assert!(p.contains("SENTIMENT"));
        assert!(p.contains("MACRO"));
        assert!(p.contains("POSITION ADVICE"));
        assert!(p.contains("WARNINGS RULES"));
    }

    #[test]
    fn user_prompt_includes_feature_names() {
        let ctx = AdvisorContext {
            as_of: Utc::now(),
            symbol: "QQQ".to_string(),
            market_session: "Regular".to_string(),
            next_open_utc: None,
            next_close_utc: None,
            is_trading_day: true,
            holiday_name: None,
            pred_1d: Some(0.003),
            pred_5d: Some(0.012),
            pred_21d: Some(0.031),
            pred_ts: Some(1000000),
            features: super::super::FeatureSnapshot::default(),
            sentiment_score: Some(0.42),
            sentiment_buzz: Some(47),
            sentiment_source: "finnhub".to_string(),
            macro_ctx: super::super::MacroSnapshot {
                ust_10y_latest: Some(4.32),
                ust_10y_prev: Some(4.28),
                dxy_latest: Some(104.1),
                dxy_prev: Some(104.4),
                vix_latest: Some(18.4),
                earnings_in_next_7d: vec![],
                macro_releases_in_next_7d: vec![],
            },
            position_side: "long".to_string(),
            entry_price: Some(687.99),
            entry_ts: Some(1000000),
            unrealized_pnl: Some(0.005),
            realized_pnl_session: None,
            recent_trades: vec![],
        };
        let prompt = compile_user_prompt(&ctx);
        assert!(prompt.contains("trend_slope"));
        assert!(prompt.contains("trend_adx"));
        assert!(prompt.contains("rsi_14"));
        assert!(prompt.contains("vix_regime"));
        assert!(prompt.contains("tlt_corr_20d"));
        assert!(prompt.contains("rvol_20d"));
        assert!(prompt.contains("gap_pct"));
        assert!(prompt.contains("drawdown_from_50d_high"));
        assert!(prompt.contains("$UST10Y"));
        assert!(prompt.contains("$DXY"));
        assert!(prompt.contains("$VIX"));
        assert!(prompt.contains("finnhub"));
    }
}