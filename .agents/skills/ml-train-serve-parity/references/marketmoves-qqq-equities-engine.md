# MarketMoves QQQ Equities Engine — Train/Serve Parity Notes

Session: 2026-07-23. Pivoting the engine from BTC/crypto hourly to QQQ daily
equities. Wave C model artifacts trained in Colab, served from Rust engine +
Python ZMQ inference service.

## 1. PyTorch state_dict key mismatch (CausalConv1d)

**Symptom:** `RuntimeError: Error(s) in loading state_dict for QqqTCN:
Missing key(s) in state_dict: "blocks.0.conv1.conv.weight" ...
Unexpected key(s) in state_dict: "blocks.0.conv1.weight"`

**Root cause:** The Colab training code defines:
```python
class CausalConv1d(nn.Conv1d):  # subclass — params at blocks.0.conv1.weight
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
```

The serving code (initial attempt) defined:
```python
class CausalConv1d(nn.Module):  # wrapper — params at blocks.0.conv1.conv.weight
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__()
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
```

Same math, different state_dict keys. `load_state_dict()` does a strict key
match and fails.

**Fix:** Subclass `nn.Conv1d` directly in serving code:
```python
class CausalConv1d(nn.Conv1d):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
        self._causal_padding = (kernel_size - 1) * dilation

    def forward(self, x):
        return super().forward(F.pad(x, (self._causal_padding, 0)))
```

**Verification test pattern:**
```python
def test_tcn_state_dict_keys_match(self):
    sd = torch.load(str(TCN_PATH), map_location="cpu", weights_only=True)
    model = QqqTCN(in_dim=8, hidden_dim=64)
    assert set(model.state_dict().keys()) == set(sd.keys())
```

**Lesson:** When porting a PyTorch architecture, inspect the actual checkpoint
keys with `torch.load(path, weights_only=True).keys()` BEFORE writing the
serving model class. Match the module hierarchy exactly.

## 2. LightGBM Booster vs sklearn wrapper version mismatch

**Symptom:** `TypeError: 'NoneType' object is not callable` when calling
`LGBMRegressor.predict(X)`.

**Root cause:** `LGBMRegressor.predict()` internally calls
`_LGBMValidateData`, a function imported from scikit-learn utils. When the
installed scikit-learn version is newer than what the LightGBM wrapper expects,
`_LGBMValidateData` can be `None` (the import silently failed or the API was
removed). The pickle was trained in Colab with an older sklearn; the serving
environment has a newer one.

**Fix:** Extract the raw `Booster` and call it directly:
```python
def load_lgbm(pickle_path):
    with open(pickle_path, "rb") as f:
        model = pickle.load(f)
    booster = getattr(model, "booster_", None) or getattr(model, "_Booster", None)
    if booster is not None:
        return booster
    return model
```

The `Booster.predict()` API is part of LightGBM's C interface and is stable
across sklearn versions. It accepts a numpy array directly.

**Verification:**
```python
m = load_lgbm("models/qqq_lgbm_h1_v1.pkl")
row = np.zeros((1, 8), dtype=np.float64)
pred = m.predict(row)  # works — returns np.array([0.0598...])
```

## 3. dotenvy .env defeats Rust test env cleanup

**Symptom:** Config test `defaults_load_when_env_unset` fails:
```
left:  "/models/norm_stats_qqq_v1.json"   (from .env)
right: "models/norm_stats_qqq_v1.json"    (expected default)
```

**Root cause:** `Config::from_env()` calls `dotenvy::dotenv()` which re-loads
`.env` on every invocation. The test's `clear_engine_env()` calls
`env::remove_var("NORM_STATS_PATH")`, but `from_env()` immediately re-loads
`.env` and re-sets it. The `.env` file overrides the code default.

**Fix:** When you change a default path in `config.rs`, also update `.env` to
match. The `.env` is the source of truth for the test environment. Grep for
the old value:
```bash
grep -r "norm_stats.json" .env config.rs
```

