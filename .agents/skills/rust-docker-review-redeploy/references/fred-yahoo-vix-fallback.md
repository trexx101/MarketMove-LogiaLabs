# FRED → Yahoo VIX Fallback (MarketMoves Wave C)

## Why this matters

The engine's `equities_v2::compute_equity_features()` uses a VIX-derived
`vix_regime` feature (0=calm, 1=normal, 2=stress) that's part of the
TCN input vector. When `vix_regime` is wrong, the model produces bad
predictions — even if every other feature is correct.

The primary source is FRED's CSV endpoint at
`https://fred.stlouisfed.org/graph/fredgraph.csv?id=VIXCLS`. From the
VPS, this times out consistently (network blocked). So the macro
feature always degrades to 0.0, making the model "blind" to volatility.

## Solution: Yahoo `^VIX` fallback

Yahoo Finance publishes the same VIX index under ticker `^VIX`. The
Yahoo client in `engine/src/data/yahoo.rs` already handles indices —
the `fetch_chart` endpoint returns daily OHLCV for any ticker
including `^VIX`. No new infrastructure needed.

The fix in `data/mod.rs::backfill_equities()`:

```rust
// After FRED backfill completes:
let vix_count = crate::db::count_equity_candles(pool, "$VIX").await?;
if vix_count <= 1 {
    info!(count = vix_count, "FRED $VIX missing/empty — fetching ^VIX from Yahoo as fallback");
    match yahoo::backfill(pool, "^VIX", 1, "2y").await {
        Ok(n) if n > 0 => info!(rows = n, "Yahoo ^VIX fallback loaded — $VIX macro features active"),
        Ok(_) => debug!("Yahoo ^VIX returned 0 new rows (already up to date)"),
        Err(e) => tracing::warn!(error = %e, "Yahoo ^VIX fallback failed — VIX features will be 0.0"),
    }
}
```

Add `debug` to the tracing import:

```rust
use tracing::{debug, info};
```

## Symbol mapping — the second trap

Yahoo stores data as `^VIX` (with the caret). FRED stores data as
`$VIX` (with the dollar sign, in `equity_candles`). The scheduler
loads `^VIX`. The features API handler must match — querying `$VIX`
yields 0 rows and 0.0 features.

The `/api/equity/features` handler had this bug — it queried `$VIX`,
got nothing, and showed `vix_regime=0.0`. Fix:

```rust
let vix_candles = db::fetch_equity_candles_asc(&state.pool, "^VIX", limit)
```

## Verification after fix

```bash
# 1. Yahoo backfill fired on startup
docker logs mmn-engine 2>&1 | grep "Yahoo ^VIX fallback loaded"
# Expected: "Yahoo ^VIX fallback loaded — $VIX macro features active rows=N"

# 2. ^VIX has data in DB
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/equity/data?symbol=^VIX&limit=3'
# Expected: count >= 3, recent dates, close ~15-30

# 3. vix_regime is non-zero in features (needs enough rows for the lookback)
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/equity/features?symbol=QQQ&limit=1260' \
  | python3 -c "import sys, json; d=json.load(sys.stdin); print('vix_regime:', d['latest']['vix_regime'])"
# Expected: 0.0 (calm), 1.0 (normal), or 2.0 (stress) — not stuck at 0.0

# 4. Default features call uses small limit
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/equity/features?symbol=QQQ' \
  | python3 -c "import sys, json; d=json.load(sys.stdin); print('default limit vix_regime:', d['latest']['vix_regime'])"
# NOTE: With the default limit, the `latest` row may be from years ago
# because the API returns the last N rows of the compute_equity_features
# output, which uses ALL available history including very old VIX data.
# Always pass ?limit=1260 to get the current snapshot.
```

## Other FRED series

The same fallback pattern works for `$UST10Y` and `$DXY` — Yahoo has
`tickers ^TNX` (CBOE 10Y yield) and `DX-Y.NYB` (US Dollar Index). Add
similar blocks for each in `data/mod.rs`. The same symbol-mapping rule
applies: store under the FRED symbol in `equity_candles` (so any
existing queries keep working), but accept that the Yahoo fallback
uses the Yahoo ticker.