# Deployment Summary - July 14, 2026

## Issues Fixed

### 1. Model Architecture Mismatch
**Problem**: The inference service failed to load `model.pt` with state_dict key errors.

**Root Cause**: `inference/model.py` was written from the design doc, not the actual Colab training code. The architectures differed in:
- Backbone structure (nested vs flat Sequential)
- GroupNorm configuration (num_groups=8 vs 1)
- Layer naming (draft_1h vs draft_h1)
- Markov head implementation (custom class vs raw Linear layers)
- Input tensor format (channels-first vs sequence-first)

**Solution**: Rewrote `inference/model.py` to match the Colab training code exactly:
- Flat `nn.Sequential` backbone with interleaved CausalConv1d/GroupNorm(1)/SiLU
- Renamed layers to match checkpoint: `draft_h1`, `draft_h4`, `draft_h24`
- Replaced `_LowRankMarkovHead` with raw `nn.Linear` layers (`markov_wA_1to4`, etc.)
- Changed input format to `(batch, seq_len, n_features)` with internal transpose
- Kept `/100` scaling (training targets were ×100)

**Files Changed**:
- `inference/model.py` - Complete rewrite
- `inference/inference_engine.py` - Updated `_tensorize()` and env vars
- `inference/tests/test_inference_contract.py` - Updated test expectations

### 2. Docker Build Cache Issue
**Problem**: The engine binary was a placeholder (`deps-warmup`) instead of the real binary.

**Root Cause**: Docker's layer caching preserved cargo's incremental build artifacts from the placeholder build step, so the real source wasn't recompiled.

**Solution**: Added `touch engine/src/main.rs` before the final build to force cargo to detect the source change.

**File Changed**: `engine/Dockerfile` (line 47)

### 3. SQLite Database Path
**Problem**: Engine failed with "unable to open database file" error.

**Root Cause**: 
- Initial DATABASE_URL used relative path (`sqlite://data/candles.db`)
- sqlx requires absolute path with 3 slashes for `/app/data/candles.db`
- Empty database file needed to be pre-created for sqlx to open it

**Solution**:
- Changed DATABASE_URL to `sqlite:///app/data/candles.db` (3 slashes)
- Pre-created empty `candles.db` file in the volume with correct ownership

**File Changed**: `deploy/docker-compose.yml` (line 98)

### 4. Volume Permissions
**Problem**: Engine container (user 1000) couldn't write to the data volume.

**Root Cause**: Docker named volumes are owned by root by default.

**Solution**: Manually chowned the volume to uid 1000:
```bash
docker run --rm -v deploy_data:/app/data alpine chown -R 1000:1000 /app/data
```

### 5. Port Conflict
**Problem**: Proxy container failed to bind to port 80.

**Root Cause**: Apache2 was running on the host and using port 80.

**Solution**: Stopped and disabled Apache2:
```bash
sudo systemctl stop apache2
sudo systemctl disable apache2
```

### 6. Docker Compose v1 Bug
**Problem**: `docker-compose up` failed with `KeyError: 'ContainerConfig'`.

**Root Cause**: docker-compose v1.29.2 has a known bug with newer Docker versions when recreating containers.

**Workaround**: Remove old containers before recreating:
```bash
docker rm -f mmn-engine mmn-inference mmn-proxy
docker-compose -f deploy/docker-compose.yml up -d
```

## Current Status

All services are running and healthy:
- **mmn-inference**: Healthy, model loaded (63,395 parameters), ZMQ server ready
- **mmn-engine**: Healthy, database connected, HTTP server on :8080, connected to Kraken WebSocket
- **mmn-proxy**: Running, Caddy reverse proxy on :80/:443

API endpoint working:
```bash
curl -sk https://localhost/api/status | jq
```

Returns:
```json
{
  "mode": "paper",
  "symbol": "BTC/USD",
  "position": "flat",
  "last_close": 64593.7,
  "pred_1h": 0.0045,
  "pred_4h": 0.0074,
  "pred_24h": 0.0138
}
```

## Deployment Procedure

### Fresh Deployment

1. **Stop conflicting services**:
   ```bash
   sudo systemctl stop apache2
   sudo systemctl disable apache2
   ```

2. **Build images**:
   ```bash
   cd /home/ubuntu/projects/MarketMoves
   docker-compose -f deploy/docker-compose.yml build
   ```

3. **Initialize volumes**:
   ```bash
   # Create empty database file
   docker run --rm -v deploy_data:/app/data alpine sh -c \
     "touch /app/data/candles.db && chown 1000:1000 /app/data/candles.db"
   
   # Copy model files
   docker run --rm -v deploy_models:/target -v /opt/marketmarkovnet/models:/src:ro \
     alpine cp -a /src/. /target/
   ```

4. **Start services**:
   ```bash
   docker-compose -f deploy/docker-compose.yml up -d
   ```

5. **Verify**:
   ```bash
   docker-compose -f deploy/docker-compose.yml ps
   curl -sk https://localhost/api/status | jq
   ```

### Updating After Code Changes

1. **Rebuild affected images**:
   ```bash
   # For inference changes:
   docker-compose -f deploy/docker-compose.yml build inference
   
   # For engine changes:
   docker-compose -f deploy/docker-compose.yml build engine
   ```

2. **Remove old containers** (workaround for docker-compose v1 bug):
   ```bash
   docker rm -f mmn-engine mmn-inference mmn-proxy
   ```

3. **Start services**:
   ```bash
   docker-compose -f deploy/docker-compose.yml up -d
   ```

### Troubleshooting

**Engine shows "deps-warmup"**:
- Rebuild with `--no-cache`: `docker-compose build --no-cache engine`

**Database connection error**:
- Verify volume permissions: `docker run --rm -v deploy_data:/app/data alpine ls -la /app/data`
- Ensure `candles.db` exists: `docker run --rm -v deploy_data:/app/data alpine touch /app/data/candles.db`

**Model loading error**:
- Rebuild inference image: `docker-compose build inference`
- Verify model files in volume: `docker run --rm -v deploy_models:/models alpine ls -la /models`

**Port 80/443 already in use**:
- Check for conflicting services: `sudo ss -tlnp | grep -E ':80|:443'`
- Stop Apache/Nginx: `sudo systemctl stop apache2 nginx`

**ContainerConfig error**:
- Remove old containers first: `docker rm -f mmn-engine mmn-inference mmn-proxy`

## Volume Names

Docker Compose prefixes volume names with the project directory name:
- `deploy_data` - SQLite database and parity marker
- `deploy_models` - Model files (model.pt, norm_stats.json)
- `deploy_caddy_data` - Caddy TLS certificates
- `deploy_caddy_config` - Caddy configuration cache

## Environment Variables

Key environment variables (set in `.env` or docker-compose.yml):
- `DATABASE_URL`: `sqlite:///app/data/candles.db`
- `MODEL_PATH`: `/models/model.pt`
- `NORM_STATS_PATH`: `/models/norm_stats.json`
- `ZMQ_BIND`: `tcp://0.0.0.0:5555` (inference)
- `ZMQ_ENDPOINT`: `tcp://inference:5555` (engine)
- `TRADING_MODE`: `paper` or `live`
- `MAGNITUDE_THRESHOLD`: `0.005`

## Next Steps

1. **Monitor logs**: `docker-compose -f deploy/docker-compose.yml logs -f`
2. **Check predictions**: `curl -sk https://localhost/api/predictions | jq`
3. **View trades**: `curl -sk https://localhost/api/trades | jq`
4. **Access frontend**: Open `https://localhost` in browser (accept self-signed cert warning)

## Known Limitations

1. **docker-compose v1**: The VPS has docker-compose v1.29.2 which has bugs with newer Docker. Consider upgrading to Docker Compose v2 plugin.

2. **Self-signed certificates**: Caddy generates self-signed certs for localhost. For production, set `HOST` environment variable to your domain for Let's Encrypt certificates.

3. **Paper trading only**: System is currently in paper trading mode. Live trading requires:
   - Setting `TRADING_MODE=live` in `.env`
   - Adding Kraken API keys to `.env`
   - Passing parity verification (Feature 13)
