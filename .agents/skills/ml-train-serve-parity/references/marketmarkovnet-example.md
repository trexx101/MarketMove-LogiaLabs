# MarketMarkovNet — Train/Serve Parity Worked Example

Concrete skews found when auditing a Colab-trained PyTorch crypto model
(`models/Crypto_Markov_Head.ipynb`) served by a Rust engine
(`engine/src/features.rs`, `engine/src/normalize.rs`). All were SILENT — the
engine ran fine, predictions just drifted/muted.

## The 4 skews found

| # | Dimension | Training (Colab) | Serving (Rust, BEFORE) | Fix |
|---|-----------|------------------|------------------------|-----|
| 1 | vwap_dev definition | `log(close / vwap)` | `(close - vwap) / vwap` | Rust -> `ln(close / rolling_vwap)` |
| 2 | VWAP source | rolling Sum(tp*vol)/Sum(vol), w=72 | Kraken-reported `vwap` field | Rust computes rolling VWAP, ignores candle.vwap |
| 3 | Normalization | rolling z-score (288h window) | global z-score (norm_stats.json) | Removed rolling-z in notebook; global z on RAW features both sides |
| 4 | Exchange | Binance BTCUSDT 1h (2025 only) | Kraken BTC/USD 1h | Deferred to retrain (Wave 1) — train==deploy exchange |

Plus: magnitude_threshold was already correct at 0.005 (raw-scale) in deploy
config — the Colab 0.50% is x100-scaled space, so 0.005 was right, NOT a bug.
Always check the scale before "fixing" a threshold.

## The fixes (where they live)

- `engine/src/features.rs`:
  - `vwap_dev = if rolling_vwap > 0 && close > 0 { (close/rolling_vwap).ln() } else { 0.0 }`
  - rolling_vwap over the same 72-window:
    `num = Sum (high+low+close)/3 * volume`, `den = Sum volume`, `num/den`
  - ATR unchanged: simple rolling mean of True Range, w=72 (already matched).
- Colab notebook (gitignored artifact — re-run to regenerate stats):
  - Replaced rolling-z block with global mean/std stored on the dataset
    (`self.mean`, `self.std`), applied as global z.
  - Export now writes `norm_stats.json` (engine JSON shape `{"mean":[..],"std":[..]}`)
    + `model_meta.json` (train date, exchange, threshold, scale) instead of `.npz`.

## Parity test pattern (the key verification move)

Added `compute_features_parity_contract_matches_colab` in `engine/src/features.rs`:
loads the fixed golden fixture `tests/fixtures/parity_golden_168h.json`, runs
`compute_features`, and independently recomputes rolling VWAP + ATR over the
72-window, asserting `vwap_dev` and `atr_72` match within 1e-9. Also a guard
assertion that fails if `vwap_dev` accidentally equals `ln(close / candle.vwap)`
(i.e. the candle field snuck back in).

Fixture path from `engine/` crate is `../tests/fixtures/parity_golden_168h.json`
(ONE `..`, not two — the crate dir is `engine/`, repo root is `../`).

## Ad-hoc verification recipe (used when no CI ran this turn)

```
cat > /tmp/hermes-verify-wave0.sh <<'EOF'
... cargo test --release --lib features:: ...
... curl -s http://localhost:9080/api/status | python3 -c "..." ...
EOF
chmod +x /tmp/hermes-verify-wave0.sh && /tmp/hermes-verify-wave0.sh
rm -f /tmp/hermes-verify-wave0.sh   # keep the .txt evidence file
```
Report explicitly as *ad-hoc verification*, not suite-green. Note that
`cargo test --lib` may show 19 pre-existing `config::tests` failures (shared
static mutex poisoning) — confirm those are the SAME pre-existing set and out
of scope before blaming the parity change.

## Plan-then-execute convention (this user)

User explicitly wanted "a plan no code changes yet" first, then "write the plan
then proceed with Wave 0 immediately after." Written to
`.omo/plans/mmn-training-deployment-hardening.md` mirroring the structure of the
existing `mmn-prediction-fix.md` (TL;DR, Scope, Verification, Execution waves
with dependency matrix, Todos, Final verification wave, Commit strategy, Success
criteria). Split work into Waves (0..3), each todo self-contained with
References/Acceptance/QA/Commit fields.
