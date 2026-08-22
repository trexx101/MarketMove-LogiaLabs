# Distinguishing Real Inference Requests from Healthchecks

## What the inference logs look like

`mmn-inference` logs every request. Two shapes appear:

### Healthcheck
```json
{"req_id":15,"symbol":"","seq_len":1,"n_features":8,"atr_ratio":0.005,
 "pred_1d":0.002987,"pred_5d":0.014423,"pred_21d":-0.000063}
```

- `symbol` is empty
- `seq_len` is 1
- `atr_ratio` is always `0.005` (hardcoded in healthcheck)
- Predictions are from an all-zero feature window

### Real request from engine
```json
{"req_id":18,"symbol":"QQQ","seq_len":126,"n_features":8,"atr_ratio":0.016864,
 "pred_1d":0.012442,"pred_5d":-0.059034,"pred_21d":0.001050}
```

- `symbol` is the active ticker (e.g. `QQQ`)
- `seq_len` is 126 (the model window)
- `atr_ratio` is the real ATR(14)/close for that candle

## Why this matters

1. **Do not tune thresholds or debug prediction magnitude from healthcheck logs.** The all-zero window + `atr_ratio=0.005` gives values that may differ wildly from real predictions.
2. **If you see ONLY `seq_len=1` requests, the scheduler may not be processing candles.** Check `docker logs mmn-engine | grep -i scheduler` and confirm `last_processed_ts` is advancing.
3. **If real `seq_len=126` predictions are sane but the DB values are 50× larger,** the `atr_ratio` conversion in `equity_model.py` is missing. See `references/atr-label-scaling-contract.md`.

## One-liner to see only real requests

```bash
docker logs mmn-inference 2>&1 | grep 'seq_len":126' | tail -5
```

## One-liner to compare with stored predictions

```bash
sudo python3 - <<'PY'
import sqlite3
conn = sqlite3.connect('/var/lib/docker/volumes/deploy_data/_data/candles.db')
c = conn.cursor()
c.execute("""SELECT candle_ts, datetime(candle_ts, 'unixepoch'), pred_1d, pred_5d, pred_21d
             FROM equity_predictions WHERE symbol = 'QQQ' ORDER BY candle_ts DESC LIMIT 3""")
for row in c.fetchall(): print(row)
conn.close()
PY
```
