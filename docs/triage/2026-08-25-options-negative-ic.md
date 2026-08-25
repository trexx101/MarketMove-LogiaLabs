# MarketMoves options-flow triage — 2026-08-25 (negative-IC round)

Carved by teech at Inah's request via `omh-triage` (v0.1, adapted: this repo has
no GitHub issue tracker — the inventory is synthesized from
`reports/2026-08-25_state_report.md`). Ground-truth anchored to repo HEAD `f8309d3`.

## Headline finding

5 items filed → 0 stale, 1 recast (ISSUE 4 refiled smaller), 4 live.
The load-bearing item (NEG-IC) is **genuine, not a bug**: hypothesis (a) "sign
inversion in the eval objective" is DEAD (independent Python re-implementation
of `eval.rs` reproduces stored fold ICs to 3 decimals); hypothesis (b) "the
sma_regime signal genuinely mean-reverts at the 5-day horizon" is SUPPORTED —
with two Skeptic-imposed caveats: the evidence is strong for XLF/SMH and weak
for QQQ, and the economic magnitude is small for QQQ/SMH. The NEG-IC item is
also **not the critical path** — promotion advances a status column nothing
consumes (OptionsScheduler is not enabled), so ISSUE 2 (P7 enablement) outranks it.

## Run shape

- Inventory: 5 items (NEG-IC, OPTIONS-NOT-LIVE, TAPE-SHORT, CONFIG-KV-EMPTY, PARQUET-ZERO)
- Roles dispatched: Maintainer, Skeptic — **both subagent runs died to provider
  503s mid-pass; the orchestrator executed both passes directly with the same
  grounding discipline (code reads + live-DB reproduction, no vibes)**. Recorded
  here honestly rather than presenting clean subagent output.
- Conflicts surfaced: 0 matrix conflicts; 2 user decisions escalated (below)
- Orchestrator override (T4): ISSUE 4 disposition recast→refile-smaller applied
  as a dispatcher-judgment call.

## Verdicts (Maintainer × Skeptic → disposition)

| # | Issue | Maintainer | Skeptic | Disposition |
|---|-------|-----------|---------|-------------|
| 1 | NEG-IC | live, hypothesis (b) | keep (w/ caveats) | **LIVE — fix the grid, don't chase it ahead of P7** |
| 2 | OPTIONS-NOT-LIVE | live (P7 unbuilt) | keep | **LIVE — true #1 critical path** |
| 3 | TAPE-SHORT | live (watch) | keep | LIVE watch — keep recorder running, no action |
| 4 | CONFIG-KV-EMPTY | live/low-pri | refile-smaller | **Recast: seed the KV only when first override is needed** (registry defaults at `config_store.rs:67` make the empty table functionally inert) |
| 5 | PARQUET-ZERO | watch-item | keep | LIVE watch — recheck 2026-08-26 AM that today's parquet flushed non-zero |

## ISSUE 1 — NEG-IC: evidence stack

### What is dead
- **Hypothesis (a), sign inversion: DEAD.** `eval.rs` signal=(close−SMA)/SMA
  zeroed below threshold (L56-81); label=5-bar forward return (L84-92); Spearman
  per fold (L136-149); 5 walk-forward folds, embargo 136 (L155-212). Unit test
  asserts positive IC on an uptrend (L281-291). `candidate_store.rs` stores
  mean_ic verbatim; `promotion.rs` gates on it directly. Independent Python
  reimplementation reproduces stored fold ICs to ~3 decimals on all 9 QQQ configs.

### What is confirmed
- Negative IC is not noise: random-signal baseline mean +0.002 ± 0.042; observed
  −0.065…−0.286 ⇒ XLF ~5σ below the noise floor.
- Negative at EVERY horizon (h1/h2/h5/h10/h21) and EVERY sma_window tested
  (10/20/40/50/100/200) — no window saves momentum. Even the binary regime
  signal (sign of close−SMA40, the analog of the deployed entry rule) scores
  negative IC: QQQ −0.036, SMH −0.024, XLF −0.107.
