# Options Momentum Engine — Settled Design (grill session 2026-08-16)

**Canonical source:** `.hermes/plans/options-momentum-engine/PLAN.md` on branch
`feature/options-momentum-engine` (commit 2d9712d). That file holds all 21
locked decisions (D1–D21), phase breakdown P0–P8, and acceptance criteria.
This reference is the quick-recall summary; read the plan before any
implementation work.

## What it is

Freqtrade-inspired medium-frequency options module added to the existing Rust
engine + SvelteKit UI + Moomoo OpenD stack. Long single-leg options only
(30–45 DTE, ~0.45 delta), underlying-price-driven exits, nightly batch
hyperopt with evidence-gated promotion, "Strategy Auto-Pilot" UI. Universe:
QQQ, SMH, XLF. Paper first, tiered to live (paper → 1-contract micro → full).

## Architecture invariants (do not violate)

1. **Strategy layer decides entries only; never places orders.** All exits go
   through a single `ExitArbiter` with fixed priority:
   force-close > circuit breaker > hardcoded overrides > trailing stop >
   minimal_roi > signal reversal.
2. **Risk layer is outside the strategy version lifecycle.** Macro gate
   (VIX level+slope, FOMC/CPI/NFP calendar blackout) and hardcoded overrides
   (DTE<7 exit, delta-drift band [0.15,0.70] exit, earnings entry blackout)
   are in code, non-optimizable, apply to all strategy versions. Hot-swaps
   can never disable them.
3. **Staged exit ladder, never raw market orders:** fresh get_option_quote →
   `BID + k×tick` (3s) → `BID` (3s) → `BID − max_slippage` (10s). Slippage
   budget = multiplier of entry-time spread capped at % of premium. Stage-3
   failure → CIRCUIT_BREAKER status, residual stays open, entry halt +
   cooldown, operator alert. Force-close skips to deep-limit immediately.
4. **Reconciliation doctrine (B + write-ahead):** DB owns strategy intent,
   broker owns facts; every stage transition persisted pre-send; startup gate
   diffs both; exits auto-resume, entries never, mismatches quarantine
   (RECONCILING). Same bug class as the equity Flat-state bug (commit
   21e6d78), worse blast radius.
5. **Sunset hot-swap model:** positions bound to `strategy_version_id`
   forever; dethroned champions manage residuals to natural death (~45d);
   promotion requires operator button, only at daily candle boundary, never
   mid-exit-ladder. Per-version P&L attribution in trade history.
6. **Backtest fuel = phased hybrid:** synthetic premiums (BSM, IV = realized
   vol × 1.1) optimize; live tape recorder validates; promotion evidence gate
   = ≥30 paper trades AND ≥4 weeks tape AND synthetic-vs-paper divergence
   ≤ ±25%, then micro size, full size after 60 clean live trades. (Inah
   negotiated the calendar gate down to this evidence gate — do not revert
   to a 3-month calendar requirement.)
7. **Chain selection:** monthly expiry preferred, min |delta−0.45| among
   liquidity-passing candidates, hard CONFIGURABLE floors (bid>0, spread ≤8%
   mid, OI ≥100), no candidate → SKIPPED_ENTRY event, no rolls in v1.
8. **Sizing:** `contracts = floor(equity × risk% / (stop_distance × delta ×
   100))` capped by debit-% slider, 10% of OI, 1 position/underlying (3 max),
   25% deployed premium. Sliders map to risk %, never contract count.

## Cadence

Daily-candle entries (process_only_new_candles), tick-driven exits on
underlying price. Hourly entries = parked v2 item.

## Open items at plan time

- Verify account option quota tier in OpenD (assumed 60) — blocks Phase 0.
- Alert channel choice for circuit breaker (phone push vs Slack).
- Per-ticker earnings calendar source.
- v2 parking lot: hourly entries, rolling, FRED yield spread, vendor data.

## Phase order

P0 prerequisites/quota verify → P1 tape recorder (calendar-bound, start
early) ‖ P2 synthetic backtester → P3 ExitArbiter/ladder/reconciliation
(paper first) → P4 entries/chain/sizing → P5 hyperopt+promotion → P6 UI
Auto-Pilot → P7 4-week paper campaign → P8 tiered live.
