# Deploying the MarketMoves multi-model stack

Session: 2026-08-06 redeploy after NVDA multi-model + sentiment overlay.

## What changed

- `trading_models` registry now drives the engine bootstrap.
- Two enabled models: `qqq-v1` (QQQ/PSQ) and `nvda-v1` (NVDA/NVDD).
- Sentiment overlay added: 4 new strategy params per model.
- Inference container is still **single-model**: both schedulers hit the same ZMQ endpoint.

## Redeploy recipe

```bash
cd /home/ubuntu/projects/MarketMoves

# 1. Build images
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
docker build -f inference/Dockerfile -t marketmarkovnet/inference:latest ./inference

# 2. Full down/up (avoids compose v1.29.2 ContainerConfig KeyError)
docker-compose -f deploy/docker-compose.yml down
docker-compose -f deploy/docker-compose.yml --env-file .env up -d
```

`--env-file .env` is required because compose v1 resolves `${VAR:-}` in the
compose file from the project `.env`, not the `env_file` declared inside the
service. Without it, `FINNHUB_API_KEY` and `OPENROUTER_API_KEY` expand to empty.

## Registering models in a fresh DB

If `deploy_data` is a fresh volume, the `trading_models` table is empty.
Register with the checked-in script:

```bash
# Use alpine (root) because keinos/sqlite3 runs as uid 100 and cannot write
# a DB owned by uid 1000.
docker run --rm -v deploy_data:/data \
  -v ./scripts/register_models.sql:/register_models.sql:ro \
  alpine sh -c "apk add --no-cache sqlite && sqlite3 /data/candles.db < /register_models.sql"

# Verify
docker run --rm -v deploy_data:/data alpine sh -c \
  "apk add --no-cache sqlite && sqlite3 /data/candles.db 'SELECT model_id, primary_symbol, inverse_symbol FROM trading_models'"
```

Then restart the engine so it reads the registry:

```bash
docker restart mmn-engine
```

## Adding new model artifacts to the named `models` volume

Model artifacts live on the named `models` volume, not in the image.
Copy a new model directory (e.g. `models/NVDA`) into the volume:

```bash
docker run --rm -v deploy_models:/models -v ./models:/hostmodels:ro \
  alpine cp -r /hostmodels/NVDA /models/
```

## Fixing norm_stats paths for Docker

The engine inside the container sees models at `/models`. Registry rows
must use absolute paths:

```sql
UPDATE trading_models SET norm_stats_path='/models/norm_stats_qqq_v1.json' WHERE model_id='qqq-v1';
UPDATE trading_models SET norm_stats_path='/models/NVDA/norm_stats_nvda_v1.json' WHERE model_id='nvda-v1';
```

## Health checks

```bash
# Wait for healthy
docker ps --format "table {{.Names}}\t{{.Status}}"

# Verify APIs
curl -sS http://localhost:9080/api/status | python3 -m json.tool
curl -sS http://localhost:9080/api/models | python3 -m json.tool
curl -sS "http://localhost:9080/api/strategy-config?model_id=nvda-v1" | python3 -m json.tool

# Verify per-model sentiment toggle
curl -sS -X PUT "http://localhost:9080/api/strategy-config?model_id=nvda-v1" \
  -H "Content-Type: application/json" \
  -d '{"enable_sentiment_overlay":true,"sentiment_reduce_threshold":-0.45,"sentiment_exit_threshold":-0.75,"sentiment_min_articles":10}'
```

## Known live issues

- **Finnhub 403**: the free API key tier cannot access `/news_sentiment`. Engine falls back to stub score 0.5.
- **LLM 400**: the configured advisor model string (`google/gemini-2.5-flash-lite`) is rejected by OpenRouter. LLM regime falls back to neutral 0.5.
- **Single-model inference**: the inference container loads only the QQQ model. Both schedulers send requests to the same endpoint, so NVDA predictions are actually produced by the QQQ model. See `multi-model-inference.md` for the per-model inference path.
