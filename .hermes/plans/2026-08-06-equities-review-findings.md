# MarketMoves Equities Stack — Audit Findings

**Date:** 2026-08-06
**Scope:** equities trading path — Rust axum engine, Svelte 4 frontend, Python inference service, ZMQ bridge, per-model PaperExecutor scheduling, live-mode gate, and the QQQ 8-feature train/serve pipeline.
**Method:** Read-only code inspection, cross-referenced against the notebook ground truth (`models/colab/EQ_Equities_Model.ipynb`), the `.py` reference helper (`training/equities_features.py`), the Rust serving path (`engine/src/features/equities_v2.rs`), and the parity fixtures. Numerical verification of the `drawdown_from_50d_high` divergence by recomputing 7 sampled fixture rows.

---

## Verdict

The **live prediction path is sound** — Rust features match the notebook (high-based drawdown), norm stats come from the notebook with sanity asserts, and the normalization pipeline is consistent. The damage is concentrated in **position visibility, the live-mode story, and quarantined parity tests**. Six confirmed findings, ranked by severity.

---

## F1 — `/api/status` is blind to the short (PSQ) leg — **HIGH, active bug**

### What's wrong

Shorts are expressed as **buys of the inverse ETF** (PSQ for QQQ), recorded in `equity_trades` under `symbol='PSQ'`. But the status endpoint derives **everything** from the primary symbol only.

### Evidence

| File | Line(s) | What |
|---|---|---|
| `engine/src/exec/paper.rs` | `symbol_for()` | `Position::Short → short_symbol` (PSQ); entry = BUY PSQ, exit = SELL PSQ |
| `engine/src/db.rs` | 1644 | `INSERT INTO equity_trades (symbol, …)` — fill recorded per-symbol |
| `engine/src/api/status.rs` | ~64 | `symbol = params.symbol.unwrap_or(&state.symbol)` — primary only |
| `engine/src/api/status.rs` | 36–46 | `derive_position`: `SELECT SUM(CASE WHEN side='buy' THEN qty ELSE -qty END) FROM equity_trades WHERE symbol = ?1` |
| `engine/src/api/status.rs` | 74 | `sum_equity_realized_pnl(pool, symbol)` — single-symbol |
| `engine/src/api/status.rs` | 76–78 | `derive_unrealized_pnl(pool, symbol, position)` — single-symbol |
| `engine/src/api/status.rs` | 80–81 | `fetch_equity_entry_trade_price(pool, symbol)` — single-symbol |
| `engine/src/api/status.rs` | 82–83 | `fetch_latest_equity_candle(pool, symbol)` — single-symbol |
| `frontend/src/views/Dashboard.svelte` | — | `fetchStatus(m.primary_symbol \|\| 'QQQ')` — polls primary only |

### Impact

During a short: the dashboard shows **position = FLAT, unrealized = 0, entry = None**. The operator's headline P&L widget is wrong exactly when risk is on. Fills are visible in `TradeHistory` (fetched with `'*'`), which makes the inconsistency worse — the ledger says short, the status panel says flat.

### Corollary

Even if position were made visible, unrealized PnL is computed from the primary's close series (QQQ) — not the held instrument (PSQ). During a short, the P&L needs the PSQ price series and the PSQ entry price. The fix must change the held-instrument resolution.

---

## F2 — `signal_state` singleton (id=1) clobbered across models — **HIGH for multi-model**

### What's wrong

The `signal_state` table is a single-row singleton (`id=1`). Every per-model scheduler writes to it after every trade. With N≥2 active models, the last writer wins — the row reflects whichever model traded last, not any specific model.

### Evidence

| File | Line(s) | What |
|---|---|---|
| `engine/src/db.rs` | 33–37 | DDL: `id INTEGER PRIMARY KEY CHECK (id = 1)` — singleton enforced at schema level |
| `engine/src/db.rs` | 1556–1572 | `save_position`: `INSERT INTO signal_state (id, position, updated_at) VALUES (1, ?, ?) ON CONFLICT(id) DO UPDATE …` |
| `engine/src/db.rs` | 1544–1546 | `load_position`: `SELECT position FROM signal_state WHERE id = 1` |
| `engine/src/scheduler.rs` | 397 | Paper branch: `db::save_position(&self.pool, new_pos.as_i64()).await` |
| `engine/src/scheduler.rs` | 463 | Live branch: `db::save_position(&self.pool, new_pos.as_i64()).await` |
| `engine/src/scheduler.rs` | 291 | Startup: `db::load_position(&self.pool).await` — every model inherits the last writer's position |
| `engine/src/main.rs` | 284 | One `EquityScheduler` spawned per `active_models` entry |

