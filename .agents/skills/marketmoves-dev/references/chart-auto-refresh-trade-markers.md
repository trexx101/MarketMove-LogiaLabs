# Chart Auto-Refresh + Trade Markers Implementation

## Problem: Static Chart
The candlestick chart fetched `/api/chart` once on mount and never refreshed.
New candles, trades, and prediction updates were invisible without a page reload.

## Solution: Three-Phase Refresh

### 1. Timer auto-refresh (60s)
```javascript
refreshTimer = setInterval(async () => {
  await refreshChart();
  await refreshTrades();
}, 60000);
```

### 2. WS-triggered refetch (store clear pattern)
When `StrategyConfigChange` fires (SMA window changed), old chart data is stale.
WS handler clears `chartData` store to `null`. Chart component's reactive block
detects the null and refetches:
```javascript
$: if ($chartData === null && candles.length) {
  refreshChart();
}
```

### 3. Prediction cone field name mismatch fix
REST `/api/predictions` returns `pred_24h` / `pred_5h_approx` / `pred_1h`.
WS `PredictionUpdate` sends `pred_1d` / `pred_5d` / `pred_21d`.
Chart expected `pred_1d` — got `undefined` from REST, cones were broken on
initial load. Fixed with fallback chain:
```javascript
preds = {
  pred_1d: p.pred_1d ?? p.pred_24h ?? p.pred_1h,
  pred_5d: p.pred_5d ?? p.pred_5h_approx,
  pred_21d: p.pred_21d,
};
```

## Trade Markers
- Green up-triangle at long entry price
- Red down-triangle at short entry price
- `tsToX()` maps trade timestamps to candle x-positions (exact match then
  nearest-timestamp fallback)
- Trade prices included in Y-axis range calculation

## Trade API Shape
`GET /api/equity/trades?symbol=QQQ&limit=200` returns:
```json
{
  "trades": [
    { "ts": "...", "side": "long|short", "price": 400.0, "qty": 1.0, ... }
  ]
}
```
Trades are chronological (oldest first). Each trade has `ts` as RFC3339 string
matching candle timestamps for exact x-position mapping.

---

## CRITICAL: Chart Staleness + Live Price Anchor

**The #1 chart bug: stale OHLCV candles + live price = prices off by 2-3x.**

Symptom: chart renders 2021-era candles (QQQ ~$365) while live price and
predictions show $688. Chart Y-axis is centered on wrong-era data.

Root cause: Yahoo backfill fails (rate-limit 429, network timeout, or
backfill never ran), leaving only old candles in the DB. The chart endpoint
returns these stale candles without flagging them.

**Fix: backend bundles live_quote into every chart response + frontend
re-centers Y-axis on the live price when stale.**

### Backend: always fetch and return live_quote

`engine/src/api/chart.rs` — always call `fetch_quote()` even when candle
backfill fails. Return it in every chart response:

```rust
#[derive(Serialize)]
pub(crate) struct ChartResponse {
    pub candles: Vec<CandleDto>,
    pub sma: Vec<SmaPoint>,
    pub stale: bool,             // candles > 48h old
    pub live_quote: Option<LiveQuote>,  // ALWAYS included
}

#[derive(Serialize)]
pub(crate) struct LiveQuote {
    pub price: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
}
```

The `live_quote` call uses Yahoo's meta block (fast, minimal data) — separate
from the OHLCV backfill. Even if candles are stale, live_quote succeeds as
long as Yahoo is reachable.

### Backend: always attempt backfill, flag staleness

```rust
// Always try to refresh — skip if data is fresh enough (12h threshold)
match crate::data::yahoo::backfill(&state.pool, &state.symbol,
    200, range, 43_200).await {
    Ok(n) => { if n > 0 { tracing::info!(fetched=n, "backfill complete"); } }
    Err(e) => { tracing::warn!(error=%e, "backfill failed — serving DB data"); }
}

let stale = candles.is_empty()
    || now_ts.saturating_sub(latest_ts) > 172_800; // 48h
```

### Frontend: use live_quote from chart response, NOT a separate fetch

```javascript
// In refreshChart() — single round-trip, no separate fetchQuote():
const data = await fetchChart();
candles = data.candles || [];
isStale = !!data.stale;
if (data.live_quote) {
  livePrice = data.live_quote.price;  // primary price anchor
}
```

**Rationale:** A separate `fetchQuote()` call doubles network round-trips
and creates a race condition where `livePrice` and `candles` may be from
different Yahoo requests. Bundling the live quote into the chart response
keeps them consistent.

### Frontend: re-center chart on live price when stale

```javascript
// In draw() — AFTER computing minP/maxP from candles:
// ... candle range calc + expand to include prediction targets ...

if (isStale && livePrice != null) {
  const half = (maxP - minP) / 2;
  minP = livePrice - half;
  maxP = livePrice + half;  // Y-axis now centered on today's price
}
```

Stale candles still render (xStep computed from their count) but the visible
Y-axis is anchored to TODAY. Prediction cones project from $688, not $365.

### Frontend: live quote + stale indicator in UI

```svelte
{#if liveQuote}
  <div class="quote-badge" class:stale={isStale}>
    <span class="quote-price">{liveQuote.price.toFixed(2)}</span>
    <span class="quote-change" class:up={liveQuote.change >= 0} class:down={liveQuote.change < 0}>
      {liveQuote.change >= 0 ? '+' : ''}{liveQuote.change.toFixed(2)}
      ({liveQuote.change_pct >= 0 ? '+' : ''}{liveQuote.change_pct.toFixed(2)}%)
    </span>
    {#if isStale}<span class="stale-tag">stale data</span>{/if}
  </div>
{/if}
```

**Yahoo rate-limiting (HTTP 429):** If the container's VPS IP is 429'd by
Yahoo, `fetch_quote()` fails. `live_quote` becomes `None`. The frontend
shows no quote badge and falls back to `last_close` (less accurate but
functional). The `stale` flag stays `true` from the empty/old candles.

**Diagnosis commands:**
```bash
# 1. Check if backfill is succeeding inside the container
docker logs mmn-engine --since 5m | grep -i "yahoo\|backfill\|chart"

# 2. Test Yahoo directly from inside the container (is it 429?)
docker exec mmn-engine curl -s -o /dev/null -w "%{http_code}\n" --max-time 10 \
  "https://query1.finance.yahoo.com/v8/finance/chart/QQQ?interval=1d&range=5d"

# 3. Verify the chart endpoint returns live_quote + stale flag
IP=$(docker inspect mmn-engine --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
curl -s "http://${IP}:8080/api/chart?limit=2" | python3 -c "
import json,sys
d = json.load(sys.stdin)
print('stale:', d['stale'])
print('live_quote:', d.get('live_quote'))
print('candles:', len(d['candles']))
"
```