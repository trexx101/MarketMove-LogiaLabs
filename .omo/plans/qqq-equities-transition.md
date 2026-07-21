# QQQ Equities Engine — Transition Plan from BTC/Quantitative

> **Status:** Wave 1 (data backbone) starting now.
> **Origin:** MarketMarkovNet BTC/quantitative engine (retired; V2 IC = 0 after
> 4 walk-forward retrains on Binance + Vision data, 2026-07-21).
> **Goal:** Build a daily-horizon QQQ equities engine with **realistic, durable
> edge** using the same Rust + Python inference + Moomoo OpenAPI stack.

---

## 0 · Why we are switching

**BTC 1h results (last 4 retrains):**

| Run | H1 IC | H2 IC | H3 IC | Edge? |
|-----|-------|-------|-------|-------|
| Pre-Vision (50 ep, lr 1e-3) | -0.0095 | -0.0066 | -0.0053 | no |
| Vision data, 50 ep, lr 1e-3 | +0.0210 | +0.0139 | +0.0172 | no (< 0.03) |
| Vision + header fix, 150 ep, lr 5e-4 | +0.0004 | -0.0085 | -0.0127 | no (worse) |
| Vision (final), 150 ep early-stop | +0.0004 | -0.0085 | -0.0127 | no |

Walk-forward IC ≈ 0 across multiple architectures, horizons, and feature sets.
Conclusion: BTC 1h hourly prediction has no extractable alpha in this data
regime. The 4-fold variance (Folds 2-3-4: +0.06 → -0.03 → -0.03) is regime
artifact, not signal.

**QQQ daily is structurally easier:**
- Persistent drift (4-7% annual) means trend filters alone are profitable.
- Macro features (VIX, yields) are slow-moving and predictive.
- Daily bars = 252/year vs 8,760/hourly → 35× less noise per sample.
- Moomoo OpenAPI gives free QQQ OHLCV since 1999.

---

## 1 · Target architecture

```
┌──────────────────────────────────────────────────────────────┐
│ Moomoo OpenAPI  ──┐                                          │
│ Yahoo Finance     │                                          │
│ FRED (yields/VIX) ├──►  data/equities.rs  ──►  SQLite       │
│                   │                                          │
└───────────────────┘                                          │
        │                                                      │
        ▼                                                      │
  features/equities_v2.rs                                      │
   • trend: MA slopes, ADX                                     │
   • macro: VIX regime, yield dir, DXY                         │
   • cross-asset: TLT corr, GLD corr                           │
   • microstructure: RVOL, gap, range                          │
   • constituents: NVDA/AAPL/GOOG rel momentum                 │
        │                                                      │
        ▼                                                      │
  normalize_v2  ─►  inference TCN (1d, 5d, 21d horizons)       │
                          │                                    │
                          ▼                                    │
                  strategy/equity.rs                           │
                   • SMA200 regime filter                      │
                   • Kelly sizing (0.25 frac)                  │
                   • ATR stop / swing high TP                  │
                          │                                    │
                          ▼                                    │
                  exec/moomoo.rs (paper)                       │
                          │                                    │
                          ▼                                    │
                  api.rs  +  frontend  (existing shell)        │
└──────────────────────────────────────────────────────────────┘
```

We **keep** the entire existing engine shell: axum API, vanilla JS dashboard,
Docker compose, SQLite schema, parity harness, ZMQ bridge. We **replace** the
crypto feature path, data source, and executor.

---

## 2 · Why these features (and not technical indicators alone)

QQQ edge comes from three structural sources:

1. **Trend persistence** — QQQ is in a confirmed uptrend ~70% of trading
   days since 2010. A 200d MA slope filter alone captures most of this.
2. **Macro regime** — VIX > 25, 10Y yield direction, DXY all lead QQQ by
   1-5 days. These are slow, public, reliable.
3. **Tail-risk mean reversion** — Drawdown recovery (buy -8% from 52w
   high, exit +20%) has Sharpe ~1.0 since 2000 in published studies.

The model **doesn't need to invent alpha from noise**. It needs to combine
these well-known factors with timing, sized correctly.

---

## 3 · Five Waves (8 weeks total)

### Wave A — Data backbone (Week 1) **← starting now**

