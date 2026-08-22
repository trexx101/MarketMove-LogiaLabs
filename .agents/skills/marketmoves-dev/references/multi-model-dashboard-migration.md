# Multi-Model Dashboard Migration Guide

When the engine supports more than one model (e.g. `qqq-v1` and `nvda-v1`),
the frontend must switch from **flat singleton stores** to **per-model
slices**. This guide covers the recurring bugs and the migration pattern.

## Store architecture

```text
stores.js
  activeModelId    <- selected model_id (from /api/models)
  models           <- list of registered models
  _slices          <- { [model_id]: { status, predictions, features,
                                       trades, accuracy, chartData } }
  legacy proxies   <- status, predictions, features, trades, accuracy,
                      chartData (derive from active model slice)
```

Components should subscribe to the per-model slice or the legacy proxies.
The legacy proxies already handle switching, so most work is making sure
**initial data is fetched per-model** and **sub-components are reactive to
`activeModelId`**.

## Common bugs after adding the model selector

### 1. StatusPanel symbol does not change

**Cause:** `Dashboard.svelte` calls the flat `/api/status` endpoint without
a model identifier. The first model's status is loaded into every slice, or the
wrong model's data is shown.

**Fix (frontend):** pass the active model's `model_id` and
`primary_symbol` to `fetchStatus`, and react to `activeModelId`:

```javascript
// api.js — optional per-symbol / per-model query params
export async function fetchStatus(modelId = null, symbol = null) {
  const params = new URLSearchParams();
  if (modelId) params.append('model_id', modelId);
  if (symbol) params.append('symbol', symbol);
  const qs = params.toString();
  const res = await fetch(`${API_BASE}/status${qs ? '?' + qs : ''}`);
  // ...
}

// Dashboard.svelte
const s = await fetchStatus(modelId, primarySymbol);
setSlice(modelId, 'status', s);
```

**Fix (backend):** `/api/status` accepts `?model_id=` and/or `?symbol=` and
resolves the model's primary/inverse symbols from the `trading_models` registry.
It must NOT use the singleton `state.symbol`. It must read the current position
from a **per-model** `positions` row, not the legacy `signal_state` singleton.
See "Backend schema changes" below.

### Backend schema changes for per-model status

Per-model status requires the `positions` table to carry `model_id` and
`symbol`. Without these, the engine cannot distinguish QQQ's position from
NVDA's position.

Required migration (`engine/src/db.rs`):

```rust
pub async fn migrate_multi_model(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(positions)")
        .fetch_all(pool).await?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    for (col, col_type, default) in &[("model_id", "TEXT", "''"), ("symbol", "TEXT", "''")] {
        if !existing.iter().any(|name| name == col) {
            let sql = format!(
                "ALTER TABLE positions ADD COLUMN {col} {col_type} NOT NULL DEFAULT {default}"
            );
            sqlx::query(&sql).execute(pool).await?;
        }
    }

    let rows = sqlx::query("PRAGMA table_info(equity_trades)")
        .fetch_all(pool).await?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();
    if !existing.iter().any(|name| name == "model_id") {
        sqlx::query("ALTER TABLE equity_trades ADD COLUMN model_id TEXT NOT NULL DEFAULT ''")
            .execute(pool).await?;
    }
    Ok(())
}
```

Call it in `db::open()` alongside the other migrations:

```rust
migrate_predictions(&pool).await?;
migrate_sentiment_cache(&pool).await?;
migrate_trading_models(&pool).await?;
migrate_multi_model(&pool).await?;
```

The scheduler must write position events with `model_id` and `symbol`, and
trades must be inserted with `model_id`. The status handler loads the latest
`positions` row for `(model_id, symbol)`, falling back to the legacy
`signal_state` row only for backward compatibility.

### Per-symbol endpoints

### 2. CandlestickChart does not change

**Cause:** chart title hardcoded to `"QQQ OHLC + SMA + VOLUME"`; chart data
fetched once at mount; canvas not redrawn on model switch.

**Fix:** derive the symbol from `activeModelId`, pass it to `fetchChart`,
react to model changes, and build the title dynamically:

```svelte
<script>
  import { activeModelId, models, chartData } from '../stores.js';
  import { fetchChart } from '../api.js';

  let candles = [];
  let sma = [];

  $: activeModel = $models.find(m => m.model_id === $activeModelId);
  $: primarySymbol = activeModel?.primary_symbol || 'QQQ';
  $: inverseSymbol = activeModel?.inverse_symbol || '';
  $: chartTitle = inverseSymbol
    ? `${primarySymbol} / ${inverseSymbol} OHLC + SMA + Volume`
    : `${primarySymbol} OHLC + SMA + Volume`;

  async function refreshChart(symbol) {
    const data = await fetchChart(90, symbol);   // pass primary symbol
    candles = data.candles || [];
    sma = data.sma || [];
    chartData.set(data);
  }

  $: if (primarySymbol) refreshChart(primarySymbol);
</script>

<div class="chart-label">{chartTitle}</div>
```

```javascript
// api.js
export async function fetchChart(limit = 90, symbol = null) {
  let url = `${API_BASE}/chart?limit=${limit}`;
  if (symbol) url += `&symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  // ...
}
```

**Backend:** `GET /api/chart` must accept `?symbol=` and query
`equity_candles` by that symbol instead of `state.symbol`.

### 2b. CandlestickChart throws `Ll.set is not a function` (caught 2026-08-13)

**Cause:** `chartData` is a `derived` store (read-only). Calling
`chartData.set(data)` on a derived store throws
`TypeError: Ll.set is not a function` in the production build (where `Ll`
is the minified name of the derived store). The error appears in the
browser console as "chart refresh failed: Ll.set is not a function" and
the chart silently stops updating.

**Fix:** use `updateSlice($activeModelId, 'chartData', data)` instead of
`chartData.set(data)`. The `updateSlice` function writes to the per-model
slice store, which the `chartData` derived store proxies from.

```javascript
// WRONG — throws on derived store
chartData.set(data);

// CORRECT — writes through the slice store
import { updateSlice } from '../stores.js';
updateSlice($activeModelId, 'chartData', data);
```

### 3. ModelHealth changes partially

**Cause:** `ModelHealth.svelte` loads `fetchAccuracy()` once in
`onMount` and never re-fetches when `activeModelId` changes. It also reads
legacy `status` for staleness.

**Fix:** react to the active model's primary symbol and fetch per-symbol
accuracy. The backend accuracy response still uses **legacy field names**
(`directional_1h`, `directional_4h`, `directional_24h`) but the computed
horizons are **1d / 5d / 21d**.

```svelte
<script>
  import { activeModelId, models, status } from '../stores.js';
  import { fetchAccuracy } from '../api.js';

  let accuracyData = null;

  $: activeModel = $models.find(m => m.model_id === $activeModelId);
  $: primarySymbol = activeModel?.primary_symbol;

  async function loadAccuracy(symbol) {
    if (!symbol) return;
    try {
      accuracyData = await fetchAccuracy(symbol);
    } catch (e) {
      accuracyData = null;
    }
  }

  $: loadAccuracy(primarySymbol);

  $: staleness = $status?.staleness_secs ?? 0;
</script>
```

```javascript
// api.js
export async function fetchAccuracy(symbol = null) {
  let url = `${API_BASE}/accuracy`;
  if (symbol) url += `?symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  // ...
}
```

Frontend labels:

| Response field | Display label |
|---|---|
| `directional_1h` | Dir. Acc. 1d |
| `directional_4h` | Dir. Acc. 5d |
| `directional_24h` | Dir. Acc. 21d |

### 4. Direction accuracy needs rethinking

Old endpoint reported `directional_1h`, `directional_4h`, `directional_24h`.
The new model horizons are **1d, 5d, 21d**.

**Recommended metric:** sign accuracy per horizon.

```text
For each resolved prediction at time t:
  predicted_sign = sign(pred_h[t])
  actual_return  = (close[t+h] - close[t]) / close[t]
  actual_sign    = sign(actual_return)
  hit            = predicted_sign == actual_sign

accuracy_h = mean(hit) across all resolved predictions
```

Backend response shape:

```json
{
  "model_id": "nvda-v1",
  "directional_1d": 0.52,
  "directional_5d": 0.55,
  "directional_21d": 0.53,
  "weighted_blend": 0.54,
  "resolved_count": 142,
  "mae_1d": 0.0081
}
```

Frontend label mapping:

| Horizon | Label | Good threshold |
|--------|-------|----------------|
| 1d     | Dir. Acc. 1d | >= 50% |
| 5d     | Dir. Acc. 5d | >= 50% |
| 21d    | Dir. Acc. 21d | >= 50% |

A weighted blend (e.g. `0.5*1d + 0.3*5d + 0.2*21d`) is useful as a single
summary number.

### 5. PnLEquityCurve mixes data on model switch

**Cause:** history array is not cleared, or `fetchEquityTrades` is called
with the wrong symbol.

**Fix:** clear history on every active model change; fetch by `model_id`
if the backend supports it, otherwise by `primary_symbol`.

```svelte
$: if ($activeModelId && $activeModelId !== lastModelId) {
  lastModelId = $activeModelId;
  pnlHistory = [];
  loadHistory($activeModelId);
}
```

### 6. TradeHistory is not global across models

**Cause:** `Dashboard.svelte` loads trades once with the initial symbol and
each model slice keeps its own trade list. Switching models does not give a
unified view.

**Fix:** make `TradeHistory.svelte` fetch all trades with `symbol='*'` and
display a `Model` and `Symbol` column. The backend returns every trade
across all models when `symbol='*'`.

```svelte
<script>
  import { onMount } from 'svelte';
  import { fetchEquityTrades } from '../api.js';

  let tradeList = [];

  async function loadTrades() {
    const data = await fetchEquityTrades('*', 200);
    tradeList = data?.trades || [];
  }

  onMount(() => loadTrades());
</script>
```

```javascript
// api.js
export async function fetchEquityTrades(symbol = '*', limit = 200) {
  const res = await fetch(`${API_BASE}/equity/trades?symbol=${symbol}&limit=${limit}`);
  // ...
}
```

**Backend:** `GET /api/equity/trades?symbol=*` must omit the `WHERE symbol`
clause and return `model_id` and `symbol` in each row. Per-symbol queries
(`?symbol=QQQ`) still work for components that need a single model's trades.

## Strategy config: global vs per-model

The engine supports per-model strategy params. Two UX options:

| Option | Behavior | When to use |
|--------|----------|-------------|
| **Per-model** (current) | Each model has its own thresholds | Different volatilities (QQQ vs NVDA) |
| **Global** | One config applies to all models | Same strategy across all assets |

**Recommendation:** keep per-model as the default. QQQ and NVDA have very
different volatility regimes; sharing thresholds would hurt one of them.
Add a "Copy settings to all models" button only if the user explicitly
asks for global sync.

## WebSocket event routing

The backend sends per-model events with `model_id`. The frontend handler
routes them to slices:

```javascript
case 'PnlTick':
  updateSlice(msg.model_id, 'status', ...);
  break;
```

Events without `model_id` (ModeChange, EngineEvent) go to global stores.

## Verification checklist

After wiring a new component:

1. Switch models in the dropdown.
2. Check each panel updates: **symbol, position, entry, last close, chart,
   PnL curve, accuracy, trades, strategy config**.
3. Open the Network tab; confirm new `fetch` requests include `symbol`
   (and `model_id` for `/api/status` if implemented).
4. Trigger a trade or config change; confirm the WebSocket event carries
   the correct `model_id`.
5. Hard-refresh the page on the non-default model; confirm the URL or
   store restores the active model correctly.
6. Run the backend test suite: `cargo test --lib` (expect 22 pre-existing
   config failures; the code changes should not reduce the pass count).

## Anti-patterns

- **Don't use `onMount` only.** Always react to `activeModelId` changes.
- **Don't hardcode the symbol in labels.** Use `model.primary_symbol` /
  `model.inverse_symbol`.
- **Don't reuse a flat array for all models.** Partition by `model_id`
  in stores.
- **Don't compute per-model accuracy in the component.** The backend
  should precompute it; the component just renders.
- **Don't call `.set()` on a `derived` store.** Svelte `derived` stores
  are read-only — they don't have a `.set()` method. Calling
  `chartData.set(data)` on a derived store throws
  `TypeError: Ll.set is not a function` (where `Ll` is the minified name
  of the derived store in the production build). The error appears in the
  browser console as "chart refresh failed: Ll.set is not a function".
  Fix: use `updateSlice($activeModelId, 'chartData', data)` — the
  per-model slice store's write path — instead of calling `.set()` on
  the derived proxy. This was the root cause of the chart refresh error
  after the multi-model migration.