### Impact

- **Startup:** on restart, every model loads the **same** position from `signal_state` — the last position any model wrote before shutdown. QQQ may start thinking it's short because SMH was the last writer.
- **Runtime:** the `signal_state` position is meaningless in a multi-model setup. The advisor's `fetch_current_position` reads from the `positions` table (not `signal_state`), so the advisor is not directly affected — but the scheduler's own state restoration is corrupted.

---

## F3 — Train/serve skew on `drawdown_from_50d_high`, quarantined by `#[ignore]` — **MEDIUM (latent), the parity trap**

### What's wrong

The `.py` reference helper computes `drawdown_from_high` using **closes** as the rolling max (close-based), contradicting the name and diverging from the notebook (high-based) and the Rust serving (high-based). The parity fixture is stale, and the one test that could catch a regression is disabled.

### Direction (verified independently)

| Component | Basis | Evidence |
|---|---|---|
| Notebook (training ground truth) | **high** | `EQ_Equities_Model.ipynb`: `roll_max = df['high'].rolling(50).max()` |
| Rust serving | **high** | `equities_v2.rs:157` — `drawdown_from_high(&highs, &closes, 50)`; unit test `parity_1a_drawdown_uses_high_not_close` (L810) |
| `training/equities_features.py:180` | **close** ✗ | `drawdown_from_high(closes)` computes `np.max(closes[i-window:i+1])` — misnamed |
| `equity_feature_parity.json` fixture | **close** ✗ | I recomputed 7 sampled rows: matches close-basis exactly (Δ 0.0), diverges from high-basis by 2.5e-3–7.1e-3 |

### Impact

- **No live skew today:** the model was trained in the notebook (high-basis), norm stats are high-basis, and Rust serving is high-basis. The deployed model sees correct features.
- **Latent trap:** if anyone retrains with `training/equities_features.py` or regenerates the fixture from it, the close-basis `drawdown_from_50d_high` silently reintroduces train/serve skew. The only regression test that would catch it (`equity_feature_parity.rs:106`) is `#[ignore]`d.
- The 168h parity harness (`parity.rs:234-236`) only compares `log_return`, `atr_72`, `vwap_dev` — the 8-feature QQQ pipeline is **not covered** by any active test.

---

## F4 — Live-mode gate: no production marker writer, wrong instruction, wrong pipeline — **MEDIUM-HIGH**

### What's wrong

The live-mode gate (`TRADING_MODE=live` startup + runtime toggle) requires a fresh `parity_verified.json`. Three problems:

1. **No production code writes it.** The sole writer is `engine::parity::write_marker`, called only from `tests/parity_harness.rs:128` — which deliberately writes to `env::temp_dir()`, not the workspace root. No `parity_verified.json` exists at the repo root.
2. **The error message is unexecutable.** `config.rs:471` says `cargo run --bin engine --bin parity-harness --release` — `parity-harness` is not a `[[bin]]` target (only `engine` + `options_recorder` exist in `Cargo.toml`).
3. **The gate certifies the wrong pipeline.** The 168h golden fixture compares `log_return`, `atr_72`, `vwap_dev` (MMN/options features), not the 8 equities-v2 features the QQQ scheduler actually feeds the model.

### Evidence

| File | Line(s) | What |
|---|---|---|
| `engine/src/config.rs` | 460–490 | `verify_parity_marker` — startup gate |
| `engine/src/config.rs` | 293–294 | `parity_max_age_secs` default = 604800 (7 days) |
| `engine/src/api/mode.rs` | 109–124 | Runtime re-check on flip to live |
| `engine/tests/parity_harness.rs` | 128–129 | `marker_path = env::temp_dir().join("parity_marker_harness_test.json")` — deliberately not production |
| `engine/src/parity.rs` | 83–87 | `GoldenFeature`: `log_return`, `atr_72`, `vwap_dev` — MMN features only |
| `engine/src/parity.rs` | 234–236 | Comparison: only the 3 MMN features |
| `engine/Cargo.toml` | 41–47 | `[[bin]]` targets: `engine`, `options_recorder` — no `parity_harness` |

---

## F5 — "Live" mode executes paper trades; the Moomoo path is dead code — **HIGH (trust/safety)**

### What's wrong

The per-model bootstrap loop **always** constructs a `PaperExecutor`. The `MoomooExecutor` constructor exists only inside a dead-code function. Flipping to LIVE via the TOTP-gated UI does not route orders to Moomoo — it runs paper fills labeled "live." The `mode.rs` doc comment claims otherwise.

### Evidence

