# syntax=docker/dockerfile:1.7
#
# MarketMarkovNet engine — Rust/Axum binary, CPU-only runtime.
#
# Build context MUST be the workspace root (the directory that contains both
# `engine/` and `frontend/`). The `frontend/` directory is served by Axum at
# runtime, so it has to be present in the runtime image at the same relative
# path the binary expects (relative to the binary's CWD, which we set to /app).
#
# Build:
#   docker build -f engine/Dockerfile -t marketmarkovnet/engine .
#
# Run (paper mode, no live keys):
#   docker run --rm \
#     -v mmn_data:/app/data \
#     -v /models:/models:ro \
#     -p 8080:8080 \
#     --env-file .env \
#     marketmarkovnet/engine

# ── Build stage ──────────────────────────────────────────────────────────────
# Pinned to a stable Debian bookworm toolchain to keep glibc / openssl
# versions consistent with the runtime image.
FROM rust:1-bookworm AS build

WORKDIR /build

# Cache deps: copy manifests only first.
COPY Cargo.toml Cargo.lock ./
COPY engine/Cargo.toml engine/

# `cargo fetch` would require a stub lib; instead pre-build a tiny `cargo
# build` that resolves the dep graph so the next layer is fully cached.
# We use a sentinel `src/main.rs` to drive the resolver; the real source
# is copied in the next layer.
RUN mkdir -p engine/src \
    && echo 'fn main() { println!("deps-warmup"); }' > engine/src/main.rs \
    && echo '' > engine/src/lib.rs \
    && cargo build --release --manifest-path engine/Cargo.toml \
    && rm -rf engine/src

# Real source: copy the engine crate and the static frontend.
COPY engine/ engine/
COPY frontend/ frontend/

# Build the actual binary.
# Touch main.rs to force cargo to detect the source change from the placeholder.
RUN touch engine/src/main.rs \
    && cargo build --release --manifest-path engine/Cargo.toml \
    && strip target/release/engine \
    && cp target/release/engine /usr/local/bin/engine

# ── Runtime stage ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Runtime deps:
#   - ca-certificates: TLS to wss://ws.kraken.com + outbound HTTPS
#   - curl: healthcheck
#   - tini: PID-1 that reaps zombies and forwards signals (Docker stop = SIGTERM)
#   - libsqlite3: sqlx's bundled-sqlite is off, so we link against system libsqlite3
#                 (sqlx::sqlite is statically linked by default; libsqlite3 is here
#                  as a defensive fallback in case any future dep needs it).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        tini \
        libsqlite3-0 \
        # libssl3: openssl backend for reqwest's native-tls (used for FRED/Yahoo;
        #          rustls-tls hangs the Akamai edge in this runtime image).
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Non-root user.
RUN groupadd --system --gid 1000 mmn \
    && useradd --system --uid 1000 --gid mmn --home /app --shell /usr/sbin/nologin mmn

WORKDIR /app
RUN mkdir -p /app/data && chown -R mmn:mmn /app

# Binary from the build stage.
COPY --from=build /usr/local/bin/engine /usr/local/bin/engine

# Static frontend, served by the engine's `ServeDir` fallback.
# Fix ownership so the mmn user can traverse/execute all parent dirs.
COPY --from=build --chown=mmn:mmn /build/frontend/ /app/frontend/

# Drop privileges.
USER mmn

# Internal: HTTP telemetry / SPA.
EXPOSE 8080

# Env defaults — the operator / compose file overrides most of these.
ENV HTTP_PORT=8080 \
    DATABASE_URL=sqlite:///app/data/candles.db \
    NORM_STATS_PATH=/models/norm_stats.json \
    PARITY_MARKER_PATH=/app/data/parity_verified.json \
    RUST_LOG=info \
    SMA_WINDOW=200 \
    FEATURE_WINDOW_SIZE=72 \
    MAGNITUDE_THRESHOLD=0.005 \
    PAPER_FEE=0.0015 \
    ZMQ_ENDPOINT=tcp://inference:5555 \
    SYMBOL=BTC/USD \
    TRADING_MODE=paper \
    PYTHONUNBUFFERED=1

# Persistent SQLite + parity marker live under /app/data.
# Compose mounts a named volume here.
VOLUME ["/app/data"]

# tini is PID 1 — it forwards SIGTERM and reaps zombies so the engine can
# shut down cleanly when `docker compose stop` is called.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/engine"]

# ── Healthcheck ──────────────────────────────────────────────────────────────
# The engine exposes GET /api/status. Compose's healthcheck gates the
# `proxy` service on this so Caddy never starts before the engine is
# actually serving HTTP.
HEALTHCHECK --interval=15s --timeout=5s --start-period=20s --retries=3 \
    CMD curl --silent --fail --max-time 4 http://127.0.0.1:8080/api/status \
        | grep -q '"mode":"paper"\|"mode":"live"' \
        || exit 1
