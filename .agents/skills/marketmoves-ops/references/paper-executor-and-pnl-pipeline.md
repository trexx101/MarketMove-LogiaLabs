# PaperExecutor state-sync + PnL pipeline pitfalls

## Bug 1: Executor doesn't restore position from DB on restart

**Symptom:** `equity_trades` table has only buy trades, zero sells. Realized PnL
is always 0.0. Dashboard shows "Waiting for PnL data" because pnlHistory has
< 2 points.

**Root cause:** `PaperExecutor` is constructed with
`current_position: Position::Flat` hardcoded (`paper.rs:61,85`). On every
container restart, the executor forgets it was Long/Short, even though the
`positions` table in the DB has the correct state.

The scheduler's `evaluate_and_execute_strategy` does:
```rust
let current_pos = db::load_position(...).await?;  // reads DB — correct
let new_pos = strategy::next_equity_position(current_pos, ...);
if new_pos != current_pos {
    exec_guard.set_target_position(new_pos, ...).await  // calls executor
}
```

But inside `set_target_position`:
```rust
if target == self.current_position {  // checks its OWN in-memory state
    return Ok(Vec::new());              // skips — no trade recorded
}
```

After restart, DB says "Long" but executor thinks "Flat". When strategy says
"stay Long" → `new_pos == current_pos` (both Long from DB) → executor never
called → executor stays Flat. When strategy later says "go Flat" → `new_pos !=
current_pos` (Flat != Long) → executor called with `target=Flat` → but
executor's in-memory is already Flat → `target == self.current_position` →
returns empty fills → **no sell trade recorded**.

**Fix:** Add a `sync_position_from_db(&mut self)` method to `PaperExecutor`
that queries `db::load_position(model_id, symbol)` and sets
`self.current_position` and `self.entry_price`. Call it after each scheduler
is constructed in `main.rs` (or at the start of the scheduler loop before the
first `process()` call).

## Bug 2: Executor stamps `model_id='legacy'` for all models

**Symptom:** All trades in `equity_trades` have `model_id='legacy'` regardless
of which model's scheduler executed them.

**Root cause:** `main.rs:417` calls `PaperExecutor::new_for_symbol(...)` which
hardcodes `model_id: "legacy".to_string()` at line 59 of `paper.rs`. There's a
`new_for_model()` constructor (line 70) that takes `model_id` as a parameter
— it's never used.

**Fix:** In `main.rs`, replace
`PaperExecutor::new_for_symbol(pool, fee, primary, inverse, tx)` with
`PaperExecutor::new_for_model(pool, fee, &model.model_id, primary, inverse, tx)`.

## Bug 3: PnL curve fetches per-symbol trades, should fetch global

**Symptom:** PnL equity curve shows "Waiting for PnL data" even when trades
exist in the DB.

**Root cause:** `PnLEquityCurve.svelte:32` does:
```js
const sym = m?.primary_symbol || '*';
const td = await fetchEquityTrades(sym, 200);
```
When the active model is `smh-v1`, it fetches `?symbol=SMH` → 1 trade →
`pnlHistory.length = 1` → `< 2` → renders "Waiting for PnL data".

**Fix:** Change to `fetchEquityTrades('*', 200)` so the PnL curve builds from
the global trade ledger (all models), matching the trade history table which
already uses `*`.

## Bug 4: Frontend field-name contract mismatches

**Symptom:** Model Health panel shows `N/A` for all accuracy values. Status
panel shows `—` for PnL.

**Root cause:** When the backend `AccuracyResponse` struct fields were renamed
from `directional_1h/4h/24h` and `mae_1h` to `directional_1d/5d/21d` and
`mae_1d/5d/21d`, the frontend component (`ModelHealth.svelte`) still read the
old field names → `undefined` → rendered as N/A.

**General lesson:** After renaming any API response struct fields, grep the
frontend for ALL old field names:
```bash
grep -rn "old_field_name" frontend/src
```
The built JS bundle must also be rebuilt and redeployed (the engine image
embeds `frontend/dist/`).

## Diagnostic query: verify trade data integrity

```python
sudo python3 - <<'PY'
import sqlite3
db = '/var/lib/docker/volumes/deploy_data/_data/candles.db'
conn = sqlite3.connect(db)
c = conn.cursor()
# Count trades by symbol and check for missing sells
for row in c.execute("SELECT symbol, side, COUNT(*) FROM equity_trades GROUP BY symbol, side"):
    print(f"  {row}")
# Check if positions have transitions that should have produced sells
for row in c.execute("SELECT model_id, symbol, COUNT(*), SUM(CASE WHEN position != 0 THEN 1 ELSE 0 END) FROM positions WHERE model_id IN ('smh-v1','xlf-v1','qqq-v1') GROUP BY model_id, symbol"):
    print(f"  {row}")
PY
```

If buys > 0 and sells == 0, the executor state-sync bug is the cause.

## Deploy flow after frontend fixes

The frontend is built on the host and embedded in the engine image:
```bash
cd frontend && npm run build       # → frontend/dist/
cd .. && docker-compose -f deploy/docker-compose.yml build engine
docker-compose -f deploy/docker-compose.yml stop engine
docker-compose -f deploy/docker-compose.yml rm -f engine
docker-compose -f deploy/docker-compose.yml up -d engine
```

Verify the served bundle:
```bash
# Check the HTML references the new bundle hash
curl -s http://localhost:9080/ | grep -o "assets/index-[A-Za-z0-9]*\.js"
# Check the bundle contains the new field names
BUNDLE=$(curl -s http://localhost:9080/ | grep -o "assets/index-[A-Za-z0-9]*\.js")
curl -s "http://localhost:9080/$BUNDLE" | grep -o "directional_1d\|mae_1d" | sort | uniq -c
```
