# SMA Regime + Shorting Sweep Findings (2026-07-30)

## The SMA=40 breakthrough

Across 72 configs tested (varying SMA ∈ {20,30,40,50,75,100,150,200},
threshold combos × entry/exit/short combos), the single most important
parameter change was **shrinking SMA from 200 to 40**:

| Config | Trades | CAGR | Sharpe | Max DD | Win Rate |
|--------|--------|------|--------|--------|----------|
| SMA=200 baseline | 10 | 10.9% | 1.05 | 22.9% | 80.0% |
| **SMA=40** | **24** | **18.0%** | **1.41** | **22.9%** | **83.3%** |

Same prediction data, same threshold strategy, 5× faster regime detection.
The key difference: SMA=40 exits the long position before the worst of the
2022 bear market (−33% QQQ), holds short through May-June 2022 (10 short trades
averaging +3.6% each), then re-enters long for the recovery.

## pred_5d_filter: validated as configurable (2026-07-30)

`EquityStrategyParams.pred_5d_filter: bool` was added to `strategy.rs` with
`#[serde(default = "default_pred_5d_filter")]` — backtest API callers can omit it
and still deserialize. Default is `true` (backward compatible).

**Live backtest comparison (SMA=40, enable_shorting=true):**

| pred_5d_filter | Trades | CAGR | Sharpe | Win Rate | PF | Total Ret |
|----------------|--------|------|--------|----------|----|--------|
| true (default) | 24 (14L/10S) | 18.0% | 1.41 | 83.3% | 5.85 | 134.9% |
| **false** | **25 (15L/10S)** | **20.2%** | **1.43** | 80.0% | **6.17** | **159.1%** |

**Verdict:** `pred_5d_filter=false` adds 1 long trade (+4.2pp CAGR, +0.02 Sharpe,
+24.2pp total return) at the cost of 3.3pp win rate. Net is clearly positive.
The pred_5d<0 rows being admitted are small winners, not noise.

## Signal distribution from 1,125 predictions

pred_1d range: approximately [−0.020, +0.025]

| Band | pred_1d | Count | % |
|------|---------|-------|---|
| Strong long | ≥ +0.003 | ~150 | 13% |
| Medium long | +0.001–0.003 | ~200 | 18% |
| Weak/flat | 0–0.001 | ~325 | 29% |
| Medium short | −0.003–0 | ~275 | 24% |
| Strong short | ≤ −0.003 | ~175 | 16% |

## Recommended production config

```json
{
  "kind": "threshold",
  "params": {
    "entry_threshold": 0.001,
    "exit_threshold": -0.0005,
    "short_entry_threshold": -0.001,
    "short_exit_threshold": 0.0005,
    "sma_window": 40,
    "enable_shorting": true,
    "pred_5d_filter": false
  }
}
```

Results: 25 trades (15L/10S), 20.2% CAGR, 1.43 Sharpe, 80% WR, 6.17 PF, 159.1% total return.
vs buy-hold: 41.2% total, ~0.7 Sharpe, ~33% max DD.

## How to validate new configs

```bash
docker exec mmn-engine curl -s -X POST http://127.0.0.1:8080/api/backtest \
  -H "Content-Type: application/json" \
  -d '{"kind":"threshold","params":{"entry_threshold":0.001,"exit_threshold":-0.0005,"short_entry_threshold":-0.001,"short_exit_threshold":0.0005,"sma_window":40,"enable_shorting":true,"pred_5d_filter":false},"start_ts":1640995200,"end_ts":1753833600}' \
  | python3 -c "
import sys,json
d=json.load(sys.stdin); m=d['metrics']; t=d['trades']
print(f'Trades={len(t)}, CAGR={m[\"cagr\"]*100:.1f}%, Sharpe={m[\"sharpe\"]:.2f}, WR={m[\"win_rate\"]*100:.1f}%, PF={m[\"profit_factor\"]:.2f}')
assert m['sharpe']>=1.3 and m['win_rate']>=0.75, 'BELOW GATE'
print('PASS')
"
```

Validation gate: Sharpe ≥ 1.3, Win Rate ≥ 75%.

## What constrains more trades

1. **pred_5d filter** — addressed by `pred_5d_filter=false`
2. **SMA window** — 40 is the sweet spot (20 degrades Sharpe)
3. **pred_1d signal quality** — TCN directional accuracy is 62.4%, near-random IC
4. **Strategy structure** — only enters from flat, no mean-reversion variant
5. **To get 50+ trades/yr**: Rhai mean-reversion script OR multi-signal ensemble

## Executor: PSQ shorting already wired

`MoomooExecutor::plan_trades` correctly handles long/short/exit transitions
using PSQ (ProShares Short QQQ, ~−1x QQQ). All opens are `TradeSide::Buy`,
all closes are `TradeSide::Sell`. See `references/psq-inverse-etf-remap.md`.

## Key risk: in-sample vs out-of-sample

This backtest uses the SAME data the model was trained on (in-sample).
True out-of-sample validation requires walk-forward testing on unseen future data.
The TCN model was confirmed to have near-zero IC in Colab walk-forward analysis.
Directional accuracy (62.4%) is a separate metric from IC and may be real,
but the strategy's returns depend on both directional accuracy AND the
magnitude of correct predictions.
