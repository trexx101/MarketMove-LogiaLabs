# DESIGN SPECIFICATION — MarketMoves Dual-Engine Framework (Engine 2: Volatility Regime)

**Status:** RATIFIED 2026-09-01 (Inah approved D-A…D-F via free-proxy path)
**Branch:** `feature/options-momentum-engine`
**Companion:** `reconciliation-review.md` (master plan), `.hermes/plans/options-momentum-engine/PLAN.md` (Engine 1 canon)
**Rule of precedence:** this doc > original dual-engine spec artifact. Where this doc and code
invariants conflict, invariants win and this doc gets patched.

---

## 0. Positioning

Engine 1 (Directional Momentum, single-leg) exists and runs paper in production. This design
adds **Engine 2 (Volatility Regime)** — 2-leg long-volatility spreads mapped to VIX regimes —
as an additive module family inside the same binary, sharing: one ExitArbiter, one staged
ladder doctrine, one macro gate (engine-scoped), one config store, one SQLite DB, one tape
format. **Nothing in Engine 1's code path changes behavior.**

### Ratified decisions (locked 2026-09-01)

| ID | Decision | Ruling |
|----|----------|--------|
| D-A | Vol universe vs tier-20 quota | **FREE PROXY PATH.** Offline ranking from free candles/VIX; OpenD options API only for Top-K finalists. ORATS ($99/mo) deferred behind a trait boundary; revisit only with paper evidence. |
| D-B | Macro gate vs high-VIX regimes | **Engine-scoped gate variants.** Directional denies high-VIX entries; volatility permits them. 20–25 band permissive for both (arbiter allocates). Hardcoded, non-optimizable, UI-auditable. |
| D-C | NVDA in volatility pool | **REJECTED.** ETF-only vol universe v1: XLK, XLY, XLE. No single names → no earnings gap-risk, no delta-hedging requirement. |
| D-D | v1 regime features | **HV percentile + VIX term-structure proxy.** True IV rank deferred until tape depth exists or vendor adopted. |
| D-E | Capital cap | **25% deployed-premium cap retained.** Capital arbiter *splits* the 25% between engines by VIX regime; total never 30%. |
| D-F | Leg-grain intent log | **New `spread_intent_log` table.** `exit_intent_log` untouched (position grain stays sacred). |

### Additional scope rulings (from reconciliation)

- **2 legs only.** Straddle + strangle for v1. Butterfly/condor DROPPED (4-leg legging risk vs marginal edge).
- **Persistent ZMQ order daemon: PARKED.** Paper v1 reuses `paper_executor`; live/micro order transport is a Phase-2+ problem.
- **Legging rollback never uses raw market orders** — staged ladder for rollbacks too; deep-limit only via circuit breaker.

---

## 1. Invariants (inherited — violation = PR rejection)

1. Strategy layer decides entries only; never places orders.
2. Single `ExitArbiter`, fixed priority, all exits — extended, never duplicated (§6).
3. Staged ladder, never raw market orders; stage-3 failure → CIRCUIT_BREAKER.
4. Write-ahead: every state transition persisted pre-send; DB owns intent, broker owns facts.
5. Sunset hot-swap: positions bound to `strategy_version_id` forever; promotion manual only, daily-candle boundary only.
6. Risk rails hardcoded + non-optimizable: macro gate, DTE<7 exit, delta-drift band, capital cap.
7. One DB writer: engine. Recorder POSTs; no direct recorder→DB writes.
8. Additive schema only: `CREATE TABLE IF NOT EXISTS` in `db.rs` DDL; no `drizzle` (AGENTS.md rule is an artifact — repo has no drizzle).
9. **New — Quota invariant:** OpenD options calls only for Top-K finalists + directional chains; discovery/scoring MUST be offline on free candles/VIX.
10. **New — Engine-family scoping:** every gate decision, event, and config key that differs between engines carries an explicit `engine_family: Directional | Volatility` tag. No implicit global behavior.

---

## 2. Target architecture (module map against REAL source tree)

No `directional/`/`volatility/`/`core/` reshuffle. Engine 1 stays in `options/`. Engine 2 lands
as a sibling family + shared governance modules:

