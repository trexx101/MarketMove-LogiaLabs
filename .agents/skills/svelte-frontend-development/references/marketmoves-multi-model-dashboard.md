# MarketMoves Multi-Model Dashboard Session

Reference from 2026-08-06: migrating the MarketMoves dashboard from single-model (QQQ-only) to multi-model (QQQ + NVDA) with per-model store partitioning.

## Key changes

### Backend (engine)

- `trading_models` registry table added (DDL + migration).
- Engine boot iterates enabled models, spawns one scheduler + executor per model.
- Per-model REST endpoints accept `?model_id=` query param; default/fallback returns first enabled model.
- WS events carry `model_id` and `pair` (e.g., "QQQ/PSQ", "NVDA/NVDD").
- `/api/accuracy` computes 1d/5d/21d directional accuracy (struct field names are legacy `directional_1h/4h/24h`, but data is daily horizons).

### Frontend (stores)

- `stores.js` migrated from flat singletons to per-model slices.
- `_slices` writable holds `{ [model_id]: { status, predictions, trades, accuracy, chartData } }`.
- `activeModelId` writable tracks current selection.
- Legacy stores (`status`, `predictions`, etc.) are `derived` from `activeModelId + _slices` for backward compat.

### Frontend (components)

- `Dashboard.svelte`: model selector dropdown, `loadModelData(modelId)` fetches per-model into slice.
- `websocket.js`: routes events by `msg.model_id` to slices; global events (ModeChange) update active slice only.
- `StatusPanel`: displays `status.symbol` from slice; shows model identifier header.
- `CandlestickChart`: title derived from active model `primary_symbol / inverse_symbol`.
- `PnLEquityCurve`: clears `pnlHistory = []` on model switch to avoid blending.
- `ModelHealth`: refetches `/api/accuracy?model_id=` on `activeModelId` change.
- `StrategyConfigPanel`: per-model PUT `/api/strategy-config?model_id=`; correct behavior is per-model because different models may need different thresholds.
- `TradeHistory`: intentionally global across all models (user preference); adds `model_id` badge to each row.

## Pitfalls discovered this session

### 1. Initial fetch without model_id loads wrong data

In `Dashboard.svelte`, the initial `loadModelData` called `fetchStatus()` (flat endpoint). When switching models, the slice started with stale default-model data. Fix: pass `model_id` (or derived symbol) to every fetcher:

```javascript
export async function fetchStatus(modelId) {
  const url = modelId ? `/api/status?model_id=${modelId}` : '/api/status';
  return fetch(url).then(r => r.json());
}
```

### 2. Chart title hardcoded

`CandlestickChart` had hardcoded "QQQ OHLC + SMA + VOLUME". When switching to NVDA, the title stayed "QQQ". Fix: derive title from active model:

```svelte
$: title = $activeModel ? `${$activeModel.primary_symbol} / ${$activeModel.inverse_symbol}` : '—';
```

### 3. ModelHealth fetches once on mount

`ModelHealth` called `fetchAccuracy()` in `onMount`, not reacting to `activeModelId`. Fix: reactive reload pattern:

```javascript
let lastModelId = null;
$: if ($activeModelId && $activeModelId !== lastModelId) {
  lastModelId = $activeModelId;
  loadAccuracy($activeModelId);
}
```

### 4. PnL curve blends history on switch

`PnLEquityCurve` accumulated `$status.realized_pnl` into a local array. When switching models, the new model's PnL was appended to the old model's history. Fix: clear local history on model switch:

```javascript
$: if ($activeModelId && $activeModelId !== lastModelId) {
  lastModelId = $activeModelId;
  pnlHistory = []; // clear
  loadHistory();
}
```

### 5. Strategy config: per-model vs global

The user explicitly stated: "Keep per model if each model need a different threshold." Per-model is correct. Global config would force QQQ and NVDA to share the same thresholds, which is wrong because they have different volatilities and behaviors.

### 6. Trade history: global preference

User requested trade history be global across all models (not per-model), with a `model_id` badge showing which model executed each trade.

### 7. Stale data label removed

User requested removal of the "Stale Data" staleness label from the status panel; staleness info moved to `ModelHealth`.

## Verification patterns

### Check WS routing

```bash
# In browser console, switch model and watch WS events
curl -s http://localhost:9080/api/events | jq '.[] | {event_type, model_id}'
```

### Check slice isolation

```javascript
import { get } from 'svelte/store';
import { _slices } from './stores.js';
console.log(get(_slices));
// Should show separate entries for qqq-v1 and nvda-v1
```

### Check fetch URLs

In browser Network tab, confirm `fetchStatus`, `fetchChart`, `fetchAccuracy` include `?model_id=` param.
