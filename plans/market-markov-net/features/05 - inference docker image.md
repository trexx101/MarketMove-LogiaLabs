# Feature 05 — Inference Docker Image

**Depends on:** 04
**Goal:** Package the Python inference service into a reproducible CPU Docker image.

## Requirements

- CPU-only PyTorch build (no CUDA).
- Pinned dependencies for reproducibility.
- Model artifacts volume-mounted from `/models` (not baked, since gitignored/large).
- Exposes 5555 only on the internal compose network.

## Technical Implementation Steps

1. `inference/Dockerfile`: base `python:3.11-slim`, install CPU torch wheel (`--index-url` for cpu), `pyzmq`, `numpy`.
2. Copy source; set entrypoint `python inference_engine.py`.
3. Mount `/models` as a read-only volume; read model path from env.
4. Add a container-level healthcheck (lightweight ZMQ ping or process check).

## Acceptance Criteria

- [ ] `docker build` succeeds and image is CPU-only.
- [ ] Container runs and serves REP on the internal network.
- [ ] Model loads from mounted volume; missing model fails fast with a clear log.
- [ ] Port 5555 is not published to the host.
