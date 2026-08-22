# PSQ Inverse ETF PnL Calculation

## Background

PSQ (ProShares Short QQQ) is an inverse ETF that targets -1x the daily return of QQQ.
It does NOT track QQQ with a fixed ratio — it tracks the INVERSE daily return.

## The Bug (fixed 2026-08-03)

The original PnL formula treated PSQ as tracking QQQ **positively**:

```rust
// WRONG — assumes PSQ ∝ QQQ (positive correlation)
let ratio = psq_ep / qqq_ep;
let psq_current = lc * ratio;
Some((psq_ep - psq_current) * 1.0)
```

This inverted the sign: when QQQ rose, PSQ should fall (inverse), but the formula
showed PSQ rising — giving a negative PnL when the short should be profitable.

## The Fix

PSQ is an inverse ETF, so:

```
PSQ_return ≈ -(QQQ_return)
QQQ_return = QQQ_current / QQQ_entry - 1
PSQ_current ≈ PSQ_entry * (1 + PSQ_return) = PSQ_entry * (2 - QQQ_current/QQQ_entry)
PnL = PSQ_entry - PSQ_current = PSQ_entry * (QQQ_current / QQQ_entry - 1)
```

In Rust:

```rust
// CORRECT — PSQ inverse ETF formula
Some(psq_ep * (lc / qqq_ep - 1.0))
```

Where:
- `psq_ep` = PSQ entry price (from `equity_trades` WHERE symbol='PSQ')
- `qqq_ep` = QQQ close at the time of PSQ entry (from `equity_candles` at `entry_ts`)
- `lc` = latest QQQ close (from `equity_candles`)

## Sign Semantics

| Market Move | QQQ | PSQ | PnL Sign | Meaning |
|---|---|---|---|---|
| QQQ rose | ↑ | ↓ | **Positive** | Short loses |
| QQQ fell | ↓ | ↑ | **Negative** | Short wins |
| QQQ flat | → | → | ~0 | Neutral |

## Implementation Location

`engine/src/api/status.rs` — `handle_status()`, Short branch:

```rust
strategy::Position::Short => {
    let psq_entry = db::fetch_equity_entry_trade_price(pool, &state.short_symbol).await?;
    let entry_ts = db::fetch_equity_entry_trade_ts(pool, &state.short_symbol).await?;
    let qqq_at_entry = db::fetch_equity_close_at_ts(pool, &state.symbol, entry_ts).await?;
    // PSQ inverse ETF: PnL ≈ entry * (QQQ_now / QQQ_at_entry - 1)
    let unrealized = psq_ep * (lc / qqq_ep - 1.0);
}
```

## Required DB Functions

- `fetch_equity_entry_trade_price(pool, symbol)` — `SELECT price FROM equity_trades WHERE symbol=?1 AND side='buy' ORDER BY id DESC LIMIT 1`
  - The `side='buy'` filter is critical: without it, a partial close (sell)
    would shadow the entry and return the exit price as "entry".
- `fetch_equity_entry_trade_ts(pool, symbol)` — `SELECT candle_ts FROM equity_trades WHERE symbol=?1 AND side='buy' ORDER BY id DESC LIMIT 1`
- `fetch_equity_close_at_ts(pool, symbol, ts)` — `SELECT close FROM equity_candles WHERE symbol=?1 AND ts <= ?2 ORDER BY ts DESC LIMIT 1`
  - Uses floor match (`ts <=`) not exact match (`ts =`). A trade's `candle_ts`
    may not exactly match a candle row timestamp; floor match finds the closest
    prior candle instead of returning None.

All in `engine/src/db.rs` alongside the existing `fetch_equity_*` family.

## Caveats

- PSQ is -1x **daily** — tracking error accumulates over multi-day holds due to
  compounding. The formula is a first-order approximation.
- PSQ's actual price also includes management fees, bid/ask spread, and ETF
  premium/discount — these are not modeled.
- For paper trading, this approximation is sufficient. In live trading, the
  actual exit price will be determined by the market, not by this formula.