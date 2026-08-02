# Environment variables

Engine and inference configuration. `.env` is gitignored — use `.env.example`
as the template. The compose file's `env_file: ../.env` resolves to the
workspace root.

## Engine (Rust)

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `TRADING_MODE` | yes | `paper` | `paper` or `live`. Live is blocked until the parity gate is satisfied (see [PARITY gate](../README.md#parity-gate)). |
| `SYMBOL` | no | `QQQ` | Primary trade target (equity ticker). |
| `SHORT_SYMBOL` | no | `PSQ` | Inverse-ETF used for short positions (ProShares Short QQQ). |
| `SHORT_ENTRY_THRESHOLD` | no | `-0.004` | Short entry threshold (pred_1d below this → Short). |
| `SHORT_EXIT_THRESHOLD` | no | `0.001` | Short exit threshold (pred_1d above this → Flat). |
| `ENABLE_SHORTING` | no | `false` | Master switch for shorting in the equities strategy. |
| `ZMQ_ENDPOINT` | yes | `tcp://inference:5555` | Inference socket. |
| `MAGNITUDE_THRESHOLD` | no | `0.005` | Long entry threshold: pred_1d must exceed this to go long. Exit threshold is `MAGNITUDE_THRESHOLD / 3` (negative). |
| `PAPER_FEE` | no | `0.0015` | Simulated fee rate (paper mode). |
| `SMA_WINDOW` | no | `200` | Regime SMA window (200 = SMA200). |
| `HTTP_PORT` | no | `8080` | Axum server port (internal only). |
| `DATABASE_URL` | no | `sqlite:///app/data/candles.db` | SQLite DB path. |
| `NORM_STATS_PATH` | no | `/models/norm_stats_qqq_v1.json` | Equity norm stats file. |
| `FEATURE_WINDOW_SIZE` | no | `126` | Feature lookback window for the TCN. |
| `PARITY_MARKER_PATH` | no | `/app/data/parity_verified.json` | Parity verification marker path. |
| `PARITY_MAX_AGE_SECS` | no | `604800` | Max age of parity marker before live is blocked (7 days). |
| `RUST_LOG` | no | `info` | Tracing filter. |
| `LIVE_EXECUTOR` | no | `paper` | `paper` (default, safe fallback) or `moomoo` (wires the MoomooExecutor). |
| `MOOMOO_TRD_ENV` | no | `SIMULATE` | `SIMULATE` or `REAL`. Only consulted when `LIVE_EXECUTOR=moomoo`. |
| `MOOMOO_CREDS_PATH` | no | `~/.moomoo/credentials.json` | Moomoo OpenAPI credentials JSON. |
| `MOOMOO_SECURITY_FIRM` | no | unset | Moomoo security firm (e.g. `FUTUSECURITIES`). |
| `MOOMOO_ACC_ID` | no | unset | Account ID override (else the first available is picked). |
| `FRED_API_KEY` | no | empty | FRED API key (free — get at https://fred.stlouisfed.org/apikey). Required for treasury yields + DXY via JSON API v2. |
| `FUTU_OPEND_HOST` | no | `127.0.0.1` | Moomoo OpenD gateway host. Use `host.docker.internal` inside Docker. |
| `FUTU_OPEND_PORT` | no | `11111` | Moomoo OpenD TCP port. |
| `TOTP_SECRET` | no | empty | Base32 TOTP secret for `/api/mode`. Empty → engine mints a fresh one and logs an `otpauth://` URL. |

## Inference (Python)

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `TCN_PATH` | yes | `/models/qqq_tcn_v1.pt` | TCN PyTorch checkpoint. |
| `LGBM_H1_PATH` | yes | `/models/qqq_lgbm_h1_v1.pkl` | LGBM horizon-1 model. |
| `LGBM_H5_PATH` | yes | `/models/qqq_lgbm_h5_v1.pkl` | LGBM horizon-5 model. |
| `LGBM_H21_PATH` | yes | `/models/qqq_lgbm_h21_v1.pkl` | LGBM horizon-21 model. |
| `ZMQ_BIND` | no | `tcp://0.0.0.0:5555` | ZMQ REP bind endpoint. |
| `PYTHONUNBUFFERED` | no | `1` | Disable stdout buffering. |

## Compose / infra

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `HOST` | no | unset | Public hostname for Let's Encrypt (leave unset for local/HTTP). |
| `COMPOSE_PROJECT_NAME` | no | `marketmoves` | Compose project name. |

## Notes

- `.env` is gitignored. Use `.env.example` as a template.
- `TRADING_MODE=live` is blocked by the parity gate at startup AND at
  `POST /api/mode` request time. The runtime endpoint re-checks the
  marker freshness every flip.
- The runtime mode toggle is wired but currently dormant. The engine
  still ships in `paper` mode by default. Going live is a deliberate
  7-step process — see [`deploy/README.md#going-live`](./README.md#going-live).
- Even with `TRADING_MODE=live`, the engine falls back to `PaperExecutor`
  when `LIVE_EXECUTOR=paper` (the default). To wire the Moomoo executor
  you must also set `LIVE_EXECUTOR=moomoo` AND provision Moomoo OpenD
  credentials on the host. See `deploy/README.md#going-live` step 3.
- **Data source routing (equities):** Moomoo OpenD first, Yahoo Finance fallback.
  The engine auto-detects OpenD reachability at startup. Without OpenD it silently
  falls back to Yahoo for equity OHLCV and live quotes with no degradation.
- **VIX:** Sourced from CBOE (cdn.cboe.com — free, no auth, no rate limits).
  Yahoo `^VIX` is the automatic fallback.
- **Treasury yields + DXY:** FRED JSON API v2 (`api.stlouisfed.org`).
  Requires `FRED_API_KEY` (free, 120 req/min). Fails gracefully to 0.0 without key.
- **Market sentiment:** Phase 1 stub (always 0.5 neutral). Phase 2 wires Finnhub
  `news_sentiment` (free tier 60 req/min). The `sentiment_cache` DB table is
  ready now — the feature pipeline reads it regardless.
