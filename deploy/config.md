# Environment variables

Authoritative source: `../plans/market-markov-net/REQUIREMENTS.md`.

## Engine (Rust)

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `KRAKEN_API_KEY` | live mode only | — | Kraken key. **Query + Trade only. Withdraw disabled.** |
| `KRAKEN_API_SECRET` | live mode only | — | Kraken secret. |
| `TRADING_MODE` | yes | `paper` | `paper` or `live`. Live is blocked until the parity gate is satisfied. |
| `ZMQ_ENDPOINT` | yes | `tcp://127.0.0.1:5555` | Inference socket. In Compose this points at the `inference` service. |
| `MAGNITUDE_THRESHOLD` | no | `0.005` | Signal threshold (z-score magnitude). |
| `PAPER_FEE` | no | `0.0015` | Simulated fee rate (paper mode). |
| `SMA_WINDOW` | no | `200` | Regime SMA window (hours). |
| `HTTP_PORT` | no | `8080` | Axum server port. |
| `SYMBOL` | no | `BTC/USD` | Trading pair. |
| `RUST_LOG` | no | `info` | Tracing filter. |

## Inference (Python)

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `MODEL_PATH` | no | `/models/model.pt` | Path to PyTorch model checkpoint. |
| `NORM_STATS_PATH` | no | `/models/norm_stats.json` | Path to normalization stats. |
| `ZMQ_BIND` | no | `tcp://*:5555` | ZMQ REP bind endpoint. |
| `PYTHONUNBUFFERED` | no | `1` | Disable stdout buffering. |

## Compose / infra

| Var | Required | Default | Purpose |
|-----|----------|---------|---------|
| `COMPOSE_PROJECT_NAME` | no | `marketmarkovnet` | Compose project name. |
| `UFW_ALLOW_TCP` | no | `22,80,443` | UFW-allowed TCP ports. **All internal ports (5555, 8080) are blocked from the public internet.** |

## Notes

- `.env` is gitignored. Use `.env.example` as a template.
- `KRAKEN_API_KEY` / `KRAKEN_API_SECRET` are only required when
  `TRADING_MODE=live`. The engine will refuse to start in live mode if either
  is missing.
- The parity gate (Feature 13) must report a clean run before `TRADING_MODE=live`
  is honoured.
