# inference/

MarketMarkovNet inference microservice (Python, PyTorch CPU, ZMQ REP).

## Status

**Feature 04 + 05 shipped.** The microservice:

- defines `MarketMarkovNet` (causal CNN backbone + parallel draft heads +
  low-rank Markov heads) matching the Colab reference,
- binds a ZMQ `REP` socket on the address given by `ZMQ_BIND`,
- serves the JSON contract
  `{ "feature_window": [[...], ...] }` → `{ "pred_1h", "pred_4h", "pred_24h" }`,
- logs every request as a single JSON line to stdout for parity auditing,
- ships as a CPU-only Docker image with a ZMQ-based `HEALTHCHECK`.

The Rust engine builds a normalized feature vector at each hourly candle
boundary and sends a ZMQ `REQ` to this service. We respond with a JSON payload:

```json
{ "pred_1h": ..., "pred_4h": ..., "pred_24h": ... }
```

Model artifacts (`model.pt`, `norm_stats.json`) are mounted from
`/models/` (gitignored, user-supplied).

## Local dev (uv)

```bash
cd inference
uv sync
uv run python -m inference.equity_model
```

Set env vars or use defaults:
```bash
# Defaults point at models/ in the project root
TCN_PATH=models/qqq_tcn_v1.pt \
LGBM_H1_PATH=models/qqq_lgbm_h1_v1.pkl \
LGBM_H5_PATH=models/qqq_lgbm_h5_v1.pkl \
LGBM_H21_PATH=models/qqq_lgbm_h21_v1.pkl \
uv run python -m inference.equity_model
```

## Docker

The image is built from this directory and is **CPU-only** (no CUDA wheels):

```bash
docker build -t marketmarkovnet/inference inference/
```
docker run --rm \
    -v /models:/models:ro \
    -e ZMQ_BIND=tcp://0.0.0.0:5555 \
    marketmarkovnet/inference
```

Notes on production deployment:

- `/models` is mounted **read-only** (`-v /models:/models:ro`).
- Port 5555 is **never published to the host**; compose places the
  container on an internal network and only the `engine` service reaches it.
- The image includes a `HEALTHCHECK` that performs a real REQ/REP
  round-trip with a minimal payload, so a stuck-forward or
  partially-loaded container is detected within 30 s.
- Missing `model.pt` / `norm_stats.json` cause the entrypoint to exit
  with a clear log line on stderr (see `InferenceConfig.require_artifacts`).
