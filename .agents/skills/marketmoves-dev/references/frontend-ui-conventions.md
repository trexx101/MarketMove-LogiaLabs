# MarketMoves Frontend UI Conventions (Svelte 4 + Vite)

For adding/modifying views in `frontend/src/`. Distilled from Phase 7 Options UI work.

## Structure conventions

- Views live in `frontend/src/views/*.svelte`, one per nav entry. Registered in TWO places in `App.svelte`: the `import` block at top, and the `{#if currentView === '...'}` chain in `<main class="content">`.
- Nav sections are plain `<li class="section-label">OPTIONS</li>` items between buttons; CSS: uppercase 0.62rem muted, letter-spacing 0.08em.
- API calls in `frontend/src/lib/api.js` — one exported async function per endpoint, JSDoc with the response shape, `API_BASE` prefix, throw `Error` with endpoint name on `!res.ok`.
- View pattern (copy from Ledger.svelte / OptionsPositions.svelte):
  - `onMount` → `load()` + `setInterval(load, N)`; `onDestroy` → `clearInterval`. Positions/Monitor use 30s, Events 15s.
  - `.view-header` (h1 + controls), optional `.summary-row` of `.stat` blocks, `.table-card > .table-wrap > table`.
  - `loading`/`error` state vars; error rendered as `.error` box (`--red-subtle` bg, `--red` border).
  - `fmt(v, dec)` / `fmtTime(ts)` / `pnlColor(v)` helpers copied per-view.

## CSS variables (defined once in App.svelte `:global(:root)`)

Dark Kraken palette: `--bg-base #0c0d12`, `--bg-surface #15161e`, `--bg-surface-hover #1c1d27`, `--bg-inset #0a0b0f`, `--border #252631`, `--text-primary #ececf1`, `--text-secondary #8b8d9a`, `--text-muted #5c5e6e`, `--accent #7132f5` (+`-dark`, `-subtle`, `-glow`), `--green #149e61`, `--red #e5484d`, `--yellow #d29922` (each with `-subtle` rgba variants), `--radius/-sm/-xs`, `--font`, `--font-mono`.

Badges: `.badge` inline-block, subtle bg + strong text color per state. Mono numerics: `font-family: var(--font-mono); font-variant-numeric: tabular-nums;`.

## Data-contract traps (verified against live API)

1. **Timestamp units are INCONSISTENT across tables.** `engine_events.ts` is SECONDS (`Utc::now().timestamp()` in `db::insert_event`); `option_positions`/candles are MILLIS (`timestamp_millis()`). Frontend date formatters must normalize: `new Date(ts < 1e12 ? ts * 1000 : ts)`. Events also expose `ts_rfc3339` — usable as a fallback display.
2. **Config registry `kind` serializes as `"int"` / `"float"`** (custom Serialize impl), NOT Rust's `i64`/`f64`. Settings page branches on `entry.kind === 'int'`.
3. **Config PUT is partial-apply**: `{"applied": N, "rejected": ["key: reason"]}`. Unknown/out-of-range keys are rejected individually, valid ones still apply. UI should show both counts. Config GET returns `{entries: [{key, value, default, min, max, tier, kind, label, description, updated_at}], count}`; `tier` is `"strategy"|"rail"` (rail keys get yellow badge).
4. **Trades endpoint returns `OptionPosition` rows** (same shape as positions), just closed-only. Positions/trades responses: `{positions|trades: [...], count}`.
5. Events response row: `{id, ts, ts_rfc3339, category, severity, mode, source, message, payload, equity}`. Filters: `category, mode, severity, equity, since, limit` (limit clamp 1..1000). Categories closed set: trade|data|system|strategy|alert|advisor; mode paper|live; severity info|warn|error.

## Build/verify

- `cd frontend && npx vite build` (background — guard may flag it as server-like; use terminal background=true + notify). Clean build ≈ 75 modules, ~3.4s.
- `vite build` compiles Svelte — catches template/syntax errors but NOT runtime field-name mismatches. After build, smoke-test endpoints with curl against the live engine (see marketmoves-ops for boot quirks: `NORM_STATS_PATH` env needed, ~8s startup with VIX/FRED backfill, existing engine holds port 8080).
