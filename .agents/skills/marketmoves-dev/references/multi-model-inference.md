# Multi-Model Inference Constraint

**Scope:** engine + inference container interaction after the §8 multi-model registry.

**Status: RESOLVED (2026-08-07).** The inference service now supports per-symbol
model loading and z-score blending. See `references/z-score-blending.md` for
the blending architecture.

## Current state (post-fix)

The `inference` microservice (`inference/equity_model.py`) loads **all** model
ensembles at startup by scanning `MODELS_DIR` for subdirectories containing
`*tcn*.pt` files. The flat legacy layout (QQQ models directly in `/models/`)
is loaded as the default `"QQQ"` ensemble. Each ensemble is keyed by symbol.

The engine sends `{"symbol": "QQQ"}` or `{"symbol": "NVDA"}` in the V3 ZMQ
request. The inference service routes to the correct per-symbol ensemble.

Each ensemble maintains per-horizon prediction buffers (252 slots) for z-score
blending, matching the Colab notebook's walk-forward evaluation pipeline.

## Engine changes required

1. `bridge.rs`: `predict_v3` and `predict_v3_with_retry` accept a `symbol: &str`
   parameter, included in the JSON payload as `"symbol"`.
2. `scheduler.rs`: passes `self.symbol` to the bridge call.
3. `api/equity.rs`: backtest replay also passes `state.symbol`.

## Inference service changes

1. `_handle_request`: extracts `symbol` from request, looks up ensemble dict.
   Falls back to first available ensemble for legacy clients.
2. `main()`: scans `MODELS_DIR` for per-symbol subdirectories (e.g. `/models/NVDA/`),
   loads each bundle, then loads legacy flat layout as `"QQQ"`.
3. `EquityEnsemble`: maintains per-horizon `deque` buffers, z-scores TCN and
   LGBM predictions against their own history before blending.

## Deploy config

```yaml
# docker-compose.yml — inference service
environment:
  MODELS_DIR: /models   # REQUIRED — enables directory scanning
  TCN_PATH: ...         # still used for legacy flat layout fallback
  ZMQ_BIND: tcp://0.0.0.0:5555
```

## Verification

```bash
# Check which models were loaded
docker logs mmn-inference 2>&1 | grep -E "loaded model bundle|equity inference configured"
# Should show: "loaded model bundle for symbol=NVDA" and "loaded default QQQ ensemble"
# Should show: "equity inference configured: 2 ensembles"

# Check per-symbol predictions
curl -s "http://localhost:9080/api/status?model_id=nvda-v1&symbol=NVDA" | python3 -m json.tool
curl -s "http://localhost:9080/api/accuracy?symbol=NVDA" | python3 -m json.tool
```

## Buffer warmup

The first 2 predictions for each model will be 0.0 because the z-score
buffers need ≥2 elements to compute mean/std. After the buffer fills (252
trading days ≈ 1 year), predictions match the notebook's walk-forward
evaluation.