| File | Line(s) | What |
|---|---|---|
| `engine/src/main.rs` | 240–246 | Per-model loop: `model_executor = Arc<RwLock<build_paper_executor_for_model(…)>>` — always Paper |
| `engine/src/main.rs` | 506–519 | `build_paper_executor_for_model` → `ExecutorKind::Paper(PaperExecutor::new_for_symbol(…))` |
| `engine/src/main.rs` | 528–579 | `build_executor_for_mode` — only `MoomooExecutor::new` call site (L566) — `#[allow(dead_code)]` |
| `engine/src/main.rs` | 110 | Comment: "per-model live swap is a future story" |
| `engine/src/api/mode.rs` | 23–24 | Comment: "The executor swap (Paper → Moomoo) is driven by the scheduler reading `trading_mode` at each cycle." — **false** |
| `engine/src/scheduler.rs` | 421–455 | Live branch: dispatches to `self.executor` — still Paper, no swap code |

### Impact

- A TOTP-authorized flip to LIVE produces paper fills logged as "live equity trade executed."
- No real money moves — fails safe for capital, but the UI, audit table (`mode_switches`), and event log all assert a reality that doesn't exist.
- If someone later "wires up Moomoo" by calling `build_executor_for_mode` without fixing the per-model routing, only the legacy single-model (QQQ/short_symbol) path would work.

---

## F6 — Timestamp unit drift: engine writes seconds, frontend reads milliseconds — **MEDIUM**

### What's wrong

The engine emits Unix **seconds** everywhere. The frontend consumes them with `new Date(ts)` — which expects **milliseconds**. Result: `new Date(1_700_000_000)` = January 20, 1970.

### Evidence

| File | Line(s) | What |
|---|---|---|
| `engine/src/api/status.rs` | 92 | `from_timestamp(c.ts, 0)` — confirms wire ts is seconds |
| `engine/src/exec/moomoo.rs` | 214 | `Utc::now().timestamp()` — seconds |
| `engine/src/db.rs` | 1644 | `equity_trades.candle_ts` bound from candle ts in seconds |
| `frontend/src/lib/components/TradeHistory.svelte` | 9 | `new Date(ts)` — no ×1000 |
| `frontend/src/lib/components/CandlestickChart.svelte` | 149, 185, 253, 256, 547 | `new Date(ts)` / `new Date(c.ts)` — no ×1000 |
| `frontend/src/views/Ledger.svelte` | 48, 58 | `new Date(ts)` — no ×1000 |
| `frontend/src/lib/components/StatusPanel.svelte` | 183 | `new Date(modeInfo.last_switch_ts * 1000)` — **correct**, proving the codebase knows the wire is seconds |

### Coverage

The bug affects equity trade history, candle chart tooltips/x-axis labels, and the ledger. Options views (`OptionsTradeHistory.svelte:46`, `OptionsMonitor.svelte:33`, `OptionsPositions.svelte:48`) also use `new Date(ts)` — the options ts unit was not independently verified and should be checked before editing those files.

---


## Verified Clean

- **Rust serving features match the notebook** on all 8 QQQ features — the deployed model sees correct features.
- **Notebook norm-stats path has guardrails:** `_sanity_check_norm_stats` asserts RSI/ADX ranges; `robust_normalize` has a MAD floor against division blow-up.
- **Mode flips are TOTP-gated and audited** (`insert_mode_switch` log).
- **Per-model architecture is genuine:** per-model executor, `strategy_params_by_model` map, per-model event routing, per-model ZMQ inference.
- **The 168h parity harness runs and round-trips its marker** (test not ignored, marker write/read passes).
- **Unit test coverage for the high-basis drawdown** exists (`parity_1a_drawdown_uses_high_not_close` in `equities_v2.rs`).

---

## Summary

| # | Finding | Severity | Active? |
|---|---|---|---|
| F1 | Status endpoint blind to short (PSQ) leg | HIGH | **Yes** — dashboard misreports position during shorts |
| F2 | `signal_state` singleton clobbered across models | HIGH | **Yes** when ≥2 models are enabled |
| F3 | Drawdown close-basis in `.py` helper; parity test ignored | MEDIUM | **Latent** — deployed model is fine; trap for retraining |
| F4 | Live gate: no producer, wrong instruction, wrong pipeline | MEDIUM-HIGH | **Yes** — gate effectively unpassable; certifies wrong features |
| F5 | "Live" mode executes paper; Moomoo is dead code | HIGH | **Yes** — trust/safety: UI says live, reality is paper |
| F6 | Engine seconds → frontend ms (1970 dates) | MEDIUM | **Yes** — trade history, chart, ledger timestamps wrong |