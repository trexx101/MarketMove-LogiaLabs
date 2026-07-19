# mmn-training-deployment-hardening - Work Plan

## TL;DR (For humans)
<!-- Fill this LAST, after the detailed plan below is written, so it summarizes the REAL plan. -->
<!-- Plain English for a non-engineer: NO file paths, NO todo numbers, NO wave/agent/tool names. -->

**What you'll get:** A trading model where the live predictions you see in the dashboard actually match the model's training assumptions — so predictions are no longer silently muted or biased — followed by a properly retrained model that was trained on the same data it trades (not a different exchange's bull market).

**Why this approach:** The current model produces worse live predictions than the Colab backtests because of train/serve mismatches: the live engine computes the VWAP-deviation feature linearly while the model was trained on a log version, normalization differs (rolling-z in training vs global-z in serving), and VWAP comes from a different source than training. These are silent bugs — the engine runs fine but feeds the model shifted inputs. Wave 0 fixes them without retraining. Waves 1-3 then retrain on aligned, multi-regime data with walk-forward validation and realistic costs.

**What it will NOT do:** It will not change the trading engine's structure, the ZMQ protocol, the candle interval, the parity gate, or the live-trading guard. Retraining stays paper-mode validated before any live switch.

**Effort:** Low-Medium (Wave 0) → Medium-Large (Waves 1-3)
**Risk:** Low (Wave 0 — additive bug fixes). Medium (retrain waves — data/label changes need validation).
**Decisions to sanity-check:** (1) Normalization scheme: adopt GLOBAL z-score on RAW features at both train and serve (removing the rolling-z from the Colab training path so train==serve). (2) VWAP source: compute rolling VWAP in the Rust engine to match Colab's Σ(typical_price×volume)/Σ(volume); revisit during retrain if Kraken-reported vwap is preferred. (3) magnitude_threshold is already correctly 0.005 in deploy config (raw-scale) — verified, NOT a bug.

Your next move: approve this plan, then run `$start-work` to begin implementation. Full execution detail follows below.

---

> TL;DR (machine): Low-Medium effort, Low risk (Wave 0). Fixes train/serve feature skews (vwap_dev log vs linear, rolling-z vs global-z, VWAP source) so live predictions match training; then retrains on aligned multi-regime data with walk-forward CV + realistic costs.

## Scope
### Must have
- Fix `vwap_dev` in Rust feature pipeline to `log(close / vwap)` to match Colab (Wave 0.1)
- Unify normalization: GLOBAL z-score on RAW features, identical at train + serve; remove rolling-z from Colab training path so train==serve (Wave 0.2)
- Align VWAP source: compute rolling VWAP in Rust (Σ(typical_price×vol)/Σ(vol), window=72) to match Colab; do not rely on Kraken-reported vwap (Wave 0.3)
- Automate norm_stats export to JSON, versioned with the checkpoint (model.pt + norm_stats.json + meta.json) (Wave 0.4)
- Add parity test asserting Colab vs Rust feature vectors are bit-close (Wave 0.6)
- Pull Kraken BTC/USD 1h history ≥ 2-3 years, multi-regime, to resolve exchange mismatch (Wave 1.1)
- Walk-forward / purged k-fold CV instead of single split (Wave 2.1)
- Realistic costs + simple position sizing in backtester (Wave 2.3)
- Report Sharpe, max DD, turnover, OOS equity — not just hit-rate (Wave 2.4)
- Vol-adaptive or multi-timeframe regime + uncertainty/confidence gate in deployment (Wave 3.1, 3.2)
- Persist model metadata + thresholds; canary/rollback on retrain (Wave 3.3)

### Must NOT have (guardrails, anti-slop, scope boundaries)
- No change to ZMQ protocol or inference service internals
- No change to candle interval (stays 60-minute hourly)
- No live Kraken trading changes or parity gate removal
- No architectural rewrite of MarketMarkovNet (keep causal CNN + low-rank Markov heads)
- No retrain that is not first validated in paper mode

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after.
- Evidence: `.omo/evidence/` directory; parity test in `engine/` test suite; backtest OOS equity printed in notebook.
- Wave 0 gate: parity test green + `cargo test` clean + `/api/predictions` returns stable non-muted predictions after redeploy.

## Execution strategy
### Parallel execution waves
> Target 5-8 todos per wave. Fewer than 3 (except the final) means you under-split.

**Wave 0** (6 todos, mostly serial — each fixes a shared feature pipeline): parity fixes, no retrain
**Wave 1** (4 todos, parallel): data + label alignment + retrain prep
**Wave 2** (5 todos, parallel): training robustness
**Wave 3** (4 todos, parallel): deployment strategy upgrade

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| 0.1 vwap_dev log fix | — | 0.6 | 0.3 |
| 0.2 unify normalization | — | 0.6 | 0.3 |
| 0.3 rolling VWAP in Rust | — | 0.6 | 0.1 |
| 0.4 automate norm_stats export | — | — | 0.2 |
| 0.5 verify threshold (VERIFIED OK) | — | — | — |
| 0.6 parity test | 0.1, 0.2, 0.3 | — | — |
| 1.1 aligned data pull | — | 1.2 | 1.3 |
| 1.2 feature/label additions | 1.1 | 2.2 | 1.3 |
| 1.3 label review | — | 2.2 | 1.2 |
| 1.4 re-export stats+meta | 1.1, 1.2 | 2.2 | — |
| 2.1 walk-forward CV | 1.4 | 2.4 | 2.2, 2.3 |
| 2.2 weight decay + dropout + early stop | 1.4 | 2.4 | 2.1, 2.3 |
| 2.3 realistic costs + sizing | 1.4 | 2.4 | 2.1, 2.2 |
| 2.4 report Sharpe/DD/turnover | 2.1, 2.2, 2.3 | 3.3 | — |
| 3.1 vol-adaptive regime | 2.4 | 3.3 | 3.2 |
| 3.2 uncertainty gate | 2.4 | 3.3 | 3.1 |
| 3.3 metadata + canary/rollback | 2.4 | — | — |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

### Wave 0: Inference parity (no retrain)

- [ ] 0.1 Fix `vwap_dev` to `log(close / vwap)` in Rust feature pipeline
  What to do / Must NOT do: In `engine/src/features.rs`, change the `vwap_dev` computation in `compute_features` from `(close - vwap) / vwap` to `(close / vwap).ln()` (guard vwap > 0.0; else 0.0). This matches the Colab training feature `df['vwap_dev'] = np.log(df['close'] / df['vwap'])`. Update the existing unit test `compute_features_single_candle` expected value and add a dedicated test asserting `vwap_dev == ln(close/vwap)`. MUST NOT change log_return or atr_72.
  Parallelization: Wave 0 | Blocked by: — | Blocks: 0.6
  References: `engine/src/features.rs:63-68` (vwap_dev), `models/Crypto_Markov_Head.ipynb:75` (Colab vwap_dev).
  Acceptance criteria: `cargo test --lib features` passes; vwap_dev unit test asserts log form.
  QA scenarios: happy: `cargo test --lib features::tests::compute_features_vwap_dev_is_log` — passes. failure: `cargo test --lib features` — no regressions.
  Commit: Y | fix(features): use log(close/vwap) for vwap_dev to match Colab training

- [ ] 0.2 Unify normalization (global z on raw features, train==serve)
  What to do / Must NOT do: In the Colab notebook, REMOVE the rolling-z normalization (`norm_window = lookback*4; df[f'{col}_norm'] = (df[col]-roll_mean)/(roll_std+1e-8)`) and instead save GLOBAL mean/std of the RAW features (log_return, atr_72, vwap_dev) — which the notebook already does for `norm_stats`. The Rust engine already applies GLOBAL z-score (`normalize_row` in `engine/src/normalize.rs`). The fix makes the served input identical to what training fed the model. Document the decision in the notebook. MUST NOT change the Rust `normalize_row` (it is already correct global-z). MUST NOT change feature definitions (only the normalization path in the notebook).
  Parallelization: Wave 0 | Blocked by: — | Blocks: 0.6
  References: `models/Crypto_Markov_Head.ipynb:77-88` (rolling-z), `engine/src/normalize.rs:31-42` (global z).
  Acceptance criteria: notebook re-run produces features identical (for the global-z transform) to the Rust path; re-saved norm_stats used by engine.
  QA scenarios: happy: parity test (0.6) green after both 0.1+0.2 land. failure: if rolling-z remains, parity test fails.
  Commit: Y | fix(notebook): use global z-score at train to match deployed global-z serve

- [ ] 0.3 Compute rolling VWAP in Rust to match Colab
  What to do / Must NOT do: In `engine/src/features.rs`, add a rolling VWAP computation matching Colab: `vwap = Σ(typical_price×volume)/Σ(volume)` over a 72-window (typical_price = (high+low+close)/3), then `vwap_dev = ln(close / vwap)`. This replaces reliance on the Kraken-reported `vwap` field. Update `FeatureRow`/compute_features to carry the rolling VWAP (or compute inline). MUST NOT use the candle's `vwap` field for the model feature. MUST NOT change ATR.
  Parallelization: Wave 0 | Blocked by: — | Blocks: 0.6
  References: `models/Crypto_Markov_Head.ipynb:73-75` (Colab VWAP), `engine/src/features.rs:63-68`.
  Acceptance criteria: unit test asserts rolling VWAP equals Colab formula on a known window.
  QA scenarios: happy: `cargo test --lib features::tests::compute_features_rolling_vwap_matches_colab` — passes. failure: parity test (0.6) green.
  Commit: Y | fix(features): compute rolling VWAP to match Colab instead of candle vwap

- [ ] 0.4 Automate norm_stats export to JSON, versioned with checkpoint
  What to do / Must NOT do: In the Colab export cell, save `norm_stats.json` (the `{"mean":[...],"std":[...]}` shape the engine loads) alongside `model.pt`, plus a `meta.json` recording train_date, data_range, exchange/symbol, lookback, threshold. Remove the old `.npz` export (or keep as archive). MUST NOT change the JSON schema the engine expects (`mean`/`std` arrays of length 3).
  Parallelization: Wave 0 | Blocked by: 0.2 | Blocks: —
  References: `models/Crypto_Markov_Head.ipynb:359-374` (npz export), `engine/src/normalize.rs:17-24` (JSON shape).
  Acceptance criteria: `norm_stats.json` loads via `NormStats::load`; `meta.json` present.
  QA scenarios: happy: `python -c "import json; json.load(open('norm_stats.json'))"` valid. failure: engine `NormStats::load` succeeds on the new file.
  Commit: Y | feat(notebook): export versioned norm_stats.json + meta.json with checkpoint

- [ ] 0.5 Verify deployed threshold (VERIFIED OK — no change)
  What to do / Must NOT do: Confirm `MAGNITUDE_THRESHOLD=0.005` in deploy config (verified in README, .env.example, docker-compose.yml, parity fixture). The Colab backtester's 0.50 is in ×100 scaled space; the engine feeds raw log-returns (÷100), so 0.005 is correct. No code change. Document as verified in the plan's evidence.
  Parallelization: Wave 0 | Blocked by: — | Blocks: —
  References: `deploy/docker-compose.yml:93`, `.env.example:30`, `README.md:108`.
  Acceptance criteria: threshold == 0.005 in running container (check /api/status or env).
  QA scenarios: happy: `docker exec mmn-engine printenv MAGNITUDE_THRESHOLD` → 0.005.
  Commit: N

- [ ] 0.6 Add parity test (Colab vs Rust feature vectors bit-close)
  What to do / Must NOT do: In `engine/src/features.rs` tests (or a new `engine/tests/parity_features.rs`), add a test that for a known candle window (e.g. the parity golden fixture) computes `compute_features` + `normalize_row` and asserts the resulting `[f64;3]` rows match the values produced by the Colab `SwingTradingDataset` for the same candles within a tight epsilon (1e-6). Use a small fixed window (e.g. 80 candles) with explicit close/high/low/vol. MUST NOT call external/network code. MUST NOT change the feature math (only verify it).
  Parallelization: Wave 0 | Blocked by: 0.1, 0.2, 0.3 | Blocks: —
  References: `engine/src/features.rs`, `tests/fixtures/parity_golden_168h.json`.
  Acceptance criteria: `cargo test --lib features` (or `cargo test --test parity_features`) passes within epsilon.
  QA scenarios: happy: `cargo test features` green. failure: `cargo test features` red on any mismatch → fix 0.1/0.2/0.3.
  Commit: Y | test(features): add Colab-vs-Rust parity test for feature vectors

### Wave 1: Data & label alignment + retrain prep

- [ ] 1.1 Pull aligned Kraken BTC/USD 1h history (multi-regime)
  What to do / Must NOT do: Acquire ≥ 2-3 years of Kraken BTC/USD 1h OHLCV (covers bull/bear/sideways). If Kraken history is limited, align the Binance symbol used for deployment, but do NOT mix exchanges. Replace the Binance 2025-only pull. MUST NOT train on one exchange and deploy on another.
  Parallelization: Wave 1 | Blocked by: — | Blocks: 1.2
  References: `models/Crypto_Markov_Head.ipynb:11-34` (Binance pull).
  Acceptance criteria: dataset covers multiple regimes; documented train range.
  QA scenarios: happy: notebook loads ≥ 2y of same-exchange 1h data.
  Commit: Y | feat(notebook): load multi-regime same-exchange 1h history

- [ ] 1.2 Feature/label additions (optional, documented)
  What to do / Must NOT do: Optionally add liquidity/vol filters and features (realized vol, volume z). Keep parity documented for any addition. MUST NOT break the 3-feature contract without updating model + norm_stats.
  Parallelization: Wave 1 | Blocked by: 1.1 | Blocks: 2.2
  References: `models/Crypto_Markov_Head.ipynb:60-92`.
  Acceptance criteria: features documented; parity re-verified if changed.
  QA scenarios: happy: notebook trains with new features; parity holds or is intentionally updated.
  Commit: Y | feat(notebook): optional vol/liquidity features with parity docs

- [ ] 1.3 Label review (log-return targets + scaling)
  What to do / Must NOT do: Keep log-return targets at 1h/4h/24h; review ×100 scaling + directional hinge on new regime data. Optionally design an uncertainty head (later wave). MUST NOT change target horizon semantics silently.
  Parallelization: Wave 1 | Blocked by: — | Blocks: 2.2
  References: `models/Crypto_Markov_Head.ipynb:106-122`.
  Acceptance criteria: labels validated across regimes; documented.
  QA scenarios: happy: target distributions sane across bull/bear.
  Commit: Y | feat(notebook): review labels across regimes

- [ ] 1.4 Re-export aligned norm_stats + meta
  What to do / Must NOT do: After 1.1/1.2, re-run export (0.4) to produce versioned `norm_stats.json` + `meta.json` for the new data. MUST NOT reuse stale stats.
  Parallelization: Wave 1 | Blocked by: 1.1, 1.2 | Blocks: 2.2
  References: `models/Crypto_Markov_Head.ipynb:347-374`.
  Acceptance criteria: new stats + meta reflect aligned data.
  QA scenarios: happy: norm_stats.json + meta.json regenerated.
  Commit: Y | feat(notebook): re-export aligned norm_stats + meta

### Wave 2: Training robustness

- [ ] 2.1 Walk-forward / purged k-fold CV
  What to do / Must NOT do: Replace single 70/15/15 split with `TimeSeriesSplit` + embargo gap. Report per-fold OOS metrics. MUST NOT use random shuffling on time series.
  Parallelization: Wave 2 | Blocked by: 1.4 | Blocks: 2.4
  References: `models/Crypto_Markov_Head.ipynb:128-141`.
  Acceptance criteria: CV folds report OOS hit-rate/correlation; no look-ahead.
  QA scenarios: happy: notebook prints per-fold OOS metrics.
  Commit: Y | feat(notebook): walk-forward purged CV instead of single split

- [ ] 2.2 Weight decay + dropout + early stopping
  What to do / Must NOT do: Add L2 weight decay to Adam, light dropout in backbone, early stopping on walk-forward val loss. Keep two-stage freeze/unfreeze. MUST NOT remove the Markov-head freeze in Stage 1.
  Parallelization: Wave 2 | Blocked by: 1.4 | Blocks: 2.4
  References: `models/Crypto_Markov_Head.ipynb:221-345`.
  Acceptance criteria: model trained with regularization; val loss tracked.
  QA scenarios: happy: training logs show decay/dropout applied.
  Commit: Y | feat(notebook): add weight decay + dropout + early stopping

- [ ] 2.3 Realistic costs + simple position sizing
  What to do / Must NOT do: In backtester, use maker fee (~0.02-0.05%) + slippage; add fixed-fraction position sizing. Keep hysteresis/regime logic. MUST NOT over-penalize with flat 0.10% as the only cost model.
  Parallelization: Wave 2 | Blocked by: 1.4 | Blocks: 2.4
  References: `models/Crypto_Markov_Head.ipynb:480-640`.
  Acceptance criteria: backtest reports net-of-cost equity.
  QA scenarios: happy: equity curve net of realistic costs.
  Commit: Y | feat(notebook): realistic costs + sizing in backtester

- [ ] 2.4 Report Sharpe / max DD / turnover / OOS equity
  What to do / Must NOT do: Augment evaluation to print Sharpe, max drawdown, turnover, and OOS equity vs buy&hold — not just hit-rate. MUST NOT claim success on hit-rate alone.
  Parallelization: Wave 2 | Blocked by: 2.1, 2.2, 2.3 | Blocks: 3.3
  References: `models/Crypto_Markov_Head.ipynb:440-467`.
  Acceptance criteria: metrics printed; OOS equity positive net of costs.
  QA scenarios: happy: notebook prints full metric suite.
  Commit: Y | feat(notebook): report Sharpe/DD/turnover/OOS equity

### Wave 3: Deployment strategy upgrade

- [ ] 3.1 Vol-adaptive or multi-timeframe regime
  What to do / Must NOT do: In `engine/src/strategy.rs`, optionally replace fixed SMA200 regime with vol-adaptive or multi-timeframe regime. Keep hysteresis ffill. MUST NOT change the ZMQ/feature contract.
  Parallelization: Wave 3 | Blocked by: 2.4 | Blocks: 3.3
  References: `engine/src/strategy.rs:88-122`.
  Acceptance criteria: strategy compiles; tests cover new regime.
  QA scenarios: happy: `cargo test --lib strategy` passes.
  Commit: Y | feat(strategy): vol-adaptive regime option

- [ ] 3.2 Uncertainty / confidence gate
  What to do / Must NOT do: Add a gate (skip trades when model uncertainty or vol is high). If no uncertainty head exists yet, use vol-based gate. MUST NOT trade blindly in high-uncertainty regimes.
  Parallelization: Wave 3 | Blocked by: 2.4 | Blocks: 3.3
  References: `engine/src/strategy.rs:88-122`.
  Acceptance criteria: gate logic unit-tested; default pass-through safe.
  QA scenarios: happy: `cargo test --lib strategy` passes with gate tests.
  Commit: Y | feat(strategy): confidence/vol gate on entries

- [ ] 3.3 Model metadata + canary/rollback on retrain
  What to do / Must NOT do: Persist `meta.json` (train date, data range, exchange, threshold) in the inference container; add a canary/rollback note for model swaps (redeploy with new model.pt + norm_stats.json; verify parity + /api/accuracy in paper mode before live). MUST NOT auto-promote to live without paper validation.
  Parallelization: Wave 3 | Blocked by: 2.4 | Blocks: —
  References: `inference/README.md`, `deploy/docker-compose.yml`.
  Acceptance criteria: meta.json loaded; redeploy verified in paper.
  QA scenarios: happy: redeploy + parity test + /api/accuracy sane in paper.
  Commit: Y | feat(deploy): model metadata + canary/rollback on retrain

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Parity audit — Wave 0.6 green; Colab vs Rust feature vectors bit-close
- [ ] F2. Build/test — `cargo build --release` + `cargo test --release --lib` clean
- [ ] F3. Live verify — redeploy; /api/predictions returns stable non-muted predictions; /api/status healthy; /api/accuracy reflects data after 24h
- [ ] F4. Scope fidelity — no ZMQ/protocol/candle-interval/parity-gate/live-guard changes

## Commit strategy
- Wave 0: 5 commits (0.1, 0.2, 0.3, 0.4, 0.6; 0.5 is no-op/verified).
- Waves 1-3: one atomic commit per todo, grouped by wave.
- Each commit independently buildable (`cargo build` passes after each Rust commit).
- Final verification wave does not produce commits (read-only audit).

## Success criteria
1. Parity test green: Colab and Rust produce bit-close feature vectors (epsilon 1e-6).
2. `vwap_dev` is log(close/vwap); VWAP is rolling (Σ tp×vol / Σ vol), not candle vwap.
3. Normalization identical at train and serve (global z on raw features).
4. norm_stats.json + meta.json exported versioned with checkpoint.
5. After Wave 0 redeploy: /api/predictions returns stable, non-muted predictions.
6. After retrain (Waves 1-3): OOS equity positive net of realistic costs; Sharpe/DD reported.
7. Exchange mismatch resolved (train == deploy exchange/symbol).
8. All tests pass: `cargo test` (full suite) clean; notebook CV + backtest run end-to-end.
