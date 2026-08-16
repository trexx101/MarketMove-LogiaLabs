# syntax=docker/dockerfile:1.7
#
# MarketMoves engine — Rust/Axum binary, Svelte SPA, CPU-only runtime.
#
# Build context MUST be the workspace root (the directory that contains both
# `engine/` and `frontend/`). The engine's `ServeDir::new("frontend")` serves
# the static Svelte build at runtime, so the build stage must produce a
# compiled `frontend/dist/` (index.html + assets/) — NOT the Svelte source.
#
# Build:
#   docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
#
# Run (paper mode, no live keys):
#   docker run --rm \
#     -v mmn_data:/app/data \
#     -v /models:/models:ro \
#     -p 8080:8080 \
#     --env-file .env \
#     marketmarkovnet/engine:latest

# ── Frontend build stage ─────────────────────────────────────────────────────
# The Svelte SPA has already been built (npm run build) on the host and the
# dist/ directory is volume-mounted or copied into the Docker build context.
# We COPY it directly — no Node toolchain needed in the runtime image.
FROM debian:bookworm-slim AS frontend-build

WORKDIR /build/frontend
COPY frontend/dist/ ./

# ── Engine build stage ───────────────────────────────────────────────────────
# Pinned to a stable Debian bookworm toolchain to keep glibc / openssl
# versions consistent with the runtime image.
FROM rust:1-bookworm AS build

WORKDIR /build

# Build tools needed to compile sqlite3-sys (which builds libsqlite3 from source
# when a system libsqlite3-dev is not pre-installed in the image).
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

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

# Real source: copy just the engine crate (the static frontend is provided
# by the frontend-build stage above).
COPY engine/ engine/

# Build the actual binary.
# Touch main.rs to force cargo to detect the source change from the placeholder.
RUN touch engine/src/main.rs \
    && cargo build --release --manifest-path engine/Cargo.toml \
    && strip target/release/engine \
    && cp target/release/engine /usr/local/bin/engine

# ── Runtime stage ───────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Runtime deps:
#   - ca-certificates: outbound HTTPS to Yahoo/FRED/Moomoo + Caddy's CA chain
#   - curl: healthcheck
#   - tini: PID-1 that reaps zombies and forwards signals (Docker stop = SIGTERM)
#   - libsqlite3: defensive fallback (sqlx's bundled-sqlite is the default)
#   - libssl3: openssl backend for reqwest's native-tls (used for FRED/Yahoo;
#              rustls-tls hangs the Akamai edge in this runtime image).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        tini \
        libsqlite3-0 \
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Non-root user.
RUN groupadd --system --gid 1000 mmn \
    && useradd --system --uid 1000 --gid mmn --home /app --shell /usr/sbin/nologin mmn

WORKDIR /app
RUN mkdir -p /app/data && chown -R mmn:mmn /app

# Binary from the engine build stage.
COPY --from=build /usr/local/bin/engine /usr/local/bin/engine

# Pre-built Svelte bundle: copied directly from build context (already built).
# No Node toolchain needed in the runtime image.
COPY --from=frontend-build --chown=mmn:mmn /build/frontend/ /app/frontend/

# Drop privileges.
USER mmn

# Internal: HTTP telemetry / SPA.
EXPOSE 8080

# Env defaults — the operator / compose file overrides most of these.
ENV HTTP_PORT=8080 \
    DATABASE_URL=sqlite:///app/data/candles.db \
    NORM_STATS_PATH=/models/norm_stats_qqq_v1.json \
    PARITY_MARKER_PATH=/app/data/parity_verified.json \
    RUST_LOG=info \
    SMA_WINDOW=40 \
    FEATURE_WINDOW_SIZE=126 \
    ENTRY_THRESHOLD=0.008 \
    EXIT_THRESHOLD=-0.003 \
    PRED_5D_FILTER=false \
    SHORT_PRED_5D_FILTER=false \
    MAGNITUDE_THRESHOLD=0.005 \
    PAPER_FEE=0.0015 \
    ZMQ_ENDPOINT=tcp://inference:5555 \
    SYMBOL=QQQ \
    SHORT_SYMBOL=PSQ \
    TRADING_MODE=paper \
    LIVE_EXECUTOR=paper \
    MOOMOO_TRD_ENV=SIMULATE \
    ENABLE_SHORTING=true \
    SHORT_ENTRY_THRESHOLD=-0.008 \
    SHORT_EXIT_THRESHOLD=0.003 \
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