**Cascade pitfall:** If one config test panics (e.g. `live_mode_falls_back_to_paper`
panics on missing parity marker), it poisons the `ENV_LOCK` mutex. All subsequent
config tests then fail with `PoisonError` — a cascade that looks like 12 failures
but is really 1 root cause. Confirm with `git stash` + re-run on clean main to
distinguish pre-existing from newly introduced.

## 4. Architecture: TCN state_dict key inspection

The trained `qqq_tcn_v1.pt` has these top-level key groups:
```
proj.weight [64, 8]          # input projection Linear(8→64)
proj.bias   [64]
blocks.{0..6}.conv1.*        # 7 ResidualBlocks, dilations [1,2,4,8,16,32,64]
blocks.{0..6}.conv2.*
blocks.{0..6}.norm1.*
blocks.{0..6}.norm2.*
heads.{0..2}.0.weight [32, 64]   # 3 horizon heads: Linear(64→32)→SiLU→Dropout→Linear(32→1)
heads.{0..2}.0.bias   [32]
heads.{0..2}.3.weight [1, 32]
heads.{0..2}.3.bias   [1]
```

Note: no `loss_weights` parameter in the exported checkpoint (training-only).
The serving model class must NOT include it or `load_state_dict(strict=True)`
will fail.

## 5. Wire protocol: V3 (equities) vs V2 (crypto)

| Field | V2 (crypto) | V3 (equities) |
|-------|-------------|---------------|
| schema_version | 2 | 3 |
| feature_window | [[f64;6]] | [[f64;8]] |
| Response | pred_1h, pred_4h, pred_24h | pred_1d, pred_5d, pred_21d |
| Feature dim | 6 (FEATURE_DIM) | 8 (EQ_FEATURE_DIM) |

The Rust `bridge.rs` has `predict_v3()` sending `schema_version: 3` with the
8-dim window. The Python `equity_model.py` `_handle_request()` validates
`n_features == 8` and returns `pred_1d/pred_5d/pred_21d`.

## 6. Dormant vs dead code: when pivoting, decide explicitly

When replacing an old model/pipeline with a new one (BTC hourly → QQQ daily,
Crypto V1 → V2, etc.), the codebase has THREE classes of old code, not two:

1. **Live** — actively called. Replace.
2. **Dormant** — kept alive for reactivation (e.g. if the new model fails
   the walk-forward IC gate, revert is one config flip away). PRESERVE.
3. **Dead** — declared but never called, no callers anywhere outside its own
   module. Delete.

**How to classify before deleting:**

```bash
# For each candidate module, find ALL cross-file references.
# A real ref looks like  `crate::foo::bar`  or  `use foo::bar`  — not just
# `pub mod foo;` (which is the declaration, not a use).
grep -rn "crate::features::crypto\|features::crypto::\|use crypto::" \
  --include="*.rs" --exclude-dir=target | grep -v graphify-out
```

If the ONLY hits are the `pub mod foo;` declaration line in `mod.rs`, it's
dead. If there are real call sites in `main.rs`, `parity.rs`, or test
fixtures, it's dormant — preserve it even if it's not on the hot path.

**MarketMoves-specific rule (USER-LOCKED):** the V2 BTC TCN + its entire
support chain (`features/legacy.rs`, `features/normalize.rs`, `parity.rs`,
`engine::tests::feature_parity.rs`) stays DORMANT — not deleted — because the
walk-forward IC gate can fail and a fallback to V2 is one config flip away.
Even though main.rs no longer invokes it on the Wave C hot path, the parity
marker + late-mode guard depend on `parity::write_marker`.

**Safe to delete in the Wave C cleanup (2026-07-23):**

- `features/crypto.rs` (82 lines) — only ref was `pub mod crypto;`
- `features/equities.rs` (26 lines) — `unimplemented!()` stub, never wired
- `features/volatility.rs` (165 lines) — only used by deleted `crypto.rs`
- `data/binance.rs` (270 lines) — called only by `data::backfill` /
  `data::run_ws_and_retention`, which themselves had zero main.rs callers

