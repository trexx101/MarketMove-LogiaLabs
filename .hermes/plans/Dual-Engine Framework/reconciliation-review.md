# Dual-Engine Framework — MASTER PLAN & Reconciliation (DEFINITIVE)

**Owner:** teech + Inah · **Ratified:** 2026-09-01 · **Branch:** `feature/options-momentum-engine` @ `389b93f`
**Design spec (source of truth for implementation):** `DESIGN-SPEC.md` in this folder
**Reviewed artifacts:** original dual-engine spec (vision doc, superseded), external review `pasted-text-2026-09-01` (largely adopted)
**Canonical baseline:** `.hermes/plans/options-momentum-engine/PLAN.md` (D1–D21) + settled design 2026-08-16

## 0. STATUS — RATIFIED

All six decisions below were approved by Inah on 2026-09-01 ("i support the free proxy path… we
agree on everything else"). The external architectural review independently converged on the same
rulings (engine-scoped gate, spread_intent_log, 2-leg cap, 25% cap). This document is now the
execution master plan; `DESIGN-SPEC.md` is the engineering contract. Original spec artifact is
superseded — keep for provenance only, do not implement from it.

### Ratified decision record

| ID | Decision | Ruling | Cost |
|----|----------|--------|------|
| D-A | Quota vs vol universe | **Free proxy path.** Offline ranking from free candles/VIX; OpenD options API only for Top-K finalists. ORATS ($99/mo verified: $99=20k req, $199=100k) deferred behind `VolFeaturesSource` trait; ThetaData free tier to be verified once (historical depth unconfirmed) | $0 + ~1d build |
| D-B | Macro gate vs Engine-2 high-VIX regimes | **Engine-scoped gate variants**, hardcoded/rail-tier. Middle band 20–25 permissive both families; arbiter allocates | — |
| D-C | NVDA / single names | **Rejected.** ETF-only pool v1: XLK, XLY, XLE | — |
| D-D | v1 regime features | **HV percentile + VIX slope proxy**; true IV rank deferred | — |
| D-E | Capital cap | **25% retained**; arbiter splits it (0.80/0.20 <20 VIX; 0.50/0.50 20–25; 0.20/0.80 ≥25) | — |
| D-F | Leg-grain intent | **New `spread_intent_log`**; `exit_intent_log` untouched | — |

### Adopted from external review
- 2-leg cap (straddle/strangle only) — butterfly/condor dropped
- Persistent ZMQ order daemon parked until micro/live tier
- WAL + busy_timeout as first deliverable

### Corrected/adjusted from external review
- Gate rules: added the 20–25 middle band they left undefined
- Their ExitArbiter priority had wing-target above max-loss; DESIGN-SPEC §6 inverts this
  (risk exit outranks profit-take on stale quotes)
- "Engine crashes on SQLITE_BUSY" overstated — latent risk (single writer today), still fixed by WAL-first
- ORATS §8 performance numbers remain rejected as deploy constants

---

## 1. Verdict

The spec is a **vision document, not an architecture spec**: the Engine-2 concept is sound and
internally coherent, but ~40% of its concrete claims (quotas, module paths, polling cadence,
DB pragmas, transport) contradict the code as it exists today. Three items are outright wrong
(tier-60 assumption, WAL pragmas "already in place", Rust Protobuf client). Three are design
conflicts with locked invariants (single ExitArbiter, staged-ladder-only exits, tier-20 quota).
The spec cannot be implemented as written. Distilled to its intent and re-sequenced, the core
is worth building — incrementally, paper-first, exactly like Engine 1 was.

---

## 2. Core Architectural Intent (distilled)

Strip the formatting noise and the spec says six things:

1. **Second options engine, same binary.** Add a multi-leg *Volatility Regime Engine*
   (straddle / strangle / reverse butterfly / reverse condor) beside the existing single-leg
   *Directional Momentum Engine*. Both are options strategies; they differ in alpha source
   (directional trajectory vs RV−IV mispricing) and payoff construction.
2. **Unified risk layer above both.** One macro gate (VIX level + 5d slope, FOMC/CPI blackouts)
   and one **global capital arbiter** that shifts capital weight `w_dir : w_vol` by VIX regime
   (80/20 < VIX 20 → 20/80 ≥ VIX 25), under a 30%-of-equity deployed-premium cap.
3. **One OpenD connection, budgeted by scheduler.** Both engines' quote needs are met by a
   staggered polling governor that keeps total request volume under the rate limit with 40%
   headroom reserved for orders.
4. **Multi-leg lifecycle = write-ahead state machine.** Spread parent + leg children, staged
   entry with legging-rollback, staged exit ladder, circuit-breaker overrides — i.e. extend
   the Engine-1 reconciliation doctrine to leg groups.
5. **Persistence is already bifurcated.** SQLite for relational state (add 3 tables:
   `option_spread_positions`, `option_spread_legs`, `volatility_regime_log`), Parquet tape
   for dense ticks, partitioned `{engine_family}/{underlying}/{chain}/{date}`.
6. **Decoupled validation.** Nightly hyperopt runs two evaluators with different gates:
   directional keeps Spearman IC ≥ 0.03 + all-folds-positive; volatility rejects IC entirely
   and gates on Profit Factor ≥ 1.40, MaxGain/MaxLoss ≥ 1.50, positive-period freq ≥ 50%.

Of these, **(5) is already done**, (2) partially exists (macro gate yes, capital arbiter no),
(6) partially exists (directional hyperopt yes, promotion state machine yes, vol evaluator no),
(1)(3)(4) are new build.

---

## 3. Reconciliation — Spec Claims vs Code Reality

| # | Spec claim | Code reality | Severity |
|---|-----------|--------------|----------|
| R1 | "assumed Tier 60 for accounts > 100k HKD"; 12+ volatility symbols + 3 directional polled concurrently | **Verified tier = 20** (config `quota_tier: 20` default, Inah-verified). Quota = unique stocks per 30 days — 15+ symbols with chain rolls **cannot fit**. Spec's entire polling budget table is built on the wrong tier. | **BLOCKER** |
| R2 | "hard API rate limit of 30 req / 30s rolling" as *the* quota | Conflates two limits. Snapshot rate limit ≈ 30–60 req/30s (per-endpoint, see `FIELD_MAPPING.md`); the binding constraint is the **subscription/historical quota** (tier 20). | High |
| R3 | Poll Group A every 15s, Group B every 60s, 1560 ticks/contract/day | Settled design D2: recorder is a **separate process, own OpenD connection**, full-ladder snapshots every **15 min**; engine `poll_interval_secs = 300`. 15s polling ≈ 11k req/day — ~40× the budget, and contradicts the design chosen *because* of quota. | High |
| R4 | Module tree `core/ arbiter.rs`, `directional/`, `volatility/`, `data/moomoo/ Protobuf TCP Client` | No such tree. Reality: flat `engine/src/` with `options/` (chain_selector, entry_executor, exit_arbiter/, staged_ladder/, macro_gate, reconciliation, intent_log, circuit_breaker, sizing, config_store, paper_executor), `options_recorder/` + `bin/options_recorder.rs`, `hyperopt/`, `data/moomoo.rs`. **No Rust Protobuf SDK exists** — all Moomoo I/O shells out to Python scripts (`data/moomoo.rs`, recorder script, `trade/place_order.py`). | High (doc fiction) |
| R5 | "All tables created using strict WAL; synchronous=NORMAL; busy_timeout=5000" — stated as existing fact | **False.** Live `data/candles.db`: `journal_mode=delete`, `synchronous=FULL(2)`, `busy_timeout=0`. No WAL pragma anywhere in code. Recorder POSTs to engine API specifically because there is one DB writer — the SQLITE_BUSY problem is solved architecturally, not by pragmas. | Medium (real latent risk) |
| R6 | `existing tables remain structurally locked`, add 3 tables | Consistent with practice — additive DDL in `db.rs` (`CREATE TABLE IF NOT EXISTS`) is the established migration path. Note: **AGENTS.md's "always run drizzle generate/migrate" rule is an artifact — there is no drizzle in this repo.** Schema changes go through `db.rs` DDL + idempotent `pragma_table_info` probes. | Low (rulebook contradiction) |
| R7 | ZMQ V3 bridge to Python TCN/LightGBM | Correct — `engine/src/bridge.rs` uses `zeromq::ReqSocket`. | ✓ matches |
| R8 | Parquet tape, Snappy, partition `{underlying}/{chain}/{date}.parquet` | Correct — `options_recorder.rs` + `bin/options_recorder.rs` (Snappy confirmed, partition layout per D2). Storage math (110KB/contract/day) is plausible at 15-min cadence; at the spec's 15s cadence it's ~40× more. | ✓ mostly matches |
| R9 | Macro gate = global arbiter input | `options/macro_gate.rs` exists but is an **entry DENY gate** (VIX > 30 → deny; steep slope → deny; FOMC/CPI/NFP blackout). Engine 2's core regimes (Crisis VIX≥30, Recovery 20–30) are **exactly the states the current gate blocks**. | **Design conflict** |
| R10 | Dedicated `volatility::exit_arbiter` | Locked invariant #1: **single ExitArbiter, fixed priority, all exits**. New vol triggers (IV crush, wing breakeven, 80% loss) must be added as trigger classes to the existing arbiter, not a second one. | Design conflict |
| R11 | Legging protocol Step 4: "Emergency Rollback — Immediately Unwind / Market-Close filled legs at Bid" | Locked invariant #3: **staged exit ladder, never raw market orders**. Rollback must go through the ladder (bid+k → bid → bid−max_slippage). Raw-market rollback is only acceptable as the circuit-breaker's deep-limit last resort. | Design conflict |
| R12 | Single-leg position model assumed away | `option_positions` is strictly single-contract (one `contract_code`, one `side`). Staged ladder, intent log, reconciliation, and paper executor are all keyed 1 position : 1 contract. Multi-leg needs a parent/leg split threaded through **four** existing modules. | High (breaking surface) |
| R13 | Capital arbiter `compute_position_budget()` 30% cap | No global arbiter exists. Engine-1 sizing caps at **25%** deployed premium, per-position debit-% slider, 1 position/underlying, 3 max. 30% vs 25% must be reconciled; also this is the direct answer to Inah's 08-28 "holistic budget allocation" ask — equities flow needs a slot in this model too, spec only covers options. | Medium |
| R14 | Volatility feature vector needs `IV_Rank_252d`, term structure (IV_60d/IV_30d), HV_20d for 12+ symbols | **No historical IV source exists.** Tape only covers 3 directional chains, and only since P1. 252-day IV rank is unbootstrappable from own data; no vendor wired (Finnhub/yfinance IV quality is poor). Spec never names a source. | **BLOCKER (data)** |
| R15 | Catalyst proximity scoring (earnings in [3,10]d) | Per-ticker earnings calendar was already an **open item** in the settled plan — still unresolved. | Medium |
| R16 | Hyperopt: IC gate ≥0.03 + all folds positive (directional) | Matches `hyperopt/promotion.rs` (all-folds-positive enforced L148–159). Promotion additions are consistent with the **fully-manual promotion** decision (08-28) — spec doesn't contradict it, good. | ✓ matches |
| R17 | Performance table (win rates, Sharpe 1.8–2.2 crisis, "68%–117.47%", "324% win-to-loss ratio") | Sloppy at best: a win *rate* of 117% is meaningless (it's a ratio of counts); 2008-vintage straddle claims; Sharpe ranges are invented-looking. These are paper-simulation numbers with no fees/slippage model for 4-leg retail legging. **Do not encode as deploy constants.** | High (credibility) |

---

## 4. Critical Risks & Breaking Changes

**RISK-1 · Quota ceiling kills the spec's universe (BLOCKER).**
Tier 20 = 20 unique underlyings per 30 days. Directional already uses 3. The spec's volatility
pool is 12 candidates + 3 sector ETFs = 15 more, each consuming quota on discovery *and* on
every chain roll. Even with Top-K=3 *active*, the scanner must evaluate all 15 → quota spent.
Options: (a) upgrade Futu tier; (b) shrink vol universe to ≤ ~8 symbols including directional
and roll rarely; (c) evaluate the pool offline (daily candles from Yahoo/Moomoo klines are cheap;
only the Top-K winners touch the options API); (d) stagger rotation so ≤ 3 new symbols/month.
**(c) is the architecturally correct answer:** IV rank can be approximated from HV + price data
for *ranking*, with options snapshots only for the 3 finalists. This inverts spec §2.2's flow.

**RISK-2 · Macro gate polarity inversion.**
Engine 2 wants to *buy volatility* when VIX ≥ 25–30; the existing macro gate *denies entries*
there. If Engine 2 shares the gate, it never trades its best regimes. If it bypasses the gate,
we violate invariant #2 (risk layer applies to all versions, non-optimizable). Resolution:
make the gate **engine-scoped** — directional keeps VIX-high denial; volatility gets a
*circuit-limit* variant (e.g. deny only when VIX slope is vertical / liquidity crisis proxies)
— with both variants still hardcoded and non-optimizable. Requires Inah's sign-off; this is a
change to a locked invariant's *application*, not its spirit.

**RISK-3 · Multi-leg breaks the write-ahead doctrine's grain.**
Intent log, staged ladder, and reconciliation all persist at position grain. A 4-leg condor has
intermediate states the spec's DDL doesn't model: `1-of-4 filled`, `legging aborted, rollback
in progress`, `rollback stage 2 failed`. The spread table's `EXITING_STAGED` / `CIRCUIT_BREAKER`
statuses are not enough; legs need their own intent rows and the arbiter needs leg-group
semantics ("never leave a spread with unhedged delta overnight in paper→micro transition").
This is the single largest build in the whole spec.

**RISK-4 · No historical IV bootstrap.**
Without a 252-day IV history source, `IV_Rank_252d` and the regime classifier's IV/HV spread
cannot be computed. Either accept a 6–12 month cold-start recording tape before Engine 2 can
gate on anything (unacceptable), or pick a vendor/synthetic proxy now. Realistic proxy path:
HV percentile rank + VIX term structure as regime inputs in v1; true IV rank later when the
tape has depth. The spec's feature vector should be treated as v2.

**RISK-5 · SQLite is not running WAL.**
The spec asserts pragmas that don't exist. Current architecture dodges SQLITE_BUSY via
single-writer (recorder POSTs to engine). That's fine today but fragile: hyperopt reads,
API reads, and engine writes on `journal_mode=delete, busy_timeout=0` will throw
`SQLITE_BUSY` under any concurrent write attempt. Turning on WAL + busy_timeout is a small,
high-value, **independent** change — but it needs a restart-window and a test (recorder +
engine + API hammering concurrently), and `synchronous=NORMAL` under WAL is safe; under
delete mode it would risk corruption on power loss. Do this once, correctly, before either
engine needs it.

**RISK-6 · Spec's execution router assumes capabilities OpenD lacks.**
Correctly identified in spec §6 — OpenD has no native complex-order routing. But the proposed
mitigation's IOC-1500ms legging is aggressive for a retail gateway where Python-script
round-trips per order are ~100–500ms each; a 4-leg all-or-none at 1.5s/leg timeouts is not
achievable with shell-out transport. Legging timeouts must be seconds, and the router must be
async-native (a persistent Python order process, not one-shot script spawns). Reuse the
`place_order.py` pattern but as a long-lived process.

**RISK-7 · NVDA in the volatility universe.**
NVDA was *removed* from the active equities roster for a reason (model roster decision).
Re-introducing it as a vol candidate may be intentional (earnings kurtosis) but needs an
explicit decision, plus per-symbol margin/premium reality checks on single-name options.

**BREAKING CHANGES (if spec were implemented literally):**
1. Moving code into `directional/` / `volatility/` trees → churn with zero behavior gain; the
   existing `options/` module *is* the directional options engine. Rename it `options/directional/`
   at most; do not refactor for the spec's cosmetic tree.
2. Replacing the single ExitArbiter with two → violates invariant #1; do not.
3. 15s polling scheduler → quota blowout; do not.
4. Schema: the 3 new tables are additive and safe as proposed (with `engine_family` column
   added to spread table for future proofing, and timestamps aligned to our ms-unix convention —
   which the spec already uses ✓).

---

## 5. Spec Document Defects (to fix in the artifact itself)

1. `[cite: N]` artifacts throughout — leftover from whatever generated this; strip all.
2. Broken ASCII table in §2 (misaligned `│`, merged cells) — re-draw or replace with a list.
3. Sections 2–8 lose their `##` heading markers mid-doc (numbered headers collapse into prose
   lines like "2. Universe & Asset Expansion Framework..."); the module-tree code block is never
   closed, swallowing subsequent content.
4. Math is double-escaped in tables (`$\\ge$`) but single-escaped in prose (`$\ge$`) — pick one.
5. Rust code blocks are not fenced (poller, arbiter enum, capital arbiter) — wrap them.
6. Typos: "Max Gain/Loss Ratiom", "major earning" (truncated), "324% win-to-loss ratio"
   mislabeled as win frequency.
7. §8 performance table: impossible values (117.47% positive-period frequency); Sharpe ranges
   unsourced; mark as *research-paper claims, unverified* or remove.
8. Missing: any data source for 252d IV history; any earnings-calendar source; any handling of
   the equities flow in the capital arbiter.

---

## 6. Realistic 1-Week Phased Plan

**Framing:** Engine 1 took ~4 weeks of phased work (P0–P5) to reach paper. One week buys
**the safe foundation** — decisions locked, schema in place, regime/science validated offline,
zero risk to the live paper flow. The multi-leg router, legging protocol, and hyperopt vol
evaluator are weeks 2–4 and are explicitly **out of scope** here.

**Pre-flight gate (Day 0, before any code):** Inah decisions on:
- D-A: quota strategy (RISK-1) — recommend **offline ranking + Top-K-only options API**.
- D-B: engine-scoped macro gate (RISK-2) — recommend scoped variant, hardcoded both.
- D-C: NVDA in vol universe (RISK-7).
- D-D: v1 regime features = HV-percentile + VIX term structure proxy (RISK-4), true IV rank deferred.

### Phase W1 · Days 1–2 — Spec freeze + WAL hardening (independent, shippable alone)
- Rewrite spec artifact: fix §5 defects, correct R1–R5 claims, align module paths to reality.
- Enable WAL + `busy_timeout=5000` + `synchronous=NORMAL` at `db.rs` pool init (idempotent),
  with a concurrency test (engine write + API read + hyperopt read simultaneously).
- Deploy in the next restart window; verify `PRAGMA journal_mode` = wal on live DB.
- **Exit criteria:** 427+ tests green, live pragma verified, zero SQLITE_BUSY under hammer test.

### Phase W2 · Day 2–3 — Schema + state model (additive DDL only)
- Add `option_spread_positions`, `option_spread_legs`, `volatility_regime_log` to `db.rs`
  (spec DDL is usable almost verbatim; add `engine_family`, keep ms-unix timestamps, UUID PKs
  per project rule).
- Add leg-grain intent rows: extend `exit_intent_log` semantics or add `spread_intent_log`
  (decide in design note first — this is the grain question from RISK-3).
- Unit tests: tables created, FK cascade, idempotent re-run.
- **Exit criteria:** DDL tests pass; no change to any live code path (pure additive).

### Phase W3 · Days 3–5 — Regime science, offline (zero quota spend)
- `volatility/regime.rs`: pure-function 4-regime classifier on (VIX level, VIX 5d slope,
  IV−HV proxy spread) — unit-tested against the spec's classification tree, edge cases on
  boundaries (VIX exactly 20/25/30).
- Data source: HV_20d from existing daily candles (Yahoo path already wired); VIX from CBOE
  fetcher (already exists: `data/cboe.rs`). IV proxy = realized-vol percentile rank (252d).
- Backfill 252d daily candles for the candidate pool; run classifier over history; produce a
  regime calendar and sanity-check against known regimes (this is the cheap verification of the
  whole thesis before any options API spend).
- **Exit criteria:** regime history report reviewed by Inah; classifier deterministic + tested.

### Phase W4 · Days 5–6 — Capital arbiter + engine-scoped macro gate
- `core/capital_arbiter.rs` (or `options/capital_arbiter.rs` — decide by boundary): pure
  function `(equity, vix) → (dir_budget, vol_budget)`, config-driven weights, 25% total cap
  (align with existing sizing cap — do not silently move it to 30%).
- Wire the arbiter as *advisory sizing input* only — no behavior change to Engine 1 entries yet.
- Macro gate: add `engine_family` dimension (directional keeps current thresholds; volatility
  gets its scoped variant), still hardcoded, still non-optimizable, UI-visible decisions.
- **Exit criteria:** arbiter + gate unit tests; Engine-1 paper flow byte-identical in behavior
  (regression: existing entry/exit integration test unchanged and green).

### Phase W5 · Day 7 — Review gate + Phase-2 plan
- Integration: deploy to VPS, verify no regressions (427+ tests, paper flow untouched).
- Write the Phase-2 plan: spread position lifecycle (paper), leg-aware intent log, vol exit
  triggers as new classes in the single ExitArbiter, long-lived order process for legging.
- **Explicit non-goals this week:** complex order router, legging protocol, volatility
  hyperopt evaluator, UI, any live/micro tier changes.

### Week-1 Definition of Done
- [ ] Spec artifact corrected and re-baselined against code
- [ ] WAL live + verified
- [ ] 3 spread tables + intent-grain decision landed (tests green)
- [ ] Regime classifier validated on historical data (report reviewed)
- [ ] Capital arbiter + engine-scoped gate merged, Engine 1 behavior provably unchanged
- [ ] Phase-2 plan written and waiting on sign-off

---

## 7. Decisions — RATIFIED (see §0)

All six decisions approved by Inah 2026-09-01. Full rulings in the §0 table above and
encoded in `DESIGN-SPEC.md` §0. Remaining *future* decision points (flagged, not blocking):

| ID | Future decision | Trigger |
|----|----------------|---------|
| F-1 | ThetaData free-tier verification (30 min) | Before any vendor conversation |
| F-2 | Alert routing for spread CIRCUIT_BREAKER | Before micro tier |
| F-3 | Engine-1 sizing starts consuming arbiter splits | After 2 weeks of logged splits |
| F-4 | Pool expansion (any symbol) | Quota re-review mandatory first |
| F-5 | ORATS adoption | Only with paper evidence that proxy ranking is unstable |

---

## 8. Change log

- **2026-09-01:** Ratified. Free proxy path adopted (Inah). External review reconciled:
  2-leg cap, spread_intent_log, ZMQ daemon parked, gate middle-band fix, exit-priority
  inversion adopted. Master plan finalized; `DESIGN-SPEC.md` created as implementation contract.
- **2026-08-31:** Initial reconciliation of original dual-engine spec vs code reality
  (tier-20 quota blocker, WAL absence, invariant conflicts, module-tree fiction).