```text
engine/src/
├── options/                        # ENGINE 1 (untouched behavior; gets family tags)
│   ├── chain_selector.rs  entry_executor.rs  entry_integration.rs
│   ├── exit_arbiter/mod.rs         # EXTENDED: new ExitSource variants (§6)
│   ├── staged_ladder/  trailing_stop/  overrides/  circuit_breaker/
│   ├── macro_gate.rs               # EXTENDED: engine-family scoping (§4.2)
│   ├── reconciliation.rs  intent_log.rs  sizing.rs  paper_executor.rs
│   └── config_store.rs             # EXTENDED: vol config keys, rail tier (§8)
├── volatility/                     # ENGINE 2 — NEW
│   ├── mod.rs
│   ├── features.rs                 # VolFeaturesSource trait + ProxyVolFeatures (§4.1)
│   ├── regime.rs                   # 4-regime classifier, pure functions (§4.3)
│   ├── scanner.rs                  # Top-K ranking over proxy features (§4.4)
│   ├── spread_builder.rs           # straddle/strangle strike mapping (§5.2)
│   └── spread_lifecycle.rs         # spread state machine + leg dispatch (§5)
├── governance/                     # NEW — cross-engine (Inah 08-28 holistic ask)
│   ├── capital_arbiter.rs          # 25% cap split w_dir:w_vol by VIX (§4.5)
│   └── engine_family.rs            # EngineFamily enum, shared types
├── hyperopt/
│   └── vol_evaluator.rs            # NEW — regime-conditioned PF gates (§7)
├── data/
│   ├── cboe.rs                     # exists: backfill_vix → regime input
│   └── yahoo.rs                    # exists: candle backfill → HV proxy input
└── db.rs                           # +4 tables DDL (§3)
```

**AppState impact:** adding vol scheduler/config to `AppState` requires updating every test
that constructs it directly (known repo rule). Design choice to minimize that: vol state hangs
off the existing `AppState` only at wiring time; feature types live in modules, tests build
modules directly. If AppState grows, the diff must enumerate every test-file fix.

---

## 3. Data model (additive DDL, repo conventions: TEXT UUID PKs, ms-unix INTEGER)

```sql
-- Spread parent entity (Engine 2 position lifecycle)
CREATE TABLE IF NOT EXISTS option_spread_positions (
    spread_id               TEXT    PRIMARY KEY,          -- UUID v4
    underlying              TEXT    NOT NULL,             -- 'US.XLK'
    engine_family           TEXT    NOT NULL DEFAULT 'volatility',
    strategy_version_id     TEXT    NOT NULL,             -- sunset hot-swap binding
    strategy_type           TEXT    NOT NULL,             -- 'STRADDLE' | 'STRANGLE'
    regime_at_entry         TEXT    NOT NULL,             -- 'CRISIS'|'RECOVERY'|'RANGE_BOUND'|'COMPLACENT'
    dte_at_entry            INTEGER NOT NULL,
    net_debit               REAL    NOT NULL,             -- premium paid, per spread (v1: long-only → always > 0)
    status                  TEXT    NOT NULL DEFAULT 'OPEN',
        -- OPEN | EXITING | CIRCUIT_BREAKER | CLOSED | LEGGING_ABORT (paper: OPEN→EXITING→CLOSED)
    entry_underlying_px     REAL    NOT NULL,
    entry_vix               REAL    NOT NULL,
    entry_proxy_iv_rank     REAL    NOT NULL,             -- proxy value used (audit trail)
    unrealized_pnl          REAL    NOT NULL DEFAULT 0.0,
    realized_pnl            REAL,
    exit_trigger_source     TEXT,                         -- ExitSource as string
    opened_at               INTEGER NOT NULL,             -- ms epoch
    closed_at               INTEGER
);
CREATE INDEX IF NOT EXISTS spread_positions_status_idx
    ON option_spread_positions (status);
CREATE INDEX IF NOT EXISTS spread_positions_underlying_status_idx
    ON option_spread_positions (underlying, status);

-- Child legs; 1:1 with an option_positions row via leg_position_id
CREATE TABLE IF NOT EXISTS option_spread_legs (
    leg_id                  TEXT    PRIMARY KEY,          -- UUID v4
    spread_id               TEXT    NOT NULL REFERENCES option_spread_positions(spread_id) ON DELETE CASCADE,
    leg_position_id         TEXT    NOT NULL,             -- FK → option_positions.id (leg IS a position)
    contract_code           TEXT    NOT NULL,
    side                    TEXT    NOT NULL,             -- v1: always 'BUY' (long straddle/strangle)
    option_type             TEXT    NOT NULL,             -- 'CALL' | 'PUT'
    strike                  REAL    NOT NULL,
    fill_price              REAL    NOT NULL,
    is_closed               INTEGER NOT NULL DEFAULT 0,   -- repo convention: INTEGER bool
    closed_at               INTEGER
);
CREATE INDEX IF NOT EXISTS spread_legs_spread_idx ON option_spread_legs (spread_id);

-- Spread-grain write-ahead intent (D-F). Position-grain rows stay in exit_intent_log.
CREATE TABLE IF NOT EXISTS spread_intent_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    spread_id     TEXT    NOT NULL,
    phase         TEXT    NOT NULL,    -- 'ENTRY_DISPATCH'|'LEG_FILLED'|'EXIT_DECISION'|'EXIT_DISPATCH'|'SPREAD_CLOSED'|'LEGGING_ABORT'
    detail_json   TEXT    NOT NULL DEFAULT '{}',
    timestamp     TEXT    NOT NULL     -- RFC3339, matches exit_intent_log convention
);
CREATE INDEX IF NOT EXISTS spread_intent_log_spread_idx
    ON spread_intent_log (spread_id);

-- Regime audit trail (daily rows)
CREATE TABLE IF NOT EXISTS volatility_regime_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              INTEGER NOT NULL,                     -- ms epoch
    vix_level       REAL    NOT NULL,
    vix_slope_5d    REAL    NOT NULL,
    regime          TEXT    NOT NULL,                     -- detected regime
    gate_directional TEXT   NOT NULL,                     -- ALLOW|DENY
    gate_volatility TEXT    NOT NULL,                     -- ALLOW|DENY
    w_dir           REAL    NOT NULL,
    w_vol           REAL    NOT NULL,
    ranked_pool_json TEXT   NOT NULL DEFAULT '[]',        -- scanner Top-K output
    active_pool_json TEXT   NOT NULL DEFAULT '[]'
);
```

