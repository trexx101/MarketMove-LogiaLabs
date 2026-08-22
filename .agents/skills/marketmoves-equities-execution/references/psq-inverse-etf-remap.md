# PSQ Inverse-ETF Remap — Exact Fill Matrix & Map

## Fill matrix (PaperExecutor, PSQ mode)
symbols: primary = `QQQ`, short (inverse ETF) = `PSQ` (default, override via `SHORT_SYMBOL`).

| Transition        | Leg 1 (exit held)        | Leg 2 (open target)       | PnL leg that is non-zero |
|-------------------|---------------------------|---------------------------|---------------------------|
| Flat → Long       | —                         | Buy `QQQ`               | entry (0.0)               |
| Long → Flat      | Sell `QQQ`                | —                         | (close - entry)·qty - fee |
| Flat → Short     | —                         | Buy `PSQ`               | entry (0.0)               |
| Short → Flat     | Sell `PSQ`                | —                         | (entry - close)·qty - fee |
| Long → Short    | Sell `QQQ` (close long)  | Buy `PSQ` (open short)  | exit long PnL on leg 1    |
| Short → Long    | Sell `PSQ` (close short) | Buy `QQQ` (open long)   | exit short PnL on leg 1   |

Key: **every open is `TradeSide::Buy`, every close is `TradeSide::Sell`.**
Never branch `TradeSide` on `Position::Short`. Side is position-agnostic;
only the `FillResult.symbol` / `equity_trades.symbol` differs (`QQQ` vs `PSQ`).

## PnL formula (symbol-independent)
- Long exit: `(close - entry_price) * qty - fee`
- Short exit: `(entry_price - close) * qty - fee`
- Entry leg: `realized_pnl = 0.0`
- `fee = qty * close * fee_rate`

## File / function map
- `engine/src/strategy.rs`
  - `EquityStrategyParams { entry_threshold, exit_threshold, sma_window, enable_shorting:bool, short_entry_threshold:f64(<0), short_exit_threshold:f64 }`
  - `Default` impl keeps all defaults (backward compat). JSON deserialize uses `#[serde(default)]` so backtest API calls that omit new fields still parse.
  - `next_equity_position(current, input, params) -> Position`
    - Step 1: exit held position first (Long exit if pred_1d < exit_threshold; Short exit if pred_1d > short_exit_threshold → flattens even when shorting disabled).
    - Step 2: enter only from Flat. Bullish (close>sma & sma_valid) → Long. Bearish → Short only if `enable_shorting && current==Flat && pred_1d < short_entry_threshold`.
    - NEVER returns Long↔Short directly (two-step guard; executor depends on it).
- `engine/src/exec/mod.rs`
  - `FillResult { side, symbol:String, qty, price, fee, realized_pnl, ts }` — `symbol` added for PSQ attribution.
  - `ExecutorKind::Paper(PaperExecutor)` (Live variant deferred to 3.3).
- `engine/src/exec/paper.rs`
  - `new(pool, fee_rate, tx)` → delegate to `new_for_symbol(pool, fee, "QQQ", "PSQ", tx)`.
  - `new_for_symbol(pool, fee_rate, primary_symbol, short_symbol, tx)` — stores both symbols.
  - `symbol_for(pos, primary, short) -> &str` returns `short_symbol` for `Position::Short`, else `primary`.
  - `set_target_position`: builds exit leg (Sell + resolved symbol) then entry leg (Buy + resolved symbol); writes each via `db::insert_equity_trade`; publishes `TelemetryEvent::TradeFill { side, symbol, .. }`.
- `engine/src/db.rs`
  - `insert_equity_trade(pool, symbol, candle_ts, side, qty, price, fee, realized_pnl)` → `equity_trades`.
  - `fetch_recent_equity_trades(pool, symbol, limit)` → `TradeRow` (read by symbol).
  - Legacy `insert_trade` / `fetch_recent_trades` / `sum_realized_pnl` (no symbol) kept for parity tests; the equities API uses the symbol-aware helpers.
- `engine/src/config.rs`
  - `Config` gains `short_symbol: String` (default `"PSQ"`).
  - `from_env` reads `ENABLE_SHORTING` (bool, default false), `SHORT_ENTRY_THRESHOLD` (f64, must be <0, default -0.004), `SHORT_EXIT_THRESHOLD` (f64, must be > entry, default 0.001), `SHORT_SYMBOL` (default "PSQ").
- `engine/src/main.rs`
  - Executor built via `PaperExecutor::new_for_symbol(pool, cfg.paper_fee, &cfg.symbol, &cfg.short_symbol, Some(tx))` for both Paper and (fallback) Live.

## Test layout
- `engine/src/strategy.rs` `mod tests` — `equity_short_*` (entry in bearish, blocked when disabled, requires bearish regime, requires Flat, exit to Flat, holds, flattens stray short when disabled, two-step Long→Short, plus long-entry/long-hold regressions).
- `engine/src/exec/paper.rs` `mod tests` — `flat_to_short_buys_psq`, `short_to_flat_sells_psq_with_pnl`, `long_to_short_trades_qqq_then_psq`, `short_to_long_trades_psq_then_qqq`, `flat_to_long_opens_position`, `long_to_flat_closes_with_pnl`, `same_position_no_trade`. Assert BOTH `side` AND `symbol` on every fill.
- `engine/src/config.rs` `mod tests` — `shorting_enabled_via_env`, `shorting_default_is_disabled`, `short_entry_threshold_must_be_negative`, `short_exit_must_exceed_short_entry`, `invalid_enable_shorting_rejected`.
- `engine/tests/exec_parity.rs`, `engine/tests/paper_verification.rs` — integration PnL fixtures; updated to PSQ semantics (open=Buy/symbol, close=Sell/symbol) and the 3-arg `PaperExecutor::new` call signature. They were pre-existing-compile-broken (E0061, 2-arg new) and were fixed as part of the PSQ work.
