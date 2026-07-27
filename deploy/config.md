# Environment variables

## Engine (Rust)

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `TRADING_MODE` | yes | `paper` | `paper` or `live`. Live is blocked until the parity gate is satisfied. |
| `SYMBOL` | no | `QQQ` | Trade target (equity ticker). |
| `ZMQ_ENDPOINT` | yes | `tcp://inference:5555` | Inference socket. |
| `MAGNITUDE_THRESHOLD` | no | `0.005` | Entry threshold: pred_1d must exceed this to go long. |
| `EXIT_THRESHOLD` | no | `-0.001` | Exit threshold: pred_1d below this triggers flat. |
| `PAPER_FEE` | no | `0.0015` | Simulated fee rate (paper mode). |
| `SMA_WINDOW` | no | `200` | Regime SMA window (200 = SMA200). |
| `HTTP_PORT` | no | `8080` | Axum server port. |
| `DATABASE_URL` | no | `sqlite:///app/data/candles.db` | SQLite DB path. |
| `NORM_STATS_PATH` | no | `/models/norm_stats_qqq_v1.json` | Equity norm stats file. |
| `FEATURE_WINDOW_SIZE` | no | `126` | Feature lookback window for the TCN. |
| `PARITY_MARKER_PATH` | no | `/app/data/parity_verified.json` | Parity verification marker path. |
| `PARITY_MAX_AGE_SECS` | no | `604800` | Max age of parity marker before live is blocked (7 days). |
| `RUST_LOG` | no | `info` | Tracing filter. |

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
- `TRADING_MODE=live` is blocked by the parity gate (parity marker must exist
  and be fresh within `PARITY_MAX_AGE_SECS`).
- Live execution against Moomoo is not yet wired — live mode still falls back
  to paper. Do not set `TRADING_MODE=live` until the execution layer is
  connected.