**Key modeling decision:** a leg *is* an `option_positions` row (`leg_position_id`). This gives
each leg the full existing ladder/intent/exit machinery for free; the spread table governs the
*group* (entry atomicity, joint P&L, joint exit decision). No duplication of position state.
`option_positions` gets ONE additive column: `spread_id TEXT` (nullable; NULL = standalone
Engine-1 position) via the repo's idempotent `pragma_table_info` migration pattern.

### Parquet tape
Engine 2 tapes under a **new top-level directory**, directional layout untouched:
`data/options_tape/volatility/{underlying}/{chain}/{date}.parquet` — same `TapeRow` schema,
Snappy. v1 scope: only Top-K finalist chains, same 15-min ladder cadence as D2 (NOT 15s).

---

## 4. Governance & feature layer

### 4.1 VolFeaturesSource (the ORATS escape hatch)

```rust
pub trait VolFeaturesSource: Send + Sync {
    /// Daily feature row per candidate. Called after market close.
    fn daily_features(&self, symbol: &str, as_of: NaiveDate) -> Result<VolFeatureRow>;
    fn name(&self) -> &'static str;   // "proxy" | future "orats" | "thetadata"
}

pub struct VolFeatureRow {
    pub hv_percentile_252d: f64,   // 0..1 — HV20 rank vs own 252d history (PROXY for IV rank)
    pub hv20: f64,                 // 20d realized vol (annualized, close-to-close)
    pub vix_level: f64,
    pub vix_slope_5d: f64,         // (VIX_t - VIX_{t-5}) / VIX_{t-5}
    pub vix_term_slope: f64,       // proxy: vix_slope_5d sign-magnitude (see 4.4 note)
}
```

`ProxyVolFeatures` (v1 impl): HV percentile computed from `equity_candles` (Yahoo backfill,
already wired); VIX from `cboe::backfill_vix` (already wired). **Why good enough:** vol pool is
ETF-only (D-C); ETF IV tracks HV closely (no single-name earnings premium), and the feature is a
*ranking* input, not the entry signal. Validity re-check is mandatory if the pool ever gains
single names. Future `VendorVolFeatures` (ORATS/ThetaData) implements the same trait — swap is
config, not code.

### 4.2 Macro gate — engine-scoped (D-B)

Signature change (the only breaking-internal change; contained, unit-tested):

```rust
impl MacroGate {
    // OLD: pub fn evaluate(&self, vix, slope, calendar) -> MacroGateDecision
    pub fn evaluate(
        &self,
        family: EngineFamily,
        vix: f64, slope_5d: f64, calendar: &[CalendarEvent], now: DateTime<Utc>,
    ) -> MacroGateDecision { ... }
}
```