- Moomoo OpenAPI client (OAuth, daily QQQ + 10 constituents + VIX)
- Yahoo Finance fallback (free, no auth)
- FRED API for DGS10, VIX (free, key optional)
- SQLite schema migration: add `equity_candles`, `macro_features`
- Rust data client at `engine/src/data/moomoo.rs`
- Backfill test: 1 year of QQQ daily, verify against Kraken-era parity harness

**Deliverable:** `GET /api/equity/data?symbol=QQQ&range=1y` returns
clean OHLCV. Cargo tests pass for equities data client.

### Wave B — Features (Week 2)

- `features/equities_v2.rs` — 8 features, all reproducible from price + macro
- `normalize_v2` — robust median/MAD scaling (same scheme as V2 crypto)
- Parity test: Rust compute vs Python reference (within 1e-6)
- Walk-forward feature importance (correlation with 5d forward return)

**Deliverable:** Features with non-zero std, IC > 0.05 in static
correlation with forward returns.

### Wave C — Models (Week 3-4)

- LightGBM baseline (fast, interpretable, well-suited to tabular daily)
- TCN (1d, 5d, 21d horizons) — reuse V2 architecture, swap features
- Walk-forward CV: 5y train / 1y test, 5 folds, embargo 5d
- Deploy gate: **mean OOS IC > 0.05** with positive OOS equity

**Deliverable:** `models/qqq_tcn_v1.pt` + `models/qqq_norm_stats.json`
that pass the deploy gate.

### Wave D — Strategy + paper trading (Week 5-6)

- `strategy/equity.rs` — regime filter (MA200) + model signal blend
- Position sizing: 0.25 fractional Kelly
- ATR-based stop, swing-high take profit
- Moomoo paper trading API integration
- Live PnL tracking via existing `/api/accuracy` (adapted for equities)

**Deliverable:** 2-week paper run with positive expectancy, max DD < 12%.

### Wave E — Live deployment (Week 7-8)

- Moomoo live trading API (after paper validation)
- Same risk gates: max position, daily loss limit
- Dashboard additions: equity curve, drawdown chart, regime badge
- Runbook for model retrain cadence (quarterly)

**Deliverable:** Live trading on QQQ, paper → live switch documented.

---

## 4 · Data flow contract

### `equity_candles` table
```sql
CREATE TABLE equity_candles (
    symbol      TEXT NOT NULL,
    ts          INTEGER NOT NULL,  -- unix seconds, midnight UTC
    open REAL NOT NULL,
    high REAL NOT NULL,
    low REAL NOT NULL,
    close REAL NOT NULL,
    volume INTEGER NOT NULL,
    PRIMARY KEY (symbol, ts)
);
```

### `macro_features` table
```sql
CREATE TABLE macro_features (
    ts INTEGER PRIMARY KEY,
    vix_close REAL,        -- VIX daily close
    tlt_close REAL,        -- 20+ year Treasury ETF
    gld_close REAL,        -- Gold ETF
    dxy_close REAL,        -- Dollar index ETF (UUP)
    ust_10y REAL           -- 10Y yield from FRED
);
```

### `/api/equity/*` endpoints (extending existing axum)
- `GET /api/equity/data?symbol=QQQ&range=1y` — raw OHLCV
- `GET /api/equity/features?symbol=QQQ` — current feature vector (8 dim)
- `GET /api/equity/predictions?symbol=QQQ` — 1d/5d/21d predictions + regime
- `GET /api/equity/pnl` — paper PnL curve

### Python inference contract
```python
# inference/equity_model.py
def predict(features: np.ndarray) -> dict[str, float]:
    return {"pred_1d": 0.012, "pred_5d": 0.034, "pred_21d": 0.081}
```

---

## 5 · Reuse from existing engine

| Component | Reuse? | Notes |
|-----------|--------|-------|
| `axum` API shell | yes | add `/api/equity/*` routes |
| Vanilla JS dashboard | yes | add equities views |
| SQLite + migrations | yes | new tables, no schema break |
| ZMQ bridge (`bridge.rs`) | yes | swap payload schema for 3 horizons |
| Parity harness | yes | adapt for equities |
| Docker compose | yes | add equities env vars |
| Cargo workspace | yes | add `equities` feature flag |
| Kraken/Binance data | no | retire cleanly |
| Crypto feature pipeline | no | replace with equities features |
| `tcn.py` inference | yes | swap model checkpoint |
| `paper.rs` executor | yes | extend for equities (qty × price) |
| `strategy.rs` (SMA200) | yes | core logic stays, sizing changes |

