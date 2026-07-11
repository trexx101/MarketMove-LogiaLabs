# inference/

MarketMarkovNet inference microservice (Python, PyTorch CPU, ZMQ REP).

## Status

**Feature 04 placeholder.** The full model + REQ/REP loop lands in Feature 04
(see `../plans/market-markov-net/features/04 - python inference microservice.md`).

This directory currently ships:

- `pyproject.toml` — project metadata + dependency list (torch, pyzmq, numpy).
- `.python-version` — pins Python 3.11.
- `inference_engine.py` — `main()` stub.
- `__init__.py` — empty package marker.

## Run (placeholder)

```bash
cd inference
uv sync
uv run python inference_engine.py
```

## Role in the system

The Rust engine builds a normalized feature vector at each hourly candle
boundary and sends a ZMQ `REQ` to this service. We respond with a JSON payload:

```json
{ "pred_1h": ..., "pred_4h": ..., "pred_24h": ... }
```

Model artifacts (`model.pt`, `norm_stats.json`) are mounted from
`../models/` (gitignored, user-supplied).