| VIX zone | Directional gate | Volatility gate |
|---|---|---|
| VIX < 20 | ALLOW | DENY (premium too cheap / choppy; arbiter w_vol ≈ 0 anyway) |
| 20 ≤ VIX < 25 | ALLOW | ALLOW (middle band permissive — arbiter allocates) |
| VIX ≥ 25, slope ≤ threshold | DENY | ALLOW |
| VIX ≥ 25, slope > slope_rail | DENY | DENY (vertical spike = liquidity-crisis rail) |
| Calendar blackout | DENY | DENY (both families; earnings N/A — ETF pool) |

Existing `MacroGateConfig` thresholds become rail-tier config keys per family
(`dir.vix_level_threshold`, `vol.vix_slope_rail`). Old callers pass
`EngineFamily::Directional` and get byte-identical behavior → Engine-1 regression proof.

### 4.3 Regime classifier (pure, deterministic, config-thresholds)

```rust
pub enum VolRegime { Crisis, Recovery, RangeBound, Complacent }

pub fn classify(vix: f64, vix_slope_5d: f64, hv_pct: f64) -> VolRegime {
    // thresholds from rail-tier config (NOT hard literals):
    //   crisis_vix = 30.0, crisis_slope = 0.10 (5d), recovery_vix = 20.0,
    //   complacent_vix = 12.0 (floor), range default band 12–20
    if vix >= 30.0 && vix_slope_5d > cfg.crisis_slope { Crisis }
    else if vix >= 20.0 { Recovery }
    else if vix < 12.0 && hv_pct < cfg.complacent_hv_pct { Complacent }
    else { RangeBound }
}
```

**Boundary tests are mandatory:** VIX exactly 12.0 / 20.0 / 30.0, slope exactly at threshold,
NaN inputs (return RangeBound + warn event, never panic). Classifier output + inputs →
`volatility_regime_log` row daily; event `options::regime` (category `strategy`).

### 4.4 Scanner — Top-K under quota invariant

Runs at daily close on FREE data only:
1. For each candidate in the ETF pool (`US.XLK`, `US.XLY`, `US.XLE` — v1 pool size 3, all may be
   active; pool expansion later needs quota re-review):
   `score = w1·hv_percentile_252d + w2·(hv20/vix_level_normalized) + w3·catalyst_flag`.
   Catalyst = earnings blackout only (ETF earnings events are non-gapping; flag exists for
   future single-name expansion).
2. Rank, bind Top-K (K = min(3, available quota slots after directional chains)) to Engine-2
   session slots. Binding = the ONLY moment OpenD options API may be touched for vol assets.
3. No candidate passes liquidity/quota check → zero vol slots that session + `options::scanner`
   warn event. Quota accounting: directional 3 chains + up to 3 vol chains = 6 unique
   underlyings/30d ≪ tier 20 even with monthly rolls.

**v1 note on `vix_term_slope`:** true term structure (60d/30d IV) is unavailable without a
vendor; v1 uses VIX slope as its substitute and the regime table degrades gracefully
(classifier uses VIX level + slope only; the IV/HV spread condition becomes HV-percentile
condition). No silent approximation: every stored row records which source produced it (`name()`).

### 4.5 Capital arbiter (governance — also serves equities, Inah 08-28)

```rust
pub fn split_budget(account_equity: f64, vix: f64, cfg: &CapitalConfig) -> BudgetSplit {
    let total = account_equity * cfg.deployed_cap_pct;   // 0.25 — NEVER 0.30
    let (w_dir, w_vol) = match vix {
        v if v < 20.0 => (0.80, 0.20),
        v if v < 25.0 => (0.50, 0.50),
        _             => (0.20, 0.80),
    };
    BudgetSplit { dir: total * w_dir, vol: total * w_vol, remaining: total * (1.0 - w_dir - w_vol) /* = 0 */ }
}
```

Weights are rail-tier config (tunable, auditable, but never strategy-hyperopt-optimizable).
**Equities hook (deliberate):** `BudgetSplit` exposes the interface the equities flow will
consume when its own allocation join is built — Engine 2 ships first, the holistic budget
contract is designed now. Advisory-only in week 1: Engine-1 sizing code unchanged until the
arbiter has ≥2 weeks of logged splits to review.

---

## 5. Spread lifecycle state machine (2-leg, paper v1)

