# MarketMarkovNet — Wave 5: Edge-First Overhaul (research plan)

STATUS: RESEARCH SPEC, NOT YET IMPLEMENTED. Source: DeepSeek-R1-0528 (via OmniRoute),
critiqued and corrected by Hermes agent. Replaces the zero-edge OHLCV-return model.

## Why this exists (the proven failure)
V2 model trained on 3 OHLCV features regressing next-period log-return. Walk-forward
results: Pearson IC 1H=-0.0145, 4H=-0.0075, 24H=-0.0302 (NEGATIVE), hit-rate ~49-50%.
Regime backtest +2.43% vs B&H -21.41% was FLAT-IN-BEAR-MARKET luck (win-rate 45.56%).
Lesson: OHLCV-return targets carry ~no signal. The fix is a different ALPHA SOURCE,
not a better net on the same features.

## Decision (locked with user)
- RETIRE KRAKEN. Full solution runs on Binance (spot + futures). Train==serve on
  Binance data. Funding/perp basis now available. No exchange mismatch.
- First universe: BTC-only. Prove edge, then widen to majors.
- Horizon: 30min–1d, long/short, structural-shift catching, NOT HFT.
- OpenRouter cheap models = LOW-FREQUENCY FEATURES ONLY (LLM regime hourly + vision
  chart hourly), cached, timeout+fallback, never in per-bar path.
- Reuse Rust engine / ZMQ / dashboard / paper-mode / norm_stats contract. Only the
  inference model + feature pipeline change.
- Risk mildly configurable: vol-targeted sizing, configurable max leverage + max-DD halt.
- Equities extensibility via a pluggable FeatureSource abstraction (shared core model).

## R1 plan (summary — verbatim structure, see /tmp/hermes-r1-plan.md for full text)
1. Data: Binance klines/fundingRate/premiumIndex/depth20+aggTrade ws; on-chain
   Glassnode/CryptoQuant; OpenRouter Mixtral (sentiment) + CLIP (chart embed) hourly.
2. Features: core = {vol_regime (GARCH), vol_break (changepoint), funding_rate,
   basis_72h_z, llm_bull_prob}; extended = {ob_imbalance, exchange_net_flow,
   chart_embed PCA}. Trait `FeatureSource` with equities stub.
3. Model: TCN (4 dilated causal conv, kernel=3, dilation 2^n), 72x5-10 input,
   3 horizon heads with monotonic constraint. Penetration labels (±k% within N bars).
   Focal loss (γ=2), temporal dropout 0.2, weight decay 1e-4.
4. Validation: 12 quarterly walk-forward folds (2019-01..2024-06), 72h embargo,
   purge FTX etc. Gate: OOS IC>0.04, OOS Sharpe>0.8, MaxDD<25%, turnover<5x/day.
5. Deploy: features.rs struct swap, ZMQ adds schema_version, vol-targeted sizing,
   DD halt. norm_stats schema_version=2.
6. Go/no-go on BTC gate, then expand futures->majors->alts.
7. Self-critique: dynamic k, VIF/PCA collinearity guard, latency staleness feature.

## CRITIQUE + CORRECTIONS (Hermes, applied)
The R1 plan is strong on structure; these are the gaps that would otherwise repeat
the V2 "no edge" outcome if taken literally:

- C1 (CRITICAL): Funding + basis + order-flow all derive from the SAME order book
  microstructure. During calm regimes they're near-independent (good); during stress
  they collapse to one factor (R1's own critique #2). Mitigation: compute a PCA /
  VIF on the feature block at train time and report the effective rank; the FIRST
  principal component of {funding, basis, ob_imbalance} is the real signal — do not
  treat the three as 3 independent edges. Keep vol_regime + llm as separate factors.
- C2: Penetration-label k must be VOLATILITY-SCALED, not fixed per horizon. Use
  k = c * rolling_ATR (R1 suggested 0.75×24h ATR; adopt). Fixed k overfits regime.
- C3: Shuffle/look-ahead in label construction. Penetration "within N bars" must be
  computed FORWARD-only from bar t using future closes; embargo must cover N bars
  PLUS the label horizon, not just 72h. Set embargo = max(72h, label_horizon + N).
- C4: OpenRouter model IDs in the plan (mixtral-8x7b, clip-vit-base-patch32) are
  illustrative; confirm current OpenRouter IDs at build time. Vision on a chart is the
  weakest, highest-cost signal — treat as OPTIONAL extended feature, gated by its own
  incremental IC; do not ship it in v1.
- C5: Storage (QuestDB/Redis) is over-engineered for BTC-only single-instrument.
  Start with the existing SQLite + in-memory feature cache; add a TSDB only if the
  equities expansion needs it. KISS.
- C6: Sizing formula divides by atr_24h*sqrt(24) — confirm units (vol_target is a
  fraction of equity; atr in price terms; need equity normalization). Specify explicitly.
- C7: Backtest fee assumption. R1's snippet uses taker 0.1% + slippage 0.05%; for
  futures maker ~0.02-0.04% is realistic. Use maker fee in the cost model (Wave 2
  already moved to 0.04%). Slippage model should scale with ob_imbalance.
- C8: "Purge FTX etc." is manual. Prefer a programmatic purge: drop windows around
  known event timestamps from BOTH train and test, OR use a structural-break-aware
  split. Manual purging is fragile.
- C9: The deploy gate OOS Sharpe>0.8 is optimistic for a single-instrument crypto
  model; keep IC>0.04 as the PRIMARY gate (Sharpe is noisy on 12 folds). Don't block
  deploy on Sharpe alone if IC clears and OOS equity is positive.
- C10: No mention of the EXISTING engine's candle staleness / WS-silence workaround
  (Kraken had this; Binance ws is more reliable but keep the REST backfill fallback).
  Preserve the existing REST-backfill-on-<200-candles gate when porting to Binance.

## Acceptance / go-no-go (from R1 + corrections)
- PRIMARY GATE: walk-forward mean OOS IC > 0.03-0.05 on a horizon AND OOS equity > 0.
- Secondary: positive OOS Sharpe, MaxDD<25%, turnover reasonable.
- If IC stays ~0 after adding funding/basis/vol features → STOP and reconsider alpha
  source (the signal genuinely may not exist at this horizon); do NOT ship weights.
- Equities extension is a LATER wave; core model + FeatureSource must stay asset-agnostic.

## Build order (proposed)
1. Binance data client: klines + fundingRate + premiumIndex (REST) + depth/aggTrade (ws).
   Reuse existing candle loop; add funding/basis polling. (Kraken code removed.)
2. Feature pipeline: vol_regime (GARCH), vol_break (changepoint), funding_rate,
   basis_z; pluggable FeatureSource trait; equities stub. norm_stats v2 (versioned).
3. Label generator: volatility-scaled penetration labels, forward-only, embargo =
   max(72h, horizon+N).
4. Model: TCN, focal loss, dropout+decay, horizon monotonicity. Local training script.
5. Walk-forward harness with the gate + stop conditions. NO DEPLOY until gate passes.
6. Only after gate: features.rs swap + ZMQ schema_version + vol-targeted sizing + DD halt.
7. Paper-mode on Binance; monitor dashboard IC/Sharpe; widen to majors on success.

## References
- V2 results: models/Crypto_Markov_Head_V2.ipynb (eval IC negative; backtest luck).
- Current engine: engine/src/features.rs, normalize.rs, strategy.rs, scheduler.rs,
  api.rs, main.rs (Wave 0–4 already deployed).
- R1 raw plan: /tmp/hermes-r1-plan.md (regenerate via hermes-run-r1-plan.py if needed).
