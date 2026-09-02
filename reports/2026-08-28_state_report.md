System Status Report — 2026-08-27 16:45 UTC
TL;DR: The bug fixes held — the prediction pipeline is healthy across all three models and hyperopt's negative-IC problem is genuinely fixed, with the first promotion ever (QQQ → PAPER) applied yesterday. But the paper-trading accounting layer is desynced from reality after the 08-26 restart: today's XLF buy is a duplicate entry, and the SMH short "closed" this morning without any recorded exit fill. Signals work; bookkeeping doesn't.

Part 1 — Equities
Pipeline health (post bug-fixes): ✅ verified working
Engine healthy 28h, inference healthy 2d, no scheduler init errors, no prediction-insert errors.
All 3 models persisted today's predictions (QQQ 14:45, SMH 13:38, XLF 16:09 UTC). The 08-25 fixes (composite ON CONFLICT, symbol-aware ZMQ, latest-N candles) have now survived 2 full daily cycles.
One artifact: actuals: updated predictions updated=0 two hours running — benign (no predictions have matured into resolution window yet today), but worth knowing it's not doing anything.
Prediction quality (full-history directional accuracy, n≈1,100 each)
Symbol	1d	5d	21d	vs 08-25 baseline
QQQ	55.8%	57.2%	64.0%	unchanged
SMH	56.5%	61.1%	65.5%	unchanged
XLF	54.3%	58.7%	56.7%	unchanged
NVDA	46.9%	47.1%	48.1%	disabled — correct
Edge profile unchanged: thin 1d, tradeable 5d/21d. Today's latest predictions:

Symbol	regime	pred_1d	pred_5d	pred_21d	read
QQQ	bull (717.63, SMA 571→ far above)	−0.00035	+0.0001	−0.00009	model has zero conviction — all horizons ~0 for 2 days
SMH	bear (566.74 < SMA 571.18)	+0.0108	+0.0299	−0.0840	horizon split: 5d bounce vs violently bearish 21d
XLF	bull (58.03 > SMA 57.00)	+0.0042	+0.0117	+0.0155	cleanest alignment in the book — all horizons agree
⚠️ QQQ's collapse to ~0 across all horizons since 08-25 (was +0.0032/+0.0159) is either genuine neutrality or an early buffer/cold-start symptom. Tomorrow's row will disambiguate.

Closed trades (with rationale)
Exactly one real round trip ever: SMH long, +$10.72 (buy 587.82 08-14 → sell 599.44 08-17, regime bull, pred-driven exit). Everything else on the ledger is noise or unclosed:
Row 3 (XLF buy, candle_ts 2026-06-10, created 08-16) = backfill replay noise — created days after its candle. Not trading.
Rows 6/7/8 (XLF/QQQ/SOXS buys 08-25 16:12, blank model_id) = the documented restart re-entry bug firing.
Total realized PnL: +$10.72. That's the entire economic output of the system to date — with qty=1 on $5–10k budgets, PnL is structurally meaningless until sizing is fixed.

Open positions — rationale and likelihood
Position (per signal_state)	Entry	Mark	Unrealized	Decision quality	Likelihood of profitable close
QQQ long (since 08-25)	712.92	717.63	+$4.71 (+0.66%)	Entry was regime-consistent (bull, pred_1d +0.0032). But current preds are ~0 — the model no longer endorses direction, the position rides regime alone.	Moderate. Bull regime + 57–64% 5d/21d accuracy favors it, but with zero conviction the exit will be regime/threshold-driven, not profit-targeted. Real risk of round-tripping the gain.
XLF long (dup entries 08-25 @58.11 + today @58.03)	58.03 (executor view)	58.03	flat	Best decision quality in the book: all-horizon bullish agreement + bull regime + XLF is also the strongest hyperopt equity.	Slightly favorable — but the executor only knows about today's lot; the DB thinks there are two. Accounting of the close will be wrong either way.
SMH	flat since 13:38 today	—	—	Exit signal fired correctly (pred_1d flipped positive in bear regime → cover).	n/a — but see concerning #2: the close was never executed on the books.
Can they close at all? At the signal level, yes — schedulers evaluate and transition correctly every day. At the accounting level, currently no: the executor's in-memory book is out of sync with signal_state since the 08-26 restart (details below), so exits will fire as state transitions without producing fills or PnL.