**Verification after deletion:** grep for `crate::features::crypto`,
`features::equities`, `features::volatility`, `data::binance`,
`RETENTION_CANDLES`, `run_ws_and_retention`, `data::staleness_secs` — all
must return 0 hits. If any remain, you missed a caller.

**Secondary surface cleanup:** after deleting a `pub mod foo;` declared but
unreferenced module, scan the parent `mod.rs` for dead **public API**
(constants, functions, trait impls) that only the deleted module used. For
the Wave C cleanup that also meant dropping `RETENTION_CANDLES`,
`data::backfill`, `data::staleness_secs`, `data::run_ws_and_retention` —
they were public entry points to deleted logic. Use `git blame` or the
`search_files pattern="RETENTION_CANDLES"` from CWD to confirm zero callers
before removing public API; private (non-pub) `fn`/`const`/dead structs
are safe to drop with no caller check.

## 7. Pre-existing test failure cascade — the `git stash` rule

When a test sweep reports many failures (e.g. 12 config tests failing), the
real cause is often ONE panicking test that poisons a shared mutex, not 12
distinct bugs. Distinguish:

```bash
# Save your changes, retest on clean main, then restore.
git stash
cargo test --lib config::tests::live_mode_falls_back_to_paper 2>&1 | tail
git stash pop
```

If the failing test ALSO fails on clean main, the cascade is pre-existing
and not introduced by your change. Report it as such; do NOT try to fix it
unless the user asks. The user's workflow is "don't fix things I didn't
ask for" — false-positive "regressions" introduced by unrelated pre-existing
bugs waste their time and erode trust.

The `PoisonError { .. }` is the signature: a test panic'd mid-flight,
holding a `Mutex` or `RwLock`, and every subsequent test fails on
`mutex.lock().unwrap()` with a poisoned-error panic. Look for the FIRST
failure (the one that produces a non-`PoisonError` message) — that's the
real bug.

Combined with the `dotenvy .env` pitfall (§3), the full diagnostic chain is:
1. Run tests, see many `PoisonError` failures.
2. Find the first non-`PoisonError` failure — that's the real one.
3. If it's a config test failing on `.env` overriding defaults, fix `.env`,
   not the test.
4. If it's a config test failing on missing parity marker, the cascade is
   pre-existing — `git stash` to confirm.

## 8. Removing dead `pub mod foo;` declarations

When a module is dead and you delete it, the `pub mod foo;` line in the
parent `mod.rs` now points at a non-existent file → `cargo` fails with
"file not found for module `foo`". Always grep the parent `mod.rs` for the
deleted module name and remove the `pub mod foo;` line as part of the same
commit. The patch tool will produce diffs you can scan before committing;
the cleanest approach is one patch per file: delete the .rs file with `rm`,
then patch `mod.rs` separately to drop the `pub mod` line.

Also strip dead **docstrings** that reference removed types/paths (e.g.
"(mirrors the old `run_ws_and_retention` blocking contract)" after
deleting `run_ws_and_retention`). Future agents grep for these names and
get false positives otherwise.

## 9. venv with no pip → `uv pip install` is the install path

The `.venv/bin/python3` may have `pip` deliberately removed (PEP 668
external-managed environment). `python3 -m pip install lightgbm` fails
with "No module named pip". `uv pip install lightgbm` (system `uv` CLI)
works — `uv` installs into the venv without needing the in-venv pip.
Always use `uv pip install <pkg>` rather than reaching for a system pip.

## 10. Config-change blast radius: `FEATURE_WINDOW_SIZE` spans 3+ files

When a single semantic default changes (e.g. `FEATURE_WINDOW_SIZE: 21 → 126`),
it typically touches multiple independent files. Always grep broadly before declaring
the change done:

```bash
grep -rn "FEATURE_WINDOW_SIZE\|feature_window\|126\|21" \
  engine/src/config.rs deploy/docker-compose.yml inference/tests/
```

Typical blast radius for a config default change:
- `engine/src/config.rs` — default value + test assertion
- `deploy/docker-compose.yml` — Docker env var default
- `inference/tests/test_equity_model.py` — hardcoded window sizes in tests
- `.env.example` — if it documents engine defaults (optional)

All must be consistent. A missed file means tests pass but deployed behavior differs.

## 11. Z-score blending requires a prediction history buffer

The Colab notebook uses z-score blending per horizon:
```python
t_z = (tcn - rolling_mean[h]) / rolling_std[h]
l_z = (lgbm - rolling_mean[h]) / rolling_std[h]
blend_z = (t_z + l_z) / 2
```
This requires rolling mean/std statistics built from prediction history.
At inference (single-shot, no history), z-scoring a single scalar → 0
(no stddev). The correct single-shot fallback is **weighted raw average**
(both models produce label-space values, so they share units).

**Never** apply per-scalar z-scoring in a stateless inference service.
Only add z-scoring if a rolling prediction buffer is maintained.

## 12. ATR normalization: Rust → Python inference pipeline

QQQ labels = `clip(fut_ret / (atr/close), -3, 3)`. The TCN and LightGBM
are trained on label-space values. The strategy thresholds (entry=0.003,
exit=-0.001) are in raw log-return units. The pipeline denormalizes at inference:

1. `engine/src/scheduler.rs` — `compute_atr_ratio(candles)` computes
   `ATR(14) / close` using Wilder's EMA (same as Colab notebook):
   - TR[i] = max(H-L, |H-prev_close|, |L-prev_close|) for i >= 1
   - ATR_warmup = mean(TR[1:14])   (simple average over first 14)
   - ATR[i] = (ATR[i-1] * 13 + TR[i]) / 14  for i > 14
   - `atr_ratio = ATR / close`
2. Bridge sends `atr_ratio` in V3 ZMQ payload.
3. `inference/equity_model.py` — `EquityEnsemble.predict`:
   ```
   label = w_t * tcn_out[h] + w_l * lgbm_out[h]   # label-space blend
   raw_return = label * atr_ratio                      # denormalize
   ```

Rust ATR formula must match Colab's exactly — any divergence corrupts denorm.

## 13. Synthetic test data produces zero outputs from normalized NNet

**Symptom:** tests using constant feature windows like `[[0.0]*8 for _ in 126]`
or `[[1.0]*8 for _ in 126]` produce `pred = 0.0` regardless of blend weights
or atr_ratio changes.

**Root cause:** The QQQ TCN was trained on QQQ-normalized features
(RobustScaler: `(x - median) / MAD`). A constant synthetic window is massively
out-of-distribution — the model's final-layer activations collapse to near zero.
Neural nets on OOD inputs produce arbitrarily small outputs; this is expected,
not a bug in the inference code.

**Fix for arithmetic/blend tests:** mock the model outputs with known values.
```python
class MockTCN:
    def __call__(self, x):
        return [torch.tensor([[v]], dtype=torch.float32) for v in [0.6, 0.2, 0.1]]
class MockLGBM:
    def __init__(self, value): self.value = value
    def predict(self, x): return [self.value]

ensemble = EquityEnsemble(MockTCN(), MockLGBM(0.4), MockLGBM(0.0), MockLGBM(-0.1))
preds = ensemble.predict(window=[[0.0]*8]*126, atr_ratio=0.005)
assert np.isclose(preds["pred_1d"], 0.0025)  # 0.5 * 0.005
```

**Fix for integration tests** (ZMQ round-trip, E2E with real artifacts):
use varied feature windows — `[[(i+j)*0.1 for j in range(8)] for i in range(126)]` —
which produces non-trivial outputs even with real models.

## 14. state_dict keys match but forward pass differs (SILENT skew — 2026-07-26)

