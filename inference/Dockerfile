# syntax=docker/dockerfile:1.7
#
# MarketMarkovNet inference microservice — CPU-only image.
#
# Build:    docker build -t marketmarkovnet/inference inference/
# Run:      docker run --rm \
#               -v /models:/models:ro \
#               -e ZMQ_BIND=tcp://0.0.0.0:5555 \
#               marketmarkovnet/inference
#
# Notes
# -----
# - The image is CPU-only (CUDA wheels are not installed).
# - /models is expected to be a read-only volume containing model.pt and
#   norm_stats.json; the entrypoint will fail fast with a clear log if either
#   file is missing (see inference.config.InferenceConfig.require_artifacts).
# - Port 5555 is documented via EXPOSE but NEVER published to the host —
#   production deployments must place this container on an internal
#   docker network and let only the Rust engine (engine service) reach it.
# - Healthcheck performs a full ZMQ REQ/REP round-trip with a minimal payload
#   to confirm both the socket and the model are responsive.

FROM python:3.11-slim AS base

# ── OS basics ────────────────────────────────────────────────────────────────
# curl is needed for compose-level network waits; ca-certificates keeps
# HTTPS for the PyTorch CPU wheel index working.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# ── Working directory ────────────────────────────────────────────────────────
WORKDIR /app

# ── Pinned dependencies (installed before copying source for layer caching) ─
# torch 2.5.1 is the most recent stable that ships a CPU-only wheel on the
# official PyTorch index at the time of writing; numpy 1.26.4 is the last
# 1.x release that is still fully supported by torch 2.x and avoids the
# copy-on-write behaviour changes introduced in numpy 2.x.
ENV PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

RUN pip install --no-cache-dir \
        --index-url https://download.pytorch.org/whl/cpu \
        torch==2.5.1+cpu

RUN pip install --no-cache-dir \
        numpy==1.26.4 \
        pyzmq==26.0.3 \
        lightgbm>=4.0

# ── Application source ───────────────────────────────────────────────────────
# The build context is this directory, so the package files live at the root
# of the context (./__init__.py, ./inference_engine.py, etc.). Models, target/,
# frontend/, etc. are explicitly excluded via .dockerignore.
COPY __init__.py          /app/inference/__init__.py
COPY config.py            /app/inference/config.py
COPY model.py             /app/inference/model.py
COPY equity_model.py      /app/inference/equity_model.py
COPY inference_engine.py  /app/inference/inference_engine.py

# ── Runtime defaults ─────────────────────────────────────────────────────────
# The mount point matches deploy/config.md and the InferenceConfig defaults.
# Operators can override at `docker run -e ...` or via compose.
ENV TCN_PATH=/models/qqq_tcn_v1.pt \
    LGBM_H1_PATH=/models/qqq_lgbm_h1_v1.pkl \
    LGBM_H5_PATH=/models/qqq_lgbm_h5_v1.pkl \
    LGBM_H21_PATH=/models/qqq_lgbm_h21_v1.pkl \
    ZMQ_BIND=tcp://0.0.0.0:5555

# Compose deploys this on an internal network — never publish to the host.
# The line below is documentation only; it does not publish the port.
EXPOSE 5555

# ── Healthcheck ──────────────────────────────────────────────────────────────
# A real REQ/REP round-trip exercises both the ZMQ socket and the model
# graph (so a stuck-forward or partially-loaded container is detected).
# We send a minimal (1×1 window) feature request and require a 3-key reply.
HEALTHCHECK --interval=30s --timeout=20s --start-period=60s --retries=3 \
    CMD python -c "import json, zmq; \
ctx=zmq.Context.instance(); \
s=ctx.socket(zmq.REQ); \
s.setsockopt(zmq.LINGER, 0); \
s.setsockopt(zmq.RCVTIMEO, 15000); \
s.setsockopt(zmq.SNDTIMEO, 15000); \
s.connect('tcp://127.0.0.1:5555'); \
s.send(json.dumps({'schema_version': 3, 'feature_window': [[0.0]*8]}).encode()); \
reply=json.loads(s.recv()); \
assert all(k in reply for k in ('pred_1d', 'pred_5d', 'pred_21d')), reply; \
s.close()" || exit 1

# ── Entrypoint ────────────────────────────────────────────────────────────────
ENTRYPOINT ["python", "-u", "-m", "inference.equity_model"]