```
        scanner binds slot (daily close)
                │
                ▼
   ENTRY_SIGNAL (regime → structure: Crisis→STRADDLE, Recovery→STRANGLE,
                 RangeBound→SKIP-v1, Complacent→SKIP-v1)      ← v1 trades only 2 regimes
                │
   spread_builder: pick expiry (reuse chain_selector DTE window [30,45], monthly preferred),
   strikes: STRADDLE = ATM call+put; STRANGLE = +5% call / −5% put (nearest listed strikes),
   liquidity floors from config_store (bid>0, spread ≤8% mid, OI ≥100)
                │
                ▼
   spread_intent_log: ENTRY_DISPATCH → create spread row (status OPEN) +
   2 × option_positions rows (spread_id set) + 2 × legs rows   ← ALL persisted BEFORE exec
                │
                ▼
   paper_executor fills legs (existing paper machinery) → LEG_FILLED intents
                │   (any leg unfillable in paper = config/data bug → LEGGING_ABORT + alert)
                ▼
              OPEN ── tick/daily eval ──> exit signals at SPREAD grain (§6)
                │                              │ winner selected once for the spread
                │                              ▼
                │                      spread_intent_log: EXIT_DECISION → EXIT_DISPATCH
                │                      → existing staged ladder runs PER LEG, dispatched
                │                        simultaneously, both legs must reach Complete
                │                              │
                ▼                              ▼
   (DTE<7 rail closes spread like any position)  SPREAD_CLOSED intent → status CLOSED,
                                                  realized_pnl = Σ legs, event emitted
```

Failure semantics:
- One leg exits, other stalls at stage 2/3 → ladder continues per doctrine; spread status stays
  EXITING; a stage-3 failure on ANY leg → CIRCUIT_BREAKER on the spread, residual leg stays
  open, entry halt + cooldown + operator alert (identical doctrine to Engine 1, spread grain).
- **No partial-hedge tolerance in paper:** paper fills are deterministic, so LEGGING_ABORT in
  paper means a code defect, not market behavior — treat as P1 bug.
- Micro/live tier (Phase 3+, parked): persistent ZMQ order daemon, legging protocol with
  seconds-scale timeouts, rollback = staged ladder on filled legs.

---

## 6. ExitArbiter extension (invariant #2 preserved)

Additive enum variants — existing values 1–6 keep their numbers (persisted `exit_signals.priority`
rows stay meaningful):

```rust
pub enum ExitSource {
    OperatorForceClose = 1,
    CircuitBreaker = 2,
    DteOverride = 3,          // SHARED rail: DTE < 7 closes spreads too
    TrailingStop = 4,
    RoiTable = 5,
    SignalReversal = 6,
    IvCrush = 7,              // NEW: proxy IV (HV-pct) drops > cfg.iv_crush_pct from entry
    MaxLossExhaustion = 8,    // NEW: spread lost ≥ cfg.max_loss_pct (80) of net debit
    WingBreakeven = 9,        // NEW: underlying beyond outer strikes → max profit banked
}
```

**Deliberate deviation from the original spec's ordering:** spec ranked wing-target above
max-loss. When both fire on stale quotes, the risk exit must win — so MaxLossExhaustion (8)
outranks WingBreakeven (9). Relative to directional sources the 7–9 band never competes in
practice (vol spreads emit only 3, 7, 8, 9; positions emit 1–6).

Spread-level evaluation: `spread_lifecycle` collects leg-level marks into one spread signal set,
`ExitArbiter::select_winner` runs ONCE on the spread (single owner of decisions — invariant
preserved), then dispatches the winner to both legs through the existing per-position ladder.

Exit config (strategy-tier, per-family keys): `vol.iv_crush_pct` (0.30), `vol.max_loss_pct`
(0.80), `vol.roi_table` (spread-level ROI ladder, mirrors Engine-1 minimal_roi concept).
IvCrush v1 caveat: with proxy features, "IV crush" = HV-percentile collapse post-event —
weaker signal than true IV; threshold conservative + feature-flag `vol.iv_crush_enabled`
(default true, can disable without redeploy).

---

## 7. Hyperopt — decoupled evaluator, shared state machine

- Existing promotion pipeline (`strategy_versions` → `pending_promotions` → manual apply at
  daily boundary) unchanged. Engine-2 versions are just rows with `family='volatility'`.
- New `hyperopt/vol_evaluator.rs` gates (rail-tier, non-optimizable thresholds):
  **Profit Factor ≥ 1.40 AND MaxGain/MaxLoss ≥ 1.50 AND positive-period freq ≥ 50%** across
  walk-forward folds. IC-based gates explicitly NOT applied to vol families (non-directional).
