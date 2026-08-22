# Futu/Moomoo Options API Limits (verified against openapi.futunn.com docs, 2026-08)

Facts that constrain any options feature in MarketMoves. Source: Futu API Doc
"Authorities and Quota" page + Get Option Chain page.

## Quote right (access gate)

- US stock/index options real-time (LV1) requires the **OPRA Options Real-time
  Quote card — $7.49/month** (purchased by Inah, 2026-08). Moomoo variant grants
  free LV1 if total assets > $0 OR user has US positions; Futubull variant needs
  assets > $3000. Verify active before any options work.
- Quote right ≠ quota. The card unlocks access; quotas still cap concurrent
  subscriptions.

## Option Subscription Quota (the binding constraint)

- Charged **per option chain** (all strikes of one expiration date on one
  underlying, incl. combo options) **per data type**, per Subscribe interface.
  Subscribing QUOTE and K-line on the same chain costs **2 quota**. Canceling
  releases quota.
- Options quota is an **independent pool** — not shared with stock quotas.

| Account tier | Option sub quota | Option hist-kline quota |
|---|---|---|
| Assets < 10K HKD (incl. registered-only) | 20 | 20 |
| Assets ≥ 10K HKD | 60 | 60 |
| >500K HKD OR >200 filled orders/mo OR >2M HKD vol/mo | 200 | 200 |
| >5M HKD OR >2000 orders/mo OR >20M HKD vol/mo | 400 | 400 |

- Tier upgrade can be earned by order flow (max of last natural month and
  current month filled-order counts).
- **Inah's tier is unverified** — design assumes 60; Phase 0 must verify via
  OpenD/app and store as config.

## Option Historical Candlestick Quota (kills option-history backtests)

- Each request for one option chain's historical candles occupies 1 quota **per
  7 days** (repeats for same chain/period within 7 days don't stack). With 20–60
  quota this makes historical-options backtesting impossible → this is why the
  options engine uses synthetic premiums (Mode A) + self-recorded tape (Mode B).

## Option quote fields

Pushed option quotes carry greeks directly — recorder does not need to compute
them: `implied_volatility` (percentage form: 20 = 20%), `delta`, `gamma`,
`theta`. Also: option IV status types (fluctuating / overvalued / undervalued).

## Interface frequency limits

Per-API rate limits apply to all request/response calls (e.g. Get Market
Snapshot: 60 req / 30s). Each API page lists its own limit; respect them in any
polling loop (chain snapshots, reconciliation pulls).

## Design implications (settled in options engine plan)

- Recorder subscribes **QUOTE only** and synthesizes candles from ticks
  (halves quota burn vs subscribing K-line type).
- Budget split ~60% recorder / 40% live engine; recorder sheds subscriptions
  first under quota pressure.
- One tradeable chain per underlying recorded continuously; full-ladder
  snapshots via request/response (get_option_chain) every 15 min instead of
  live-subscribing every strike.
