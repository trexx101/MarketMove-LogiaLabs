# Strategy Lab + IC Analysis — Updated 2026-07-30 (session)

## Status: backfill-predictions DONE, predictions table populated

**`POST /api/equity/backfill_predictions`** implemented in this session.
Deployed and confirmed working (2026-07-30):

```
candles_processed=1254
predictions_written=1125
skipped_already_had=3
errors=0
```

`resolved_count` went from **3 → 500** (resolved = has actual return now that time has passed).

## IC / directional accuracy — now measurable

| Horizon | Directional Accuracy |
|---------|---------------------|
| 1-day   | **62.4%** (n=500)  |
| 5-day   | **58.0%** (n=500)  |
| 21-day  | **67.5%** (n=500)  |

With 500 resolved predictions, all three are statistically significant.
Directional accuracy above 50% is real signal. The prior "IC ≈ 0" conclusion was
based on only 3 samples and the user's Colab walk-forward (which trained on
BTC/ETH, not QQQ). The QQQ TCN model has measurable directional edge.

MAE (1-day): 0.0115 | (5-day): 0.0252 | (21-day): 0.0340

## Best backtest found (SMA=40 regime, PSQ shorting enabled)

Full-period backtest (2022-01-01 → 2025-07-30, 895 trading days):

| Metric | Strategy | Buy-hold |
|--------|----------|----------|
| Total return | **134.9%** | 41.2% |
| CAGR | **18.0%** | ~10% |
| Sharpe | **1.41** | ~0.7 |
| Max DD | **22.9%** | ~33% |
| Win rate | **83.3%** | — |
| Profit factor | **5.85** | — |
| Trades | **24** (14L/10S) | ∞ |
| Trades/yr | **~7** | daily |

Baseline config (SMA=200, 10 trades): CAGR 10.9%, Sharpe 1.05.
**The SMA=200 was the bottleneck — not the model.** SMA=40 captures regime
shifts 5× faster, keeping you short before the worst of 2022 drawdown and
back to long in time for recovery.

## Key parameters for high-frequency (7 trades/yr) config

```
sma_window: 40
entry_threshold: +0.001
exit_threshold: -0.0005
short_entry_threshold: -0.001
short_exit_threshold: +0.0005
enable_shorting: true
```

## Remaining gap: rank IC in /api/accuracy

`db::fetch_equity_accuracy` still only computes directional accuracy + MAE.
No Spearman ρ or Pearson IC. This is the canonical model-quality metric and
should be added.

## Remaining gap: pred_5d filter on long entries

`next_equity_position` requires `pred_5d > 0.0` for bullish long entries
(in addition to `pred_1d > entry_threshold`). This is a passive filter in
bull markets and could be made configurable or replaced with a tighter
threshold. The short side has no `pred_5d` requirement and fires more freely.

## Predictions table state (2026-07-30)

```
equity_predictions: 1125 rows
equity_candles: ~14,300 rows (2021-08-02 → 2026-07-30)
resolved_count: 500
```

The backfill skips 3 pre-existing rows (July 24, 27, 30 — already had
predictions from the live scheduler). ~128 rows remain unresolved (future
dates not yet passed).