- Present on AAPL (−0.13), NVDA (−0.14), GLD (−0.09) under the same eval —
  market-wide short-term reversal, not an ETF artifact.
- Fold coverage: 2023-01→2023-11, →2024-09, →2025-07, →2026-05, →2026-08.
  Negative in essentially every fold-year.
- Threshold zeroing is not the cause: thr=0 vs thr=0.01 changes mean IC by <0.002.

### Skeptic caveats (do not skip these)
1. **"Flip clears the gate" is arithmetic, not new evidence.** Flipping the sign
   of a Spearman negates it by definition; +0.065 = −(−0.065). The flip test
   proves the gate math works, not that an edge is tradeable. The real evidence
   is fold-consistency: fold-level t-stats on the flipped IC give QQQ t=2.25
   (p≈0.09 — WEAK), SMH t=4.92 (p<0.05), XLF t=5.43 (p<0.05), 4 df.
2. **Economic magnitude check (tercile spread, mean-reversion direction):**
   XLF +0.39%/week (corroborates); SMH −0.10%/week and QQQ −0.04%/week
   (do NOT corroborate at coarse granularity). For QQQ/SMH the reversion is
   statistically detectable but economically thin. XLF is the only name where
   the effect is both statistically strong and economically coherent.
3. **Strategy tension reconciled, not ignored:** the deployed long-only
   threshold strategy (Sharpe 1.43, 2022–2025) trades regime CROSSINGS and
   sits flat below SMA-40; it never trades distance-magnitude, and its window
   predates the two most negative folds (2025-07→2026-08). A long-only regime
   filter can be profitable while the symmetric rank-IC of its underlying signal
   is negative. The shelved Rhai mean-reversion script was a different, weaker
   implementation — not a test of flipped sma_regime. No contradiction; but also
   no license to assume a flipped-strategy backtest would reproduce Sharpe 1.43.

### Proposed solution (staged, not executed)

**A1 — `direction` parameter for sma_regime (the fix).**
Add `direction ∈ {+1, −1}` to the family: `eval.rs::sma_momentum_signal`
multiplies the signal by it; `param_defs_for_family` adds it to the grid
(3×3×2 = 18 configs, still trivial). Rationale: the grid discovers both regimes
honestly instead of silently sign-flipping a strategy named "momentum"; stored
params_json carries the direction explicitly so nothing downstream infers sign.
Add a unit test: downtrending series + direction=−1 ⇒ positive IC (mirror of the
existing uptrend test).

**A2 — Gate hardening (optional, user decision).**
Promotion currently requires mean_ic ≥ +0.03 with no fold-consistency rule.
QQQ's flipped mean (+0.065) is carried by 4 folds against one at −0.046. Add
`min fold IC > 0` to the NEW→PAPER gate (`promotion.rs`). Cheap, kills the
"one bad fold dragged along" failure mode.

**A3 — Nightly loop: keep running, do not pause.**
The nightly is the only continuous monitoring signal for candle ingestion and
the eval pipeline, and the rows it stores are honest records of what the
objective measured — this triage round exists because those rows accumulated.
~27 rows/night × ~200 B is negligible. After A1 ships, stored candidates become
meaningful again. Do NOT skip storing sub-gate candidates (loses the diagnostic
history).

**A4 — No DB surgery on the existing 108 rows.**
Per standing rule (no purges without permission). Leave them NEW as historical
evidence; marking REJECTED adds a write-path exercise for zero information.

**Sequencing (the Skeptic's actual finding):** NEG-IC is the most VISIBLE
problem, not the critical path. Promotion advances a status column nothing
reads — `main.rs:404-414` runs only the promotion applier; OptionsScheduler is
never constructed; no code consumes PAPER/LIVE candidates. Fix A1/A2 because
they're cheap and stop the nightly from accumulating misleading evidence, but do
NOT build promotion-consumer machinery until ISSUE 2 (P7 enablement) is designed.

## Escalated to user (2 decisions)

1. **Fold-consistency gate (A2):** add "min fold IC > 0" to NEW→PAPER? Recommended: yes.
2. **Direction = −1 in promotion at all?** The engine is named "Options
   **Momentum** Engine". If the grid finds mean-reversion dominates (likely for
   XLF), do we let direction=−1 configs promote, or constrain the grid to
   momentum-only and accept a starved promotion path? This is a strategy/thesis
   decision, not an engineering one. Triage recommendation: allow both, let the
   gate pick; the IC gate is the honesty contract, not the name on the box.

## EXECUTED 2026-08-25 18:50 UTC (Inah approved D1/D2/D3 with "PROCEED")

- Commit `b45ac1d` on feature/options-momentum-engine: A1 (direction axis,
  grid 9→18) + A2 (fold-consistency gate: every walk-forward fold must be
  positive at every promotion transition; fold_ics carried in
  PromotionEvidence with serde-default so pre-gate queued evidence still loads).
- Image `marketmarkovnet/engine:negic-fix` (=latest, 6d9e948e6f4c) rebuilt and
  redeployed; container healthy, all three models predicting, DB untouched.
- Verify harness (/tmp/negic-verify) proves: exact negation 0.887611↔−0.887611,
  absent-direction-key = +1, all-positive-folds pass, flipped-QQQ profile
  (one fold −0.046) REJECTED, XLF-flipped profile passes, legacy evidence OK.
- Pre-existing: 8 compile errors in engine/src/api/tests.rs (proven present
  with changes stashed — stale signatures from a8b35c3); cargo test --lib stays
  red on baseline. Fixing them is a separate task, not in this triage scope.
- Next meaningful observation: tonight's 20:40 UTC nightly self-waking run
  (~1h50m after this deploy) is the FIRST direction-aware run — it will grid
  18 configs/slot including direction=−1 and store whatever clears min_trades.
  Watch tomorrow morning for the first negative-direction candidates.

## What this pass did not do

- No P7 enablement design (ISSUE 2) — needs its own round; biggest open item.
- No retest of the deployed equities strategy under the reversion finding — the
  live strategy doesn't consume the IC object; revalidation only matters if A1
  candidates get consumed somewhere.
- No tape-replay work (weeks away by construction).
- The nightly's 20:40 UTC run tonight will store 27 more NEW rows under the old
  sign-blind grid; accepted as-is.

## Lessons surfaced

- **L1:** "Flip clears the gate" is a tautology check, not evidence of edge.
  Fold-level t-stats and tercile-spread economics are the honest tests. Candidate
  for the omh-triage SKILL.md pitfalls when a quant round confirms it.
- **L2:** Both delegated role runs died to provider capacity 503s; orchestrator
  fallback (execute the pass directly with the same grounding requirements)
  preserved the round. The multi-role shape still paid: the Skeptic probes
  (especially tercile economics) changed the fix recommendation — a single-pass
  walk would have shipped "flip the signal, we're done".
- **L3:** A gate applied to an objective nobody consumes is a diagnostic, not a
  control. The 108 failing rows were the health-check that surfaced this finding;
  the promotion machinery itself is inert until P7 exists.

## Methodology note

Anchored against: `eval.rs` / `runner.rs` / `optimizer.rs` / `candidate_store.rs`
/ `promotion.rs` / `config_store.rs` / `main.rs` at HEAD f8309d3; live DB copy
(`/tmp/live_candles.db` from `deploy_data` volume); independent Python
reimplementation + 7 control experiments + 3 Skeptic probes. Next pass should
anchor against the first OptionsScheduler-enabled commit.

⚒️ teech
- 2026-08-25: options negative-IC round (omh-triage v0.1, adapted)
