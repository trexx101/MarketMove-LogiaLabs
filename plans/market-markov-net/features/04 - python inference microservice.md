# Feature 04 — Python Inference Microservice

**Depends on:** 01
**Goal:** A headless ZeroMQ REQ/REP service that loads MarketMarkovNet and returns 1H/4H/24H predictions for a normalized feature window.

## Requirements

- Port the `MarketMarkovNet` architecture exactly from the Colab notebook (CausalConv1d backbone with GroupNorm + SiLU, parallel draft heads, low-rank Markov heads).
- Load weights from `model.pt` (CPU, `eval()`, `no_grad`).
- ZMQ REP socket bound to `tcp://0.0.0.0:5555` (localhost-only via Docker network + UFW).
- JSON contract: request `{ "feature_window": [[...], ...] }` → response `{ "pred_1h", "pred_4h", "pred_24h" }` (outputs scaled by /100 to match the training target scaling).
- Structured logging of every request (inputs + outputs) for parity auditing.
- Health/liveness handling and graceful shutdown.

## Technical Implementation Steps

1. `inference/model.py`: define `CausalConv1d` and `MarketMarkovNet` matching Colab cell `g6OcfSsDQAVq`.
2. `inference/inference_engine.py`: load model + `norm_stats.json` (for reference/logging only — normalization happens Rust-side), bind ZMQ REP, loop: recv JSON → tensorize → forward → reply JSON.
3. Apply the same output scaling convention as training (divide by 100).
4. Add per-request logging to stdout (JSON lines).
5. Add a unit test using randomly-initialized weights to validate the request/response contract shape.

## Acceptance Criteria

- [ ] Service starts and binds `:5555`.
- [ ] A test client REQ returns three float predictions with correct scaling.
- [ ] Architecture matches Colab (layer count, heads) — code-reviewed against `Training_model_Design.md`.
- [ ] Every request is logged as a structured line.
