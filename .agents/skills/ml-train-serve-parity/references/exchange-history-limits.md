# Exchange free-history limits + quant-review notes (MarketMarkovNet audit)

Condensed knowledge from the MarketMarkovNet train/serve parity + model-review
session. Pair with `ml-train-serve-parity` SKILL.md.

## 1. Free OHLC history limits (verified by live probe, 2026-07-19)

| Venue | Free multi-year history? | How to get it |
|-------|--------------------------|---------------|
| **Kraken** `0/public/OHLC?pair=XBTUSD&interval=60` | **NO** — `since` param is IGNORED; always returns only the most recent ~720 candles (~30 days). Unauthenticated. Paginating forward with the returned `last` cursor also rate-limits hard ("EGeneral:Too many requests" after ~50 reqs, no backoff). | Buy a Kraken data plan, or use the WS + a long-running collector. Not viable for a one-shot retrain. |
| **Binance Vision** `https://data.binance.vision/data/spot/monthly/klines/<SYM>/1h/<SYM>-1h-<YYYY>-<MM>.zip` | **YES** — full 2017+ monthly CSV zips, one per symbol/month. `urllib.request.urlretrieve` + `zipfile` extract. Reliable, no auth. | Use for any multi-year crypto training set. |

Symbol mapping: Binance `BTCUSDT` to Kraken `XBTUSD` / display `BTC/USD`.
Kraken OHLC array shape: `[time, open, high, low, close, vwap, volume, count]`
(7 fields, wrapped under pair key `XXBTZUSD`). This matches the Rust engine's
REST parser in `engine/src/data/rest.rs`.

## 2. train == serve resolution matrix (when live venue != free-history venue)

| Option | Action | Tradeoff |
|--------|--------|----------|
| Switch deploy to Binance | Change engine REST+WS to Binance `BTCUSDT`; train on Binance Vision CSV | True train==serve, free multi-year data. But changes the user's live venue (Kraken acct). |
| Train on Binance, keep Kraken deploy | Train on Binance CSV; **DOCUMENT** the mismatch in `meta.json` as known venue-drift risk | No engine change; mismatch remains (microstructure/symbol drift). |
| Buy Kraken data plan | Train on real Kraken multi-year | Cleanest but costs money. |
| Train on Kraken free ~30d | — | **Not viable**: one regime, severe overfit. Reject. |

Rule of thumb: never mix venues silently. If you must, flag it in model metadata
so a future retrain/rollback review catches it.

## 3. Quant-review: the "near-zero prediction" trap

Symptom the user reported: predictions were "terribly low" / muted. A common
"fix" is a **directional hinge / margin penalty** in the loss:

    penalty = relu(margin - y_true * y_pred)   # pushes preds away from 0

This DOES raise prediction magnitude — but it amplifies NOISE into
confidently-wrong predictions. The model gains no real edge.

**How to detect it (the only honest test):**
- Run the held-out evaluation and read the NUMBERS, not the prediction sizes.
- Directional hit-rate ~ 50% => coin flip, no edge.
- Pearson correlation(pred, actual) ~ 0.01-0.04 => essentially zero linear relationship.
- If those hold while predictions "look healthy," the model is confidently wrong.

MarketMarkovNet actual eval (held-out, 2025): hit-rate 49.5/50.4/51.4% (1h/4h/24h),
corr 0.0096/0.0108/0.0367, MAE 0.39/0.86/2.01%. => No edge. The x100 + hinge
masked the "near-zero" symptom without creating signal.

## 4. Backtest metrics that actually matter

- Report **Sharpe, max drawdown, turnover, and out-of-sample equity** — not just
  cumulative return or hit-rate.
- A regime-filtered backtest showing +29% with hit-rate 47% is an artifact of the
  test window's trend (tail of a single year), NOT skill.
- Require **walk-forward / purged k-fold CV** (TimeSeriesSplit + embargo) instead
  of one contiguous 70/15/15 split (which overfits to the training year).
- Gate any retrain on: does Pearson/IC beat ~0.05 out-of-sample BEFORE live capital.

## 5. Threshold scale footgun

Notebook backtester used `magnitude_threshold = 0.50` (x100 scaled space).
Deployed engine uses `0.005` (raw log-return space, because the model divides
targets by 100 on export). These are CONSISTENT only because of that /100 —
but it is a footgun: anyone retraining and forgetting the scale ships a model
that never trades or always trades. Always record the threshold + its unit in
`meta.json`.
