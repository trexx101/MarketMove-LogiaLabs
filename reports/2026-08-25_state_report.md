# MarketMoves State Report — 2026-08-25 17:15 UTC

Compiled by teech. All data pulled live from the DB (copy of `deploy_data` volume
`/app/data/candles.db`), engine logs/events, parquet tape files, and the
options-recorder systemd journal.

---

## PART 1 — EQUITIES PREDICTION FLOW

### Pipeline status

HEALTHY, end-to-end. Candles current for all symbols (today 13:30 UTC candle
ingested), predictions persisting per-model for QQQ/SMH/XLF through today.
NVDA disabled (row retained, `enabled=0`) — its predictions stop at
2026-08-13 by design. All the 2026-08-25 fixes (composite `ON CONFLICT`,
symbol-aware ZMQ payload, latest-N candle fetch, per-symbol inference
bundles) are holding: 3 distinct, plausible predictions per day.

### Directional accuracy (computed over full history, ~1,100 resolved rows each)

| Symbol | 1d    | 5d    | 21d   | Note |
|--------|-------|-------|-------|------|
| QQQ    | 55.7% | 57.2% | 63.9% | best long-horizon signal |
| SMH    | 56.5% | 61.1% | 65.4% | strongest overall |
| XLF    | 54.4% | 58.6% | 56.6% | weakest; 21d barely above coin-flip |
| NVDA   | 46.9% | 47.1% | 48.1% | sub-50% at every horizon; disabling was correct |

Honest read: 1d accuracy (54–57%) is thin edge; the tradeable edge is in
5d/21d. This matches the strategy design (regime + threshold, swing holds).

### Closed trades (all history: 7 `equity_trades` rows)

**Only ONE completed round trip exists:**

- SMH LONG: buy 587.82 (2026-08-14) → sell 599.44 (2026-08-17)
- Realized: **+$10.72** on 1 share (~+1.8% net, 3-day hold)
- Rationale: bull regime (close > SMA40) + positive pred_1d crossed entry
  threshold; exited when signal flipped. The system working as designed —
  it's just tiny (see sizing concern below).

**Noise rows, not real trading:**

- XLF buy 52.23, candle 2026-06-10, created 2026-08-16 — backfill replay
  emitted a historical trade. Never closed. Should not count.
- SMH buy 587.82 created 2026-08-15 09:45 is the live entry of the round
  trip above.

### Open positions (`signal_state` + trades, as of report time)

**QQQ — LONG (QQQ)** entry 712.92 today
- Rationale: regime bull, pred_1d +0.34% > threshold, pred_5d +1.5%.
- Unrealized ≈ $0 (entered at today's close). Clean, textbook entry.

**SMH — SHORT (via SOXS)** first entry 567.12 (2026-08-18) + SECOND buy
559.01 today 16:12 — see concern #2, this is a double entry.
- Rationale: regime bear (close 559 < SMA ~575), pred_1d -0.40%.
- Tension: pred_5d is +4.2% (bullish) but regime dominates, and pred_21d
  is -9.8% (strongly bearish). The short is regime+21d consistent,
  5d-inconsistent. Not a bug — the strategy weights regime — but a
  horizon-conflict exit rule would have kept this flat.

**XLF — LONG** entry 58.11 today
- Rationale: regime bull, pred_1d +0.06%, pred_21d +0.0%. Weakest of the
  three — barely-positive signal, entered mostly on regime. (The June
  backfill row muddies its trade history; executor state itself is
  single-position so real exposure is just today's entry.)

**NVDA**: disabled, flat. No position.

### Equities concerns (ranked)

1. **Position sizing is degenerate**: qty=1.0 on every trade. $713 deployed
   against a $10,000 QQQ budget = 7%. Until sizing scales to budget, PnL
   numbers prove the logic but say nothing about dollar profitability.
2. **Double entry on restart.** Today's 16:12 engine restart re-processed
   today's candle and re-entered all three models — including a second SOXS
   buy on SMH which already had an open short from 08-18. `sync_from_db()`
   prevents state loss but did NOT prevent re-entry when a fresh candle is
   replayed post-restart. `positions` table also shows duplicate
   same-candle rows with NULL `model_id` from the bootstrap loop writing
   alongside per-model schedulers. Most important live bug right now.
3. **Backfill replays emit trades** (XLF June row). Backfill should be
   prediction-only; trade emission from historical candles pollutes history
   and PnL accounting.
4. **No inverse-symbol candles**: SOXS and FAZ have no rows in
   `equity_candles`, so short unrealized PnL is priced off the primary
   symbol's move — an approximation, and for 3x leveraged inverses a
   materially wrong one over multi-day holds. SMH short since 567.12: SMH
   fell ~1.4%, a 3x inverse should be up ~4%, but no actual SOXS quote
   exists to verify.
5. **XLF 21d accuracy (56.6%) is marginal** — weakest model of the three.
   Fine to keep, but don't weight it equally in any portfolio math.
6. **SMH's pred_21d of -9.8% is an extreme call.** If right, great; but
   predictions of that magnitude deserve a sanity watch on the de-norm path
   (`atr_ratio`/`label_std`) — it's at the edge of the historical
   distribution.

---

## PART 2 — OPTIONS FLOW

### Tape recorder

Running since 2026-08-21 (4 days), healthy, heartbeats fresh (17:05 UTC,
all three tapes). Recording Sep-25 chains (30–45 DTE, delta target 0.45),
2 contracts/underlying, 15s poll.

Files (`data/options_tape/`):

| Date | State |
|------|-------|
| 2026-08-20 | all 0 bytes (setup day, partial) |
| 2026-08-21 | valid (~3,200 QQQ rows; SMH/XLF similar) |
| 2026-08-24 | valid (flushed at today's day boundary) |
| 2026-08-25 | 0 bytes right now — normal: writer flushes at day boundary; verify tomorrow morning, treat still-0 as a bug |

Total: 304 KB across 2 full sessions.

**Data quality is GOOD where it exists.** Sample
(`US.QQQ260925C720000`, 08-24): bid 14.26 / ask 14.48 (1.5% rel spread),
delta 0.469, IV 19.4%, OI 249, volume 70. Greeks (gamma, theta) present
and sane. Liquidity filters (bid>=0.01, spread<=8%, OI>=100) are doing
their job. One 502 heartbeat at 16:12 = engine restart window; recovered.

### Hyperopt runs (nightly, `sma_regime` family, equity candles)

| Run | Started (UTC) | Equities | Stored | Promoted |
|-----|---------------|----------|--------|----------|
| 1 | 2026-08-22 02:42 | 3 | 27 | 0 |
| 2 | 2026-08-22 20:42 | 3 | 27 | 0 |
| 3 | 2026-08-23 20:44 | 3 | 27 | 0 |
| 4 | 2026-08-24 20:40 | 3 | 27 | 0 |

108 candidates total, ALL status `NEW`, ALL failing the IC gate:

| Equity | Best mean_ic | Range |
|--------|--------------|-------|
| QQQ | -0.065 | -0.065 … -0.256 |
| SMH | -0.075 | -0.075 … -0.190 |
| XLF | -0.196 | -0.196 … -0.286 |

Fold-level: 4 of 5 folds strongly negative; one fold weakly positive
(+0.037). n_trades ~1,050–1,200 per candidate (passes the trade-count
gate easily). This is SYSTEMATIC negative IC, not noise — the same open
finding from the first run, unresolved.

### What counts as a promotion candidate (current gates, `promotion.rs`)

| Transition | Gates |
|------------|-------|
| NEW → PAPER | n_trades >= 100 AND mean_ic >= +0.03 |
| PAPER → MICRO | n_trades >= 30 AND mean_ic >= +0.03 |
| MICRO → LIVE | n_trades >= 50 AND mean_ic >= +0.04 |

Sharpe and days-observed gates exist in code but are disabled (0.0) — no
live backtest/observation source feeds them yet.

Mechanics: `POST /api/hyperopt/:equity/promote/:id` never flips status
directly — it queues into `pending_promotions`, rejects if the equity has
open option positions, and applies at the next daily candle boundary.
`pending_promotions` is empty: nothing has ever been requested.

### Good vs bad — trajectory assessment

**Good:**
- Plumbing is real and verified: 4 consecutive nightly runs fired on
  schedule, stored exactly 9 candidates/equity each, zero crashes, zero
  duplicate rows (the once-per-window guard works).
- Tape data quality is high — exactly the contract shape P5.6 replay
  needs, greeks intact.
- Gates are doing their job: 108 bad candidates, 0 promotions. The system
  is refusing to promote garbage, which is the whole point.

**Bad:**
- IC is negative everywhere. Two hypotheses, one must be resolved before
  trusting anything: (a) sign inversion in the eval objective
  (`engine/src/hyperopt/eval.rs`), or (b) sma_regime momentum genuinely
  mean-reverts at the 5-day horizon for these ETFs. If (b) is real it's
  actually interesting — flip the signal and those -0.07…-0.20 ICs become
  positive. Either way, ZERO candidates are promotable until it's answered.
- Tape depth is nowhere near sufficient: 2 sessions (~6.5h each). Tape
  replay validation needs weeks of data minimum.
- Only 2 contracts per chain (quota tier 20) — the chain selector has
  almost nothing to select from.

### Options concerns (ranked)

1. **The options pipeline has NEVER actually run live.** OptionsScheduler
   is not enabled in `main.rs` ("not-yet-enabled" per the comment),
   `option_positions` / `option_fills` / `exit_signals` are all empty, and
   there are zero SKIPPED_ENTRY events. P1–P6 are built and tested; P7
   (live integration) is the gap between "code exists" and "system runs."
2. **Negative-IC mystery** (above) — the single biggest blocker for the
   whole options promotion path.
3. **Tape accumulation is a long pole**: at ~2 sessions/week pace
   (holidays, outages), meaningful replay coverage is 1–2 months out.
   Nothing to do but keep the recorder running and protect it.
4. **2026-08-25 parquet files must be non-zero by tomorrow morning** —
   check first thing; a silent writer death would burn tape days before
   anyone notices.
5. **Config store (`options_config_kv`) is EMPTY in the DB** — the 37-key
   registry defaults haven't been seeded live. Harmless while the
   scheduler is off; will matter the moment P7 turns it on.

---

## OVERALL VERDICT

**Equities:** the loop is real and working end-to-end, with
modest-but-genuine predictive edge at 5d/21d. It is not "remotely
profitable" yet for two mechanical reasons, not a signal reason: 1-share
sizing and the restart re-entry bug. Fix those two and there is a real
paper PnL curve to judge.

**Options:** infrastructure ahead of evidence. Everything up to live
integration is built and tested, but no option has ever been traded, the
tape is days old, and the candidate generator is producing systematically
negative IC. Next highest-leverage moves, in order:

1. Resolve the negative-IC sign question (cheapest, unblocks everything downstream).
2. Fix equities re-entry + sizing so the paper track record becomes meaningful.
3. Let the tape accumulate, then P7.

**Housekeeping:** branch `feature/options-momentum-engine` is 58 commits
ahead of `origin/main` and unpushed, with 4 files modified (the chart work
from today). Push once today's chart fixes are verified.
