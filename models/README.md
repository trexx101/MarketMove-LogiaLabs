# models/

This directory holds the model artifacts used by the inference microservice.

## Files (user-supplied, gitignored)

- `model.pt` — PyTorch checkpoint for `MarketMarkovNet`. Loaded by
  `inference/inference_engine.py` (Feature 04) and the Rust engine during the
  parity harness (Feature 13, read-only).
- `norm_stats.json` — Z-score normalization stats (mean / std per feature)
  produced by the Colab training notebook. The Rust engine also uses these
  during the parity check.

## Where to drop them

Place both files at the root of this directory:

```
models/
├── model.pt
└── norm_stats.json
```

In production (Feature 14, Docker Compose) this directory is bind-mounted
into the `inference` container at `/models`. The default paths inside the
container are:

- `MODEL_PATH=/models/model.pt`
- `NORM_STATS_PATH=/models/norm_stats.json`

## Source

These artifacts are **supplied by the user** — they are the output of the
Colab training notebook (`Training_model_Design.md`). No training is in
scope for this repo.

The directory is covered by `.gitignore` (`/models/*` with `!.gitkeep` and
`!README.md` allowed), so the artifacts themselves will never be committed.
