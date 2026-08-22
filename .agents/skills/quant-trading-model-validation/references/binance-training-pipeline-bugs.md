# Binance training pipeline bugs (Wave 5 session, Jul 2026)

Debugging narrative for the MarketMarkovNet V2 training pipeline. Each bug was
identified via Colab output + code review; most required 2-3 iteration cycles.

## Bug 1: ATR NaN propagation

**Symptom:** Penetration rates = 0.0% across all 3 horizons and all 34,919 samples.
Loss → 0.0, IC = nan.

**Root cause:** `compute_atr` uses `pd.concat([high_low, high_close, low_close],
axis=1).max(axis=1)`. At index 0, `close.shift(1)` is NaN, so `.max()` returns NaN.
Then `tr.ewm(span=N, adjust=False).mean()` propagates NaN indefinitely — every ATR
value is NaN → every barrier is NaN → no barrier is ever hit → 0% penetration.

**Fix:** `.fillna(0)` after `.max(axis=1)`.

**Diagnosis path:** Printed `df['atr'].isna().sum()` in Colab — confirmed all NaN.
Then traced back to `ewm` propagation from the first-row NaN.

## Bug 2: k in absolute USD instead of fraction

**Symptom:** Same as Bug 1 (0% penetration) — persisted after the fillna fix.

**Root cause:** `df['k'] = c * df['atr']` where ATR ≈ $1,000 for BTC 1h. Then
k = 0.15 × 1000 = 150. Then `upper = close * (1 + 150)` = 70,000 × 151 = $10.5M.
Bitcoin never reaches $10.5M in 12–72h.

**Fix:** `df['k'] = c * df['atr'] / df['close']` → k ≈ 0.00214 = 0.214% barrier.

**Lesson:** ATR is in absolute price units. Any multiplier of ATR must be divided by
price to produce a dimensionless fraction. In the Rust engine the same pattern exists.

## Bug 3: GARCH fixed-params produce constant output

**Symptom:** vol_regime std = 0.000024 across 35,000 bars. Zero signal.

**Root cause:** GARCH(1,1) with ω=0.05, α=0.1, β=0.85. BTC 1h log-returns are
~0.001. The α·r² term contributes α·0.001² = 1e-7, negligible vs ω=0.05. The
recursion converges to σ² ≈ ω/(1-β) ≈ 0.333 for every window. The scaling
`2σ/(σ+1)` gives 0.732 always.

**Fix:** Replace with adaptive realized-vol percentile — compute recent 20-bar
realized vol and compare to the last 500 bars of historical rolling vol. Output
= 2 × percentile, scaled [0,2]. Only needs ~20-500 bars of close data per call,
O(n) per bar.

**Rust parity:** Exact same algorithm must be used in engine/src/features/volatility.rs
for train==serve. The training labels.py and Rust volatility.rs must match exactly.

## Bug 4: Binance API geo-block (HTTP 451)

**Symptom:** funding_rate and basis_z all 0.0. Error: `HTTP Error 451` on
fapi.binance.com, www.binance.com, and data.binance.com.

**Root cause:** Binance Futures API returns HTTP 451 (geo-block) from US-based
Colab IPs. All three endpoints fail for US users.

**Fix:** Use `data.binance.vision` CDN (no geo-block). Download daily ZIPs:
- Funding rates: `.../futures/um/daily/fundingRate/BTCUSDT/BTCUSDT-fundingRate-YYYY-MM-DD.zip`
- Futures klines: `.../futures/um/daily/klines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM-DD.zip`

Each ZIP contains one daily CSV. Forward-fill funding rate from 8h to 1h.
ZIPs have a header row — use `header=0` not `header=None`.

## Bug 5: Timestamp year overflow (year 57971)

**Symptom:** `ValueError: year 57971 is out of range`

**Root cause:** `ms_to_date` assumed open_time is in milliseconds (13 digits).
Some Binance CSVs have open_time in microseconds (16 digits).
`utcfromtimestamp(1640995200000000 / 1000)` = `utcfromtimestamp(1640995200000)` =
year 57971.

**Fix:** Auto-detect scale:
```python
if ts > 1e15: ts //= 1000   # µs → ms
if ts > 1e12: ts //= 1000   # ms → s
datetime.utcfromtimestamp(ts).strftime('%Y-%m-%d')
```

## Bug 6: Funding rate CSV has header row

**Symptom:** Empty funding DataFrame, no error. Funding stays 0.0.

**Root cause:** `pd.read_csv(z.open(z.namelist()[0]), header=None)` treats the
CSV header row ("symbol,fundingTime,fundingRate,markPrice") as data row 0.
Then `df.astype({'funding_time_ms': 'int64'})` crashes on the string header
value. The crash is inside a `try/except Exception: pass`, so it's silently
swallowed and the DataFrame stays empty.

**Fix:** `pd.read_csv(z.open(z.namelist()[0]), header=0)`, then rename columns
by position, then `pd.to_numeric(..., errors='coerce')` + `dropna` before
`astype(int64)`.

## Bug 7: Focal loss + magnitude mask collapse on >99% neutral class

**Symptom:** Loss → 0.0000 by epoch 10, IC = nan (std of predictions = 0).

**Root cause:** With `horizons_bars=(1, 4, 24)` and `k=0.5*ATR`, penetration events
are ~0.01% of bars. 99.99% are direction=0, magnitude=0. The focal loss zeroes out
on the majority neutral class, and the magnitude mask `(direction != 0)` also zeroes
out. The model trivially learns "always predict neutral" and loss → 0.

**Fix:** Replace separate dir+mag classification+regression with a single signed-
regression target `y = direction * magnitude` trained with plain MSE. No class
imbalance issue — the target is a continuous value where 0 means "no penetration."
Pair with compute_ic returning 0.0 (not nan) on zero-variance predictions.

## Model architecture traps (from Claude Sonnet 4 review)

These aren't bugs but optimization errors that suppressed OOS IC by ~0.01-0.02:

1. **No residual connections** — gradient vanishes through 6+ causal conv dilation
   layers. Add ResidualBlock with GroupNorm + SiLU + skip connection.
2. **MSELoss vs Huber** — MSE dominates outlier magnitudes. Use SmoothL1Loss.
3. **Fixed LR vs OneCycleLR** — OneCycleLR warms up then anneals. Fixed LR either
   plateaus at high loss or converges to sharp minima.
4. **No early stopping** — overfits across 5 folds. Track val OOS loss every 5 epochs,
   stop after patience=10 with no improvement.
5. **No learnable loss weights** — one horizon's scale dominates. Use softmax-scaled
   nn.Parameter per head.
6. **No feature normalization** — median/MAD scaling removes flash-crash outlier
   sensitivity vs z-score.

## Cross-run validation protocol

After fixing one bug, re-upload ALL 3 Python files to Colab, not just the changed
one — labels.py and fetch_features.py changed independently during this session and
the training file imports both. A stale upload causes the previous bug to persist
and the new run's output looks identical, wasting a Colab run.

When running `!python train_tcn.py`, the first ~90 seconds are Vision ZIP downloads
(funding rates + futures klines). If funding/basis show 0.0 after that, the ZIP
fetch failed silently. Check `len(funding_df)` or add a debug print.
