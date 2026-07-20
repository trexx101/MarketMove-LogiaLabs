# Wave 5 — Gemini 3.1 Pro Implementation Scaffold (committed artifact)

SOURCE: `openrouter/google/gemini-3.1-pro-preview` via OmniRoute (run 2026-07-20,
15,850 chars). Produced the architectural SCAFFOLD (file tree, Rust interfaces, stubs,
build-order DAG). Full text: `/tmp/hermes-gemini-scaffold.md` (regenerate via
`hermes-run-gemini-scaffold.py`). The R1 plan + 10 corrections are the authoritative
design: `.omo/plans/mmn-edge-overhaul.md`.

## Scaffold summary (what Gemini built)
- RETIRED Kraken: delete `src/data/kraken.rs` + `src/exec/kraken.rs`; remove exports.
- `FeatureRowV2` (6-dim, ORDER FIXED):
  [0] vol_regime [1] vol_break [2] funding_rate [3] basis_z
  [4] llm_bull_prob [5] ob_imbalance. `to_array()` assembles the ZMQ/TCN vector.
- `NormStats` v2: adds `schema_version` (default 1), `mean/std` become `Vec<f64>`,
  validates v1=3 feats / v2=6 feats, keeps backward-compat. `normalize_row_v2`.
- `FeatureSource` trait (`fetch_latest` / `backfill_window`) + `BinanceFeatureSource`
  + `EquitiesFeatureSourceStub`.
- ZMQ `ZmqPayload { schema_version, feature_window: &[[f64; FEATURE_DIM]] }`; bridge
  v2 keeps `Prediction { pred_1h/4h/24h }`.
- config.rs: `BinanceConfig` + `LlmConfig`.
- Build-order DAG: P1 cleanup+config → P2 core structs → P3 data ingestion →
  P4 feature assembly → P5 bridge → P6 plug D1/D4 → P7 training D2/D3.

## SECTOR SPLIT (as requested)
- Gemini (SCAFFOLD NOW): S1 data-client layout, S2 FeatureSource trait, S3 FeatureRowV2,
  S4 NormStats v2, S5 ZMQ/config schema, S6 build DAG.
- DEFERRED to R1-class reasoning model:
  - D1 GARCH vol_regime + changepoint vol_break (`features/volatility.rs`)
  - D2 vol-scaled penetration labels (`training/labels.py`)
  - D3 TCN train loop + focal loss + walk-forward embargo/purge + deploy gate
       (`training/train_tcn.py`)
- DEFERRED to cheap/fast LLM: D4 OpenRouter LLM/vision hourly-cached adapter
  (`features/llm.rs`), timeout+fallback, never in per-bar path.

## CORRECTIONS TO GEMINI'S ASSUMPTIONS (verify at build time)
- C1: Gemini assumed `sqlx::PgPool` (PostgreSQL). ACTUAL engine uses SQLite
  (`engine/src/db.rs` is SQLite, e.g. `prune_old`). Keep SQLite; do NOT introduce Pg.
- C2: Gemini assumed `tmq` for ZMQ. ACTUAL engine uses `zeromq` crate
  (`bridge.rs`: `use zeromq::{ReqSocket, Socket, SocketRecv, SocketSend, ZmqMessage}`).
  Bridge v2 stub MUST keep the `zeromq` crate + existing `ReqSocket` API, not switch
  to `tmq`. The `ZmqPayload` shape change (schema_version + [f64;6] window) is correct.
- C3: DB schema for FeatureRowV2 columns (funding_rate, basis_z, etc.) must be added to
  the existing SQLite migration path — NOT a separate out-of-band migration. Use the
  project's migration tooling (do NOT `drizzle push` per AGENTS.md; use generate+migrate).
- C4: Equities `FeatureSource` stub uses `unimplemented!()` — fine for now, but mark it
  clearly so it can't be selected at runtime.

## WHO TRAINS THE MODEL (clarification)
No LLM trains. Gemini + R1 produce CODE. You (or Colab) execute the training script
`training/train_tcn.py` on a GPU. Artifacts (`model.pt` + `norm_stats.json` v2) drop
into the repo; the Rust engine loads them. Engine never trains.

## NEXT STEPS (build order, per DAG)
1. P1: delete kraken files; add BinanceConfig/LlmConfig to config.rs (keep zeromq).
2. P2: FeatureRowV2 + NormStats v2 (backward-compat).
3. P3: Binance data client (REST klines/funding/premiumIndex + ws depth20/aggTrade),
   preserve REST-backfill + 5min staleness watchdog.
4. P4: FeatureSource trait + Binance impl (D1/D4 fields stubbed 0.0) + equities stub.
5. P5: bridge v2 with schema_version + [f64;6] window (zeromq crate).
6. P6: hand D1/D4 to R1/cheap model; P7: hand D2/D3 (training) to R1; you run on Colab.
7. Deploy ONLY if walk-forward OOS IC > 0.03-0.05 AND OOS equity > 0.