- Backtest fuel: synthetic premiums (BSM in `options_recorder::bsm`, existing) for walk-forward;
  live paper fills + tape for the evidence gate — same doctrine as Engine 1 (§6 of settled
  design): ≥30 paper trades, ≥4 weeks tape, divergence ≤ ±25% before micro.
- The spec's §8 performance table (win rates, Sharpe ranges) is **rejected as deploy constants**
  — recorded as research claims only; the evaluator's empirical gates supersede them.

## 8. Config store additions (tiers per existing doctrine)

| Key | Tier | Default | Note |
|---|---|---|---|
| `vol.deployed_cap_pct` (share of 25%) | rail | derived from weights | arbiter owns |
| `vol.w_dir_low_vix` / `_mid` / `_high` | rail | 0.80/0.50/0.20 | weights |
| `vol.regime_crisis_vix`, `vol.regime_recovery_vix`, `vol.regime_complacent_vix`, `vol.crisis_slope` | rail | 30/20/12/0.10 | classifier |
| `vol.gate_min_vix`, `vol.vix_slope_rail` | rail | 20.0/0.5 | vol macro gate |
| `vol.pool` | rail | `["US.XLK","US.XLY","US.XLE"]` | ETF-only |
| `vol.top_k` | rail | 3 | scanner |
| `vol.iv_crush_pct`, `vol.iv_crush_enabled`, `vol.max_loss_pct`, `vol.roi_table` | strategy | 0.30/true/0.80/{} | exits, tunable |
| `vol.dte_min/max`, `vol.delta_band`, `vol.strangle_wing_pct` | strategy | 30/45/—/0.05 | builder |

Same D13 rules apply: queue at API, apply at daily candle boundary, never mid-exit.

## 9. Events & UI contract

New event sources (existing `engine_events` table, closed category set respected):
`options::regime` (strategy), `options::scanner` (strategy), `options::spread_opened`,
`options::spread_closed` (trade), `options::legging_abort` (alert), `options::gate`
(strategy — per-family decisions). UI: engine badge (DIR/VOL) on existing centralized events
view; no new pages in week 1. Global trade history shows spread as parent row + leg rows
(per-model/per-version attribution preserved via `strategy_version_id`).

## 10. Quota budget (tier 20, verified)

| Consumer | Unique symbols / 30d | Notes |
|---|---|---|
| Directional recorder chains | 3 | QQX/SMH/XLF, monthly roll ≈ +0–3 |
| Vol finalist chains | ≤ 3 | only after scanner binding |
| Chain discovery (daily) | snapshot-batched | 1 call per 400 codes, negligible |
| **Headroom** | **≥ 11** | emergency, rolls, new-symbol months |

Rate limit (snapshot ~60 req/30s): recorder 15-min ladder cadence (D2) unchanged; vol finalists
join the recorder rotation at the same cadence. No 15s polling anywhere.

## 11. Verification matrix (acceptance per phase)

| Surface | Test |
|---|---|
| DDL | `options_spread_tables_created_by_ddl` (4 tables + indexes, idempotent re-run) |
| Classifier | boundary values 12/20/30, exact slopes, NaN robustness, golden fixture history run |
| Gate scoping | Engine-1 regression: `evaluate(Directional, …)` == old output for a fixture sweep |
| Arbiter | pure-fn golden outputs; weights sum to 1.0; cap math |
| ExitArbiter | existing 427+ tests green unchanged; new variant priority tests |
| Lifecycle | integration: regime → scan → entry → both legs open → DTE/IV-crush exit → both legs closed → spread CLOSED, PnL matches Σ legs, intents complete |
| Engine-1 no-regression | entry→exit round-trip integration test (`389b93f`) passes byte-identical |

## 12. Non-goals (v1 / week 1–2)

Butterfly/condor; 4-leg anything; persistent ZMQ order daemon; live/micro vol tiers;
vendor data ingestion (ORATS/ThetaData); hourly cadence; single-name candidates; UI pages;
equities-flow budget consumption (interface only); true IV rank.

## 13. Open items (flagged for human decision before their phase)

1. ThetaData free-tier historical depth — verify once (30-min task) before any vendor talk.
2. Alert routing for spread CIRCUIT_BREAKER (same channel as Engine 1 — confirm).
3. When (if ever) Engine-1 sizing starts consuming arbiter splits (advisory review after 2 weeks of logs).
4. Pool expansion criteria (quota re-review mandatory before adding any symbol).