---

## 6 · Hard constraints

- **No live trading** until paper validation passes (2 weeks positive expectancy)
- **No leverage** in Wave D/E (cash equity only, 1x)
- **Max position**: 95% of portfolio, single-name
- **Daily loss limit**: 2% triggers halt
- **Retrain cadence**: quarterly, walk-forward validated
- **Backtest realism**: 0.05% slippage + 0.01% commission per trade
- **Walk-forward IC gate**: mean > 0.05 across all horizons (vs 0.03 for crypto)
- **Stop loss**: 2× ATR(14) from entry
- **Take profit**: 4× ATR or swing high (whichever first)

---

## 7 · Risk factors

1. **Regime change** — 2022 bear market. Backtest must include it.
2. **Concentration risk** — QQQ top-10 holdings drive most variance. Avoid
   trading individual constituents (only QQQ itself).
3. **Slippage** — Paper uses mid-price. Live will have spread cost. Use limit
   orders with 1-tick slippage tolerance.
4. **Data quality** — Moomoo occasionally has split-adjustment bugs. Yahoo
   fallback for cross-checks.
5. **Moomoo API limits** — 60 calls/min. Batch requests, cache daily closes.

---

## 8 · File-by-file migration plan (Wave A)

### New files
```
engine/src/data/moomoo.rs            # Moomoo OpenAPI client
engine/src/data/yahoo.rs             # Yahoo Finance fallback
engine/src/data/fred.rs              # FRED macro fetcher
engine/migrations/2026_07_22_equities.sql
engine/tests/moomoo_data.rs         # data client parity test
```

### Modified files
```
engine/src/data/mod.rs               # add moomoo/yahoo/fred modules
engine/src/config.rs                 # add MoomooConfig, FRED_API_KEY
engine/src/main.rs                   # spawn equities data task
engine/src/scheduler.rs              # add equities ingestion cadence (daily)
engine/Cargo.toml                    # add reqwest, serde_json, chrono
```

### Retired (deferred, not deleted in Wave A)
- `engine/src/data/binance.rs` (dormant; keep for reference)
- `engine/src/features/crypto.rs`
- `engine/src/features/legacy.rs`
- `training/fetch_features.py`
- `training/train_tcn.py`
- `training/labels.py`

These get cleaned up in Wave B once equities features are validated.

---

## 9 · Verification plan (per wave)

- **Wave A**: `cargo test --release -p engine --lib data::` passes;
  backfill test loads 1y QQQ data within 30s; `/api/equity/data?symbol=QQQ&range=1y`
  returns valid JSON with 250+ candles.
- **Wave B**: Parity test green; feature IC report shows top 3 features
  with |IC| > 0.05 against 5d forward return.
- **Wave C**: Walk-forward IC > 0.05 on at least 4/5 folds; mean equity > 0;
  model passes deploy gate.
- **Wave D**: 14-day paper run; max DD < 12%; net positive PnL; no halts.
- **Wave E**: Live trading with 1x leverage; first 2 weeks under same gates.

---

## 10 · Decision log

| Date | Decision | Why |
|------|----------|-----|
| 2026-07-22 | Switch from BTC 1h to QQQ daily | IC=0 across 4 retrains; structural noise floor |
| 2026-07-22 | Keep Rust + Python inference shell | Works, tested, deployed; 80% reusable |
| 2026-07-22 | Use Moomoo OpenAPI for data | User has account; free; deep history |
| 2026-07-22 | Start Wave A immediately | Data is the foundation; everything blocks on it |
| 2026-07-22 | Walk-forward IC gate 0.05 (vs 0.03) | Daily is cleaner; tighter gate is honest |
| 2026-07-22 | No leverage in paper/live | Volatility + drawdown control |
| 2026-07-22 | Quarterly retrain | Avoids overfit churn, aligns with seasonality |

---

## 11 · Next milestones

- **2026-07-22 end**: Wave A code complete, `/api/equity/data` working
- **2026-07-25**: Wave B features complete, IC report
- **2026-07-30**: Wave C model passing deploy gate
- **2026-08-05**: Wave D paper trading live
- **2026-08-12**: Wave E live trading (conditional on Wave D green)

---

**This plan replaces the BTC/quantitative engine scaffold. Implementation
starts immediately with Wave A: Moomoo OpenAPI client + SQLite schema +
backfill test.**