Part 2 — Options
Tape: healthy infrastructure, drifting spec
Recorder up 6 days, heartbeats 7–8 seconds fresh on all 3 tapes, quota tier-20 respected (2 contracts × 3 underlyings = 6 of 20), clean idle behavior outside market hours.
~4 full days of tape per underlying (08-21, 24, 25, 26), 60–70KB/underlying/day. Today's files 0-byte = normal day-boundary flush.
Spec drift: all three tapes are pinned to 260925 expiry = 29 DTE as of today — below your 30–45 window. The roll never fired: chain selections live in an in-memory dict that only re-scans when empty, and it's been populated since 08-21. The pre-open DTE check I expected at the scan gate isn't actually gating on DTE. A recorder restart at next pre-open forces a re-scan onto October expiries.
Watch item: QQQ's session quote counter reads 6 vs XLF's 216 — QQQ may be under-recording today. Verify tomorrow's QQQ parquet size.
Hyperopt: the bug fix is confirmed working
6 runs, all completed, once-per-window guard intact (no duplicates):
Runs	Grid	Result
1–4 (08-22→24)	9 configs, no direction	all negative IC — the pre-fix pathology
5 (08-25), 6 (08-26)	18 configs with direction	30 candidates each; every dir=−1 config positive, every dir=+1 negative. Ranking reproduced both nights: XLF 0.296 > QQQ 0.246 > SMH 0.179. Threshold axis inert (<0.002 ΔIC) as predicted.
First-ever promotion: vQQQ_1787690945_0 (QQQ, sma_window=200, thr=0, dir=−1, mean_ic 0.245, folds +0.20/+0.26/+0.25/+0.19/+0.26 — all positive) passed the fold-consistency gate, was queued 08-26 01:37, and applied at the candle boundary 08-26 12:18 → status PAPER. The D13 timing gate worked exactly as designed.

Honest caveats that survive: fold t-stats are overstated (signal autocorr 0.99 → ESS ≈ 5–6), XLF's fold-2 (+0.49) leans on very few SMA-200 flips, and PAPER currently has no execution sink — OptionsScheduler is explicitly not wired in main.rs, options_config_kv is empty, option_positions/fills/exit_signals all zero. The promotion is real; the stage it promotes into is a stub. XLF and SMH winners also passed all gates but were never queued — only QQQ went through.

The Good
Prediction pipeline fully healed — 3 models, fresh daily predictions, accuracy stable, zero errors for 2 cycles.
NEG-IC fixed for real — direction axis flipped the sign, two consecutive runs reproduce the same ranking, fold-consistency gate working.
First promotion ever landed cleanly through the full queue→boundary→apply path with persisted evidence.
Tape infra is solid: heartbeats fresh, quota respected, clean idle/flush semantics.
Today's XLF entry is the best-justified trade the system has taken: full horizon agreement + regime + your strongest hyperopt equity.
The Bad
qty=1 sizing — $10.72 lifetime realized PnL on $20k of budgets. Economics can't be evaluated until this scales.
XLF/SMH hyperopt winners sit unqueued — pipeline proven on QQQ; the other two need a deliberate queue decision (or auto-pilot needs to queue best-per-equity).
PAPER stage accumulates no evidence — no options executor, and min_sharpe/min_days pinned to 0 for exactly that reason. MICRO/LIVE gates are unreachable today.
SOXS/FAZ candles never ingested (only PSQ exists, stale since 08-25) — short positions can never be marked to market.
Tape is 29 DTE, outside the 30–45 spec, until a restart/re-scan.
The Concerning (ranked)
Executor restart state-loss is NOT fixed — the ops skill claims sync_from_db() ships; it does not exist anywhere in engine/src/. Consequences, all observed today: - Duplicate XLF entry (08-25 lot + today's lot, executor only tracks one, entry price overwritten). - QQQ long is a ghost — scheduler says long since 08-25, executor (reborn flat 08-26) holds nothing. Its future exit will sell nothing. - SMH short vanished: flipped short→flat 13:38 today with no sell fill, no realized PnL — two orphan SOXS entries (567.12, 559.01) on the books, SMH rallied +2.1% today, so that exit is almost certainly an unrecorded loss the system cannot quantify.
insert_equity_trade has no model_id parameter — every fill since 08-25 has blank model_id (4 rows incl. today's XLF buy), making bootstrap re-entries indistinguishable from legitimate entries in audits. Telemetry stamps model_id; persistence drops it.
positions rows also carry blank symbol/model_id — same attribution hole.
Your two questions
Do I see signs of a live candidate?
Signal-level: yes, one. QQQ w=200/thr=0/dir=−1 cleared every gate with consistent folds and is now PAPER; XLF's 0.296 is stronger on paper but rests on thinner fold evidence. Execution-level: no — there is no options execution path for a candidate to trade through, so nothing can earn the PAPER→MICRO evidence. The honest blocker list to a live candidate is: wire OptionsScheduler → externalize options config → fix executor sync + trade attribution. In that order.

Should you scan more chains into the tape?
Not yet. Three reasons: (1) fix the roll first — otherwise new selections drift out of the 30–45 window the same way; (2) the tape is only 4 days deep — depth beats breadth right now, and nothing consumes the tape yet; (3) when you do widen, you have headroom (6 of 20 quota used — room to ~double, e.g. a second expiry month per underlying for term-structure features). My recommendation: restart the recorder at next pre-open to roll onto October chains, let it accumulate 3–4 weeks, then widen.

Next actions I'd prioritize
sync_from_db on the executor (+ idempotent entry per (model_id, candle_ts)) — this is corrupting the paper record daily.
Add model_id to insert_equity_trade; backfill the 4 blank rows from telemetry timestamps (they're recoverable).
Decide: queue XLF/SMH winners to PAPER, or extend auto-pilot to queue best-per-equity.
Restart recorder pre-open to fix DTE drift.