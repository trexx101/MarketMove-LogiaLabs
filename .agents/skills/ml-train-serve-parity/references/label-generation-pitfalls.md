# Label Generation & Training Pipeline Pitfalls

Captured during MarketMarkovNet Wave 5 training pipeline development
(penetration labels, TCN training, walk-forward validation).

## 1. ATR NaN Propagation via `ewm(adjust=False)`

**Symptom:** All penetration labels are 0.0 despite hours of lookahead data.
Penetration rate printed as 0.00% for every horizon. No error traceback.

**Root cause:** `compute_atr` computes True Range components — `high - low`,
`abs(high - close.shift(1))`, and `abs(low - close.shift(1))`. At index 0,
`close.shift(1)` is NaN, so the second and third components are NaN.
`pd.concat([...], axis=1).max(axis=1)` on inputs containing NaN returns NaN
for that row. Then `ewm(span=N, adjust=False).mean()` on a Series starting
with NaN PROPAGATES NaN through every subsequent row — the EWM never
converges. All ATR values are NaN → all barriers (`close * (1 ± k)`) are
NaN → all barrier comparisons (`fh >= upper`) return False → all labels say
"no penetration."

**Fix (one line):** `.fillna(0)` before the ewm call:
```python
tr = pd.concat([high_low, high_close, low_close], axis=1).max(axis=1).fillna(0)
return tr.ewm(span=window, adjust=False).mean()
```

**Lesson:** `ewm(adjust=False)` does NOT skip NaN — it poisons the entire
window. Always `.fillna()` or truncate before feeding into any rolling
window function when the input may contain NaN.

## 1b. Absolute vs Fractional Barrier `k` (Units Mismatch)

**Symptom:** Penetration rate is still 0.0% after the NaN fix. ATR values
look reasonable (~1000 USD for BTC). No errors raised.

**Root cause:** `k = c * df['atr']` produces an ABSOLUTE value (e.g.
0.15 × 1000 = 150). Then `upper = close * (1 + k)` = 70000 × (1 + 150) =
~$10.5M. The barrier is 151× the close price — bitcoin never hits $10.5M in
12 hours, so no barrier is ever touched. The code runs without error because
the math is valid — just catastrophically wrong in units.

**Fix:** Divide by close to make `k` a dimensionless fraction:
```python
df['k'] = c * df['atr'] / df['close']  # k ≈ 0.15 * 1000/70000 = 0.00214
```

**Lesson:** ATR is in price units. When used as a multiplicative factor in
`close * (1 + k)`, `k` MUST be dimensionless. Always normalize by price.

## 1c. Timestamp Overflow: `open_time * 1000` When Already Milliseconds

**Symptom:** Binance API returns empty results or HTTP errors when fetching
futures data. The `startTime` parameter appears as a 16+ digit number.

**Root cause:** Binance Vision CSV `open_time` column is already in
milliseconds (13 digits, e.g. `1640995200000`). Code that does
`start_ms = int(df['open_time'].iloc[0]) * 1000` produces a 16-digit number
(`1640995200000000`) that the API silently ignores or rejects.

**Fix:** Use `open_time` directly — it IS in milliseconds:
```python
start_ms = int(df['open_time'].iloc[0])       # CORRECT: 1640995200000
# NOT: start_ms = int(df['open_time'].iloc[0]) * 1000  # WRONG: 1640995200000000
```

**Lesson:** Binance CSV `open_time` = milliseconds since epoch. Do NOT
multiply by 1000. Check the digit count: 13 digits = ms, 10 digits = sec,
16 digits = nanoseconds (wrong).

## 2. Signed-Regression Target vs Classification + Magnitude Heads

**The failure mode:** A penetration-label model uses 3-class direction
(up/down/neutral) + magnitude regression heads with Focal Loss. When >99%
of bars are "neutral" (no penetration within the lookahead window), the
Focal Loss on the majority class trivially zeroes out → the model learns
"always predict neutral" → magnitude loss is masked → total loss → 0 →
model outputs constant → OOS Pearson IC = NaN.

**The better design:** Use a SINGLE signed-regression target:
- **Target**: `direction * signed_magnitude` — continuous value encoding
  both direction AND strength. Neutral bars have target = 0.0.
- **Loss**: plain `nn.MSELoss()` — no class imbalance, no focal loss.
- **Evaluation**: `pearsonr(pred, target)` directly — IC > 0 means edge.

## 2b. Magnitude Clipping (Prevent Outlier Domination)

**Symptom:** Loss is huge (~2500–4500) and decreases only ~5% over 50
epochs. OOS IC oscillates +0.04 to -0.05 across folds (not consistent).

**Root cause:** With tight barriers, magnitudes can be 50× the barrier
width. MSE on unbounded targets means a single 50× outlier contributes
2500× the loss of a 1× event — the model chases outliers instead of
learning directional signal.

**Fix:** Clip signed magnitudes:
```python
MAG_CLIP = 3.0
signed_mag = np.clip(direction * max_pen, -MAG_CLIP, MAG_CLIP)
```

**Lesson:** Inspect `mag_mean`, `mag_std`, `mag_max` before training. If
`mag_max > 10×`, clip. Bounded targets keep MSE well-behaved.

## 3. Penetration Rate Sanity Check + Barrier Calibration

Print penetration rate per horizon before training.

**Corrected barrier calibration for BTC 1h (empirical, 4+ years data,
confirmed on Colab 2026-07-21):**
- `c=0.15` → **100% penetration** (barriers too tight, magnitudes explode to
  50× width). NOT useful — label becomes "how far" not "will it hit."
- `c=0.5` → **99.7-100% penetration** (STILL too tight). Barriers at ~0.2%
  from entry; BTC easily moves 0.2% in 12h. The previous claim of "40-60%
  penetration at c=0.5" was WRONG — it was not confirmed on real data.
  The ATR(72)/close ratio for BTC 1h is ~0.4-0.7%, so c=0.5 gives barriers
  at ~0.2-0.35% — well within BTC's 12h range.
- `c=2.0+` → **expected ~40-60% penetration** (not yet confirmed on Colab).
  Barriers at ~0.8-1.4% from entry. This is the likely sweet spot.
- If penetration rate is 0%: check NaN propagation (#1), then units
  mismatch (#1b). If penetration rate is >95%: widen barrier (increase c).
- **Rule:** calibrate c empirically by running label generation first and
  checking penetration rates BEFORE training. Never assume a c value —
  test it. The penetration rate IS your label quality metric.

## 3b. Placeholder Zero Features → Guaranteed No Edge

**Symptom:** Walk-forward IC oscillates around zero (±0.05). Mean IC
~0.005-0.007 — below the 0.03 deploy gate.

**Root cause:** 4 of 6 features (funding_rate, basis_z, ob_imbalance,
llm_bull_prob) are placeholders — all zeros or constant 0.5. The model only
has 2 real features (vol_regime, vol_break). Two volatility features cannot
produce directional edge — they capture vol regime, not price direction.

**Fix:** Wire real feature data:
- `funding_rate`: Binance Futures funding rates. API geo-blocked from Colab
  (HTTP 451) — use `data.binance.vision` file downloads instead (see #5-6).
- `basis_z`: fetch futures klines, compute `(fut - spot)/spot`, z-score
  72h window, clip [-5, 5]. Also geo-blocked via API — use Vision files.
- `ob_imbalance`: from CSV `taker_buy_base`: `2 * (taker_buy_base/volume)
  - 1`, clip [-1, 1]. FREE — already in the spot kline CSVs, no API needed.

**Lesson:** Print feature statistics (mean, std per column) before training.
If any feature has `std ≈ 0`, it's a placeholder. Fix the data pipeline, not
the model architecture.

## 3c. GARCH(1,1) with Fixed Params Produces Constant Output

**Symptom:** `vol_regime` feature has mean=0.732, std=0.000024 — essentially
a constant. The feature contributes zero signal to the model.

**Root cause:** GARCH(1,1) with params ω=0.05, α=0.1, β=0.85. For BTC 1h
log-returns (~0.001), the returns squared (r² ≈ 1e-6) are TINY relative to
ω=0.05. The GARCH recursion `σ² = ω + α·r² + β·σ²` converges to
`σ² → ω/(1-β) = 0.05/0.15 = 0.333` regardless of returns, because α·r² is
negligible. The scaled output `2·σ/(σ+1)` is always ~0.732.

**Fix:** Either:
1. Retune GARCH params for the asset's return scale (ω~1e-6, α~0.05, β~0.90)
2. Replace with a simpler ratio: `realized_vol(24h) / realized_vol(72h)`
   — responsive, asset-agnostic, always produces real variance
3. Use `log(realized_vol_recent / realized_vol_long)` as the regime indicator

**Lesson:** Always print feature std before training. If std ≈ 0, the feature
is dead. GARCH with fixed params tuned for a different return scale is a
silent killer — it compiles, runs, and produces a constant that looks like
a number but carries zero information.

## 4. Full Training Pipeline Architecture (D1-D4 Pattern)

```
D1 (Rust)  →  Engine-side features (GARCH vol_regime, CUSUM vol_break)
   • Must match D2's Python implementations exactly (train==serve parity)
   • Test with deterministic candle fixtures

D2 (Python) →  Labels + feature matrix
   • FORWARD-ONLY, embargo = max(72, horizon + lookback)
   • Output: X (n_samples, 6), norm_stats v2, labels dict

D3 (Python) →  TCN training + walk-forward + deploy gate
   • Gate: mean OOS IC > 0.03 AND OOS equity > 0, or exit 1
   • Output: model.pt + norm_stats.json v2 + meta.json (gate passes only)

D4 (Rust)  →  LLM regime adapter (hourly cached, OpenRouter via HTTP)
   • NEVER in per-bar latency path — RwLock cache, 0.5 neutral fallback
```

**LLM model assignment pattern:**
- DeepSeek-R1-0528: quant cores (GARCH, labels, TCN, loss). Strong reasoning.
- Gemini 3.1 Pro: scaffolding (file trees, interfaces, build DAGs). Multi-file
  architectural synthesis.
- Agent direct: plumbing (HTTP cache, parse, config wiring).
- User: runs training on Colab GPU, reports IC results back.

## 5. Binance Futures API HTTP 451 Geo-Block from Colab

**Symptom:** `fetch_features.py` fails with HTTP 451 from Google Colab.
All three endpoints (`fapi.binance.com`, `www.binance.com`,
`data.binance.com`) return 451 (geo-blocked from US IPs).

**Root cause:** Binance Futures REST API geo-blocks US-based IPs, including
Google Colab. This is a LEGAL restriction, not a technical error — no amount
of retries, User-Agent headers, or endpoint rotation will fix it.

**Fix:** Use `data.binance.vision` file downloads (CDN-hosted, no geo-block):
- Futures klines: `https://data.binance.vision/data/futures/um/monthly/klines/<SYM>/<interval>/<SYM>-<interval>-<YYYY>-<MM>.zip`
- **Funding rates: `https://data.binance.vision/data/futures/um/monthly/fundingRate/<SYM>/<SYM>-fundingRate-<YYYY>-<MM>.zip`**
  - NOTE: `daily/fundingRate/` path returns **404** — only the **monthly** path exists.
  - CSV columns: `calc_time` (epoch ms, 13 digits), `funding_interval_hours` (always 8), `last_funding_rate` (decimal)
  - Records arrive every 8h. **Must forward-fill to hourly** before merging:
    ```python
    df['datetime'] = pd.to_datetime(df['calc_time'], unit='ms')
    hourly = df.set_index('datetime')[['last_funding_rate']].resample('1h').ffill()
    hourly = hourly.rename(columns={'last_funding_rate': 'funding_rate'})
    ```
- These are the SAME format as spot kline CSVs (monthly ZIP archives)
- Use `urllib.request.urlretrieve` + `zipfile` to download + extract

**Fallback (resilient):** If all fetch methods fail, fall back to 0.0 with a
warning. At least `ob_imbalance` (from CSV `taker_buy_base`) is always
available — no API call needed.

**Lesson:** NEVER assume Binance API endpoints work from Colab. Always use
`data.binance.vision` for historical data. Make feature fetching resilient
— catch all exceptions and fall back gracefully.

## 6. Binance Vision Futures Data URLs

```python
# Futures klines (for basis computation):
url = f"https://data.binance.vision/data/futures/um/monthly/klines/{symbol}/{interval}/{symbol}-{interval}-{year}-{month:02d}.zip"

# Funding rates:
url = f"https://data.binance.vision/data/futures/um/monthly/fundingRate/{symbol}/{symbol}-fundingRate-{year}-{month:02d}.zip"

# These are CDN-hosted ZIP files with NO geo-block (unlike the live API)
# Format is identical to spot kline CSVs
```

Run `fetch_and_merge_features(data_dir, spot_columns)` in `main()` instead of
loading CSVs directly. Handles pagination + rate limiting + NaN filling.
When API fails, use the Vision download pattern above.

## 7. Empirical IC Ceiling for BTC 1h (Confirmed on Colab, 2026-07-21)

Even with all architectural fixes applied, the walk-forward mean OOS IC for
BTC 1h consistently plateaus at **+0.021 (H1)**, below the 0.03 deploy gate.
This was confirmed across multiple Colab runs with:

- **4+ years of data** (Jan 2022 – Jul 2026, ~35k candles, bull+bear+sideways)
- **10 features** (vol_regime, vol_break, funding_rate, basis_z, ob_imbalance,
  momentum_12h, momentum_72h, vp_divergence, vol_term, range_compression)
- **Best-practice architecture** (ResidualBlock TCN, SmoothL1Loss, OneCycleLR,
  early stopping, learnable multi-task weights, median/MAD normalization)
- **Clean labels** (50.4% H1 penetration rate, time-weighted magnitude,
  auto-calibrated barrier c=2.41, signed-regression MSE)
- **Walk-forward 5-fold CV with 144-bar embargo**

Results across 5 folds:
- H1: mean IC=+0.0210, mean equity=+44.96 (4/5 folds positive)
- H2: mean IC=+0.0139, mean equity=+52.68
- H3: mean IC=+0.0172, mean equity=+57.87

Fold 3 consistently flips negative (regime-dependent), dragging the mean
below the 0.03 gate. The model has marginal signal, not deployable edge.

**Interpretation:** BTC 1h prediction at the 12-72h horizon from
OHLCV+funding+order-flow features has a real but weak signal (~0.02 IC).
This likely represents the efficient-market floor for these feature classes.
Adding microstructure features (limit-order-book depth, trade-level flow),
on-chain data (exchange flows, miner positions), or alternative signals
(derivatives open interest skew) would be the next step — but the gain per
feature is diminishing. The honest finding: this feature set produces IC ~0.02,
not 0.05+. The deploy gate exists precisely to catch this. Do NOT ship below it.
