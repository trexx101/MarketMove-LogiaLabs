# Strategy Threshold Recalibration — 2026-08-14

## Context

After fixing the inference de-normalization bug (`label_std` is in ATR-scaled
mag-space, so it must be multiplied by `atr_ratio` to return to raw log-return
space), QQQ predictions jumped from the old ~0.1% scale to a ~1% daily scale.
The old thresholds (`entry=0.001`, `exit=-0.0005`, etc.) became far too tight
and would have fired on almost every bar.

## Empirical distribution used for calibration

1,137 stored QQQ predictions at the time of recalibration:

| Horizon | mean | std | 75th pct | 95th pct |
|---|---|---|---|---|
| pred_1d | 0.72% | 0.88% | 1.06% | 2.23% |
| pred_5d | 0.54% | 2.79% | 2.26% | 4.90% |
| pred_21d | -0.04% | 3.25% | 2.23% | 4.82% |

## New thresholds

| Param | Old | New | Rationale |
|---|---|---|---|
| `ENTRY_THRESHOLD` | 0.001 (0.1%) | **0.008 (0.8%)** | Top ~20–25% of daily signals |
| `EXIT_THRESHOLD` | -0.0005 (-0.05%) | **-0.003 (-0.3%)** | Allow normal pullback before exit |
| `SHORT_ENTRY_THRESHOLD` | -0.001 (-0.1%) | **-0.008 (-0.8%)** | Symmetric to long entry |
| `SHORT_EXIT_THRESHOLD` | 0.0005 (0.05%) | **0.003 (0.3%)** | Symmetric to long exit |

## Files to update

Three places must stay in sync for startup defaults to take effect:

1. `.env` (workspace root) — source of truth for local runs
2. `deploy/docker-compose.yml` — `environment:` defaults (`${VAR:-new_default}`)
3. `engine/Dockerfile` — `ENV` defaults (fallback when no env is injected)

Also update the `StrategySnapshot` / `EquityStrategyParams` construction sites
if a new threshold field is added; see the main `marketmoves-dev` SKILL.md
"Adding a new strategy param" checklist.

## Verification

```bash
# 1. Container env
docker exec mmn-engine env | grep -E "ENTRY|EXIT|SHORT"

# 2. Status API
curl -sS http://localhost:9080/api/status | python3 -m json.tool
```

Expected:
```json
{
  "entry_threshold": 0.008,
  "exit_threshold": -0.003,
  "short_entry_threshold": -0.008,
  "short_exit_threshold": 0.003
}
```

If the API still shows old values, run `env | grep -E "ENTRY|EXIT"` on the
host. Exported shell env vars override `.env` and Dockerfile defaults on
Docker Compose v1.

## Interaction with `pred_5d_filter`

Long entry requires **both**:
- `pred_1d > ENTRY_THRESHOLD`
- `pred_5d > 0.0` (if `PRED_5D_FILTER=true`)

At recalibration time the latest prediction was:
- `pred_1d = 1.24%` (> 0.8%)
- `pred_5d = -5.90%` (< 0.0)

Result: no long entry despite a strong 1-day signal, because the 5-day
forecast was bearish. This is the intended behavior of the filter.