**Symptom:** `load_state_dict(strict=True)` succeeds. The inference service
runs without error. But predictions are wrong because the training and serving
architectures compute different forward passes on the same weights.

**Context:** The Colab training notebook (`EquitiesTCN`) and the inference
service (`QqqTCN` in `equity_model.py`) have the same parameter names and
shapes (70 entries, all matching), but THREE structural differences in the
forward pass:

### 14a. Conv padding: symmetric vs causal

| Side | Conv layer | Padding |
|------|-----------|---------|
| Training | `nn.Conv1d(kernel_size=3, padding=dilation, dilation=dilation)` | Symmetric (both sides) |
| Inference | `CausalConv1d(kernel_size=3, dilation=dilation)` | Left-only (causal) |

Symmetric padding lets the convolution see future timesteps; causal padding
does not. The weights were trained with symmetric padding (future leakage in
training, but that's what the model learned). Feeding them into a causal conv
produces different activations.

### 14b. Residual block activation order

| Side | Forward pass |
|------|-------------|
| Training | `x = dropout(silu(norm1(conv1(x))))` → `x = dropout(silu(norm2(conv2(x))))` → `return x + res` (NO final activation on sum) |
| Inference | `out = activation(dropout(conv2(activation(dropout(norm1(conv1(x))))))` → `return activation(out + residual)` (final SiLU ON the sum) |

The training code does NOT apply SiLU after the residual addition. The
inference code DOES. This changes the activation distribution at every block
output.

### 14c. Block container: ModuleList vs Sequential

| Side | Container | Forward |
|------|-----------|---------|
| Training | `nn.ModuleList` | Manual loop: `for block in blocks: x = block(x)[:,:,:-x.size(2)] + x` |
| Inference | `nn.Sequential` | `feat = self.blocks(x)[:,:,-1]` |

Both produce `blocks.0.conv1.weight` keys, so `load_state_dict` succeeds. But
the training loop has an explicit length-alignment slice `[:,:,:-x.size(2)]`
that the Sequential forward does not replicate.

### 14d. Output shape

| Side | Output |
|------|--------|
| Training | `torch.cat([head(out) for head in heads], dim=1)` → `(batch, 3)` |
| Inference | `[head(feat).squeeze(-1) for head in heads]` → list of 3 × `(batch,)` |

The training code returns a concatenated tensor; the inference code returns a
list. This doesn't affect the weights but affects how the caller processes
output.

### Detection: forward-pass parity test

```python
def test_tcn_forward_pass_parity():
    """Load the same checkpoint into both architectures, feed the same
    input, assert outputs are bit-close."""
    import torch
    checkpoint = torch.load(TCN_PATH, map_location="cpu", weights_only=True)

    # Training architecture
    train_model = EquitiesTCN(in_dim=8)
    train_model.load_state_dict(checkpoint)
    train_model.eval()

    # Serving architecture
    serve_model = QqqTCN(in_dim=8)
    serve_model.load_state_dict(checkpoint)
    serve_model.eval()

    x = torch.randn(1, 126, 8)  # fixed seed for reproducibility
    with torch.no_grad():
        train_out = train_model(x)      # (1, 3)
        serve_out = serve_model(x)      # list of 3 × (1,)

    for h in range(3):
        t_val = train_out[0, h].item()
        s_val = serve_out[h].item()
        assert abs(t_val - s_val) < 1e-5, \
            f"Horizon {h}: train={t_val}, serve={s_val} — forward pass mismatch"
```

If this test fails, the serving architecture must be aligned to match training
EXACTLY. The weights were trained for the training forward pass; the serving
forward pass must replicate it, not just match parameter names.

### Lesson

`load_state_dict(strict=True)` is necessary but NOT sufficient for train/serve
parity. It proves parameter names and shapes match. It does NOT prove the
forward pass is identical. Always add a forward-pass parity test that feeds the
same input through both architectures and asserts bit-close outputs.
