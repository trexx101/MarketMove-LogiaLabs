# Graph Report - MarketMoves  (2026-07-24)

## Corpus Check
- 106 files · ~91,446 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1461 nodes · 2379 edges · 115 communities (105 shown, 10 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 95 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `2bda69e7`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- Inference Service
- Kraken Execution
- Parity Verification
- Deployment Infrastructure
- Configuration Management
- Trading Strategy
- Feature Computation
- Axum Telemetry API
- Frontend SPA
- Rust Best Practices
- ZMQ Bridge
- Database Layer
- Inference Config & Tests
- Trading Modes
- REST Data Ingestion
- API Keys & Security
- Parity Fixture Generator
- Docker Compose Services
- VPS Hardening
- WebSocket Ingest
- Technical Analysis Indicators
- Model Architecture
- Feature Engineering
- Setup Scripts
- Plan-to-Features Skill
- Data Module
- OpenCode Config
- Logic Engine Design
- Find Skills CLI
- Graphify Plugin
- Agent Rules
- Inference Test Fixtures
- Data Pipeline Design
- Docker Cache Issue
- SQLite Path Issue
- Volume Permissions
- MarketMarkovNet Package
- Feature Table
- Tech Stack
- QQQ Equities Engine — Transition Plan from BTC/Quantitative
- equities_features.py
- Chapter 5 - Automated Testing
- llm.rs
- Chapter 8 - Comments vs Documentation
- 9.2 When to use pointers:
- _random_model
- Deployment Summary - July 14, 2026
- mmn-prediction-fix - Work Plan
- mmn-training-deployment-hardening - Work Plan
- test_equity_model.py
- Draft: mmn-prediction-fix
- equity_model.py
- Chapter 4 - Errors Handling
- Chapter 6 - Generics, Dynamic Dispatch and Static Dispatch
- .__init__
- moomoo.rs
- fetch_features.py
- Chapter 3 - Performance Mindset
- yahoo.rs
- fred.rs
- InferenceConfig
- CausalConv1d
- Chapter 7 - Type State Pattern
- EquityInferenceConfig
- load_lgbm
- Borrowing Over Cloning
- Quick Reference
- Wave C — QQQ Equities Model: Planning Prompt for Gemini 3.1 Pro
- Feature 07: Feature Computation & ZMQ
- Feature 10: Axum Telemetry API
- Environment variables
- Provisioning
- MarketMarkovNet Model
- MarketMarkovNet — Wave 5: Edge-First Overhaul (research plan)
- tests/
- Chapter 2 - Clippy and Linting Discipline
- Correlation Analysis
- Kraken API Key Generation Checklist
- Step 3: Verify hardening
- deploy/
- models/
- Wave 5 — Gemini 3.1 Pro Implementation Scaffold (committed artifact)
- Technical Analysis
- Troubleshooting
- Operations
- Feature 03: Kraken Credentials
- Feature 13 — Regression / Parity Harness
- Feature 14 — Docker Compose Deploy & Launch
- frontend/
- _build_logger
- inference/
- _feature_window
- Feature 01 — Repo Scaffold & Workspace
- Feature 02 — VPS Hardening & Infra Setup
- Feature 03 — Kraken Credentials & Config Management
- Feature 04 — Python Inference Microservice
- Feature 05 — Inference Docker Image
- Feature 06 — Rust Data Pipeline (WS Ingestion + SQLite)
- Feature 07 — Feature Computation & ZMQ Bridge
- Feature 08 — Logic Engine (Hysteresis + Regime State Machine)
- Feature 09 — Execution Layer (Paper + Kraken)
- Feature 10 — Axum Telemetry API
- Feature 11 — Vanilla SPA Control Room
- Feature 12 — Paper-Trading Verification
- Quick start
- Pointer Types and Thread Safety
- Prompt: Convert a Plan into Per-Feature Implementation Specs
- Feature Pipeline (log_return, ATR, VWAP)
- AGENTS.md

## God Nodes (most connected - your core abstractions)
1. `compute_equity_features()` - 27 edges
2. `next_position()` - 26 edges
3. `run_parity()` - 22 edges
4. `compute_features()` - 20 edges
5. `clear_engine_env()` - 19 edges
6. `EquityCandle` - 18 edges
7. `synthetic_qqq()` - 17 edges
8. `MarketMarkovNet` - 17 edges
9. `_random_model()` - 16 edges
10. `build_fixture()` - 15 edges

## Surprising Connections (you probably didn't know these)
- `ATR Indicator` --semantically_similar_to--> `Feature Engineering Pipeline`  [INFERRED] [semantically similar]
  .agents/skills/technical-analysis/SKILL.md → Training_model_Design.md
- `Rolling Z-Score Normalization` --semantically_similar_to--> `Rolling Feature Pipeline`  [INFERRED] [semantically similar]
  Training_model_Design.md → README.md
- `SMA Indicator` --semantically_similar_to--> `Regime-Filtered Swing Backtester`  [INFERRED] [semantically similar]
  .agents/skills/technical-analysis/SKILL.md → Training_model_Design.md
- `MarketMarkovNet` --conceptually_related_to--> `MarketMarkovNet Neural Architecture`  [INFERRED]
  README.md → Training_model_Design.md
- `Parity gate` --conceptually_related_to--> `Feature Engineering Pipeline`  [INFERRED]
  README.md → Training_model_Design.md

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **MarketMarkovNet System Architecture** — readme_marketmarkovnet, readme_inference_service, readme_engine_service, readme_frontend, readme_zmq_reqrep, readme_kraken_v2, readme_sqlite, readme_axum_telemetry [EXTRACTED 1.00]
- **MarketMarkovNet Model Training Pipeline** — training_model_design_marketmarkovnet_model, training_model_design_feature_engineering, training_model_design_backbone_pretraining, training_model_design_markov_alignment, training_model_design_directional_temporal_loss, training_model_design_causal_conv1d, training_model_design_draft_heads, training_model_design_markov_heads [EXTRACTED 1.00]
- **Production Deployment Stack** — deploy_readme_docker_compose, deploy_readme_caddy, deploy_readme_security_model, deploy_provisioning_setup_sh, deploy_provisioning_ufw, deploy_provisioning_ssh_hardening, deploy_kraken_keys_api_key, deploy_config_env_vars [EXTRACTED 1.00]
- **Docker Compose Full Deployment Stack** — deploy_docker_compose_yml_inference_service, deploy_docker_compose_yml_engine_service, deploy_docker_compose_yml_proxy_service, deploy_docker_compose_yml_mmn_network, deploy_docker_compose_yml_caddy_proxy [EXTRACTED 1.00]
- **Parity Verification System (Gate + Harness + Marker)** — plans_market_markov_net_requirements_parity_gate, plans_market_markov_net_features_13_parity_harness, plans_market_markov_net_features_13_parity_marker, plans_market_markov_net_features_13_golden_fixture, tests_readme_feature_parity, tests_readme_prediction_parity, tests_readme_signal_parity, plans_market_markov_net_features_12_live_mode_guard [EXTRACTED 1.00]
- **Data Ingestion to Prediction End-to-End Flow** — plans_market_markov_net_features_06_kraken_ws_ingest, plans_market_markov_net_features_06_rest_backfill, plans_market_markov_net_features_07_feature_pipeline, plans_market_markov_net_features_07_zscore_normalization, plans_market_markov_net_features_07_zmq_bridge, inference_readme_zmq_rep, inference_readme_prediction_payload [EXTRACTED 1.00]

## Communities (115 total, 10 thin omitted)

### Community 0 - "Inference Service"
Cohesion: 0.16
Nodes (18): device, _handle_request(), Tensor, MarketMarkovNet inference microservice (ZMQ REP).  Binds a ZeroMQ REP socket and, Block until ``model_path`` exists (useful for Docker startup ordering)., Start the ZMQ REP loop.  Blocks until a shutdown signal is received., Convert a nested list (seq_len × n_features) to a model-ready tensor.      Retur, Decode one REQ message, run inference, and return the serialized reply. (+10 more)

### Community 1 - "Kraken Execution"
Cohesion: 0.10
Nodes (30): ExecutorKind, FillResult, Result, Vec, TradeSide, flat_to_long_opens_position(), long_to_flat_closes_with_pnl(), long_to_short_closes_and_opens() (+22 more)

### Community 2 - "Parity Verification"
Cohesion: 0.10
Nodes (40): build_fixture(), Candle, golden_fixture_round_trips_json(), golden_fixture_sha256_is_stable(), GoldenCandle, GoldenFeature, GoldenFixture, GoldenPrediction (+32 more)

### Community 3 - "Deployment Infrastructure"
Cohesion: 0.22
Nodes (11): Live Mode Guard, Golden Fixture (parity_golden_168h.json), Parity Harness, parity_verified.json Marker, Feature 12: Paper Trading Verification, Feature 13: Regression Parity Harness, Feature 14: Docker Compose Deploy, Paper Trading as Default Mode (+3 more)

### Community 4 - "Configuration Management"
Cohesion: 0.12
Nodes (33): BinanceConfig, clear_engine_env(), Config, config_round_trips_through_serde(), custom_env_overrides_defaults(), defaults_load_when_env_unset(), empty_parity_marker_path_rejected(), empty_symbol_rejected() (+25 more)

### Community 5 - "Trading Strategy"
Cohesion: 0.13
Nodes (39): compute_sma(), compute_sma_empty_returns_false(), compute_sma_full_window(), compute_sma_partial_window(), EquitySignalInput, next_equity_position(), next_position(), next_position_flips_long_to_short() (+31 more)

### Community 6 - "Feature Computation"
Cohesion: 0.08
Nodes (38): FeatureRowV2, candle(), compute_features(), compute_features_atr_rolling_mean(), compute_features_atr_window_clamp(), compute_features_empty_returns_empty(), compute_features_log_return_correct(), compute_features_parity_contract_matches_colab() (+30 more)

### Community 7 - "Axum Telemetry API"
Cohesion: 0.13
Nodes (47): ApiResult, Config, accuracy_returns_503_when_no_resolved(), AccuracyResponse, align_series(), AppState, BackfillResponse, candle_to_dto() (+39 more)

### Community 8 - "Frontend SPA"
Cohesion: 0.09
Nodes (34): fetchAccuracy(), fetchChart(), fetchPredictions(), fetchStatus(), request(), accuracyIntervalId, intervalId, tick() (+26 more)

### Community 9 - "Rust Best Practices"
Cohesion: 0.15
Nodes (11): Clippy Linting Discipline, Important Clippy Lints, Snapshot Testing with cargo insta, Automated Testing Practices, Dynamic Dispatch, Generics and Dispatch, Static Dispatch, PhantomData Usage (+3 more)

### Community 10 - "ZMQ Bridge"
Cohesion: 0.13
Nodes (23): Duration, EquityPrediction, Prediction, Result, Self, ZmqBridge, EquityScheduler, fake_prediction() (+15 more)

### Community 11 - "Database Layer"
Cohesion: 0.14
Nodes (46): AccuracyStats, Candle, compute_actuals(), compute_actuals_fills_null_columns(), compute_actuals_skips_unresolved_horizons(), count_candles(), count_equity_candles(), equity_candles_upsert_and_count() (+38 more)

### Community 12 - "Inference Config & Tests"
Cohesion: 0.19
Nodes (19): CaptureFixture, main(), clean_env(), Path, Tests for ``inference.config.InferenceConfig``., Engine reaches run_service when config and artifacts are valid., test_config_is_immutable(), test_custom_endpoint() (+11 more)

### Community 13 - "Trading Modes"
Cohesion: 0.21
Nodes (13): Environment Variable Reference, MAGNITUDE_THRESHOLD Configuration, SMA_WINDOW Configuration, TRADING_MODE Configuration, ZMQ_ENDPOINT Configuration, Parity Marker Refresh, Live Trading Mode, Paper Trading Mode (+5 more)

### Community 14 - "REST Data Ingestion"
Cohesion: 0.10
Nodes (43): EquityCandle, adx_14(), adx_in_range(), compute_equity_features(), compute_returns_correct_length(), drawdown_from_high(), drawdown_nonpositive(), eq_candle() (+35 more)

### Community 15 - "API Keys & Security"
Cohesion: 0.15
Nodes (15): Kraken API Key Management, IP Allowlist, Key Rotation Policy, Architecture, Axum Telemetry Server, Engine Service (Rust/Tokio), Environment variables, Inference Service (Python/PyTorch) (+7 more)

### Community 16 - "Parity Fixture Generator"
Cohesion: 0.29
Nodes (10): build_predictions(), build_signals(), compute_features(), compute_sma(), main(), next_position(), Synthetic recorded predictions: bullish drift above threshold most of the time., Mirror of engine::features::compute_features (Colab parity). (+2 more)

### Community 17 - "Docker Compose Services"
Cohesion: 0.33
Nodes (10): Caddy TLS Reverse Proxy, Engine Service (Compose), Inference Service (Compose), MMN Internal Network, Models Volume (Read-Only), Proxy Service (Compose), UFW Firewall Rules, Caddy Reverse Proxy (+2 more)

### Community 18 - "VPS Hardening"
Cohesion: 0.24
Nodes (10): Kraken Permissions Matrix, Deploy User Setup, setup.sh Hardening Script, SSH Hardening, UFW Firewall Configuration, Backups, Caddy Reverse Proxy, Docker Compose Stack (+2 more)

### Community 19 - "WebSocket Ingest"
Cohesion: 0.05
Nodes (37): 1.1 Borrowing Over Cloning, 1.2 When to pass by value? (Copy trait), 1.3 Handling `Option<T>` and `Result<T, E>`, 1.4 Prevent Early Allocation, 1.5 Iterator, `.iter` vs `for`, 1.6 Comments: Context, not Clutter, 1.7 Use Declarations - "imports", 🚨 Anti-patterns to AVOID (+29 more)

### Community 20 - "Technical Analysis Indicators"
Cohesion: 0.25
Nodes (7): ADX Indicator, Bollinger Bands, EMA Indicator, MACD Indicator, pandas-ta Library, RSI Indicator, Sharpe Ratio

### Community 21 - "Model Architecture"
Cohesion: 0.25
Nodes (8): Model Architecture Mismatch Fix, Stage 1: Backbone Pre-training, CausalConv1d, DirectionalTemporalLoss, Parallel Draft Heads, MarketMarkovNet Neural Architecture, Stage 2: Markov Alignment Fine-tuning, Low-Rank Markov Heads

### Community 22 - "Feature Engineering"
Cohesion: 0.40
Nodes (6): Rolling Feature Pipeline, ATR Indicator, Binance Data Ingestion, Feature Engineering Pipeline, Rolling Z-Score Normalization, SwingTradingDataset

### Community 23 - "Setup Scripts"
Cohesion: 0.70
Nodes (4): log(), setup.sh script, ufw_is_active(), ufw_rule_exists()

### Community 24 - "Plan-to-Features Skill"
Cohesion: 0.22
Nodes (8): Feature file template, Notes, Output structure, Plan to Features, Steps, REQUIREMENTS.md Index, Feature File Template, Plan-to-Features Prompt

### Community 25 - "Data Module"
Cohesion: 0.80
Nodes (5): backfill_equities(), prune_equity_history(), DbPool, Result, run_equities_ingestion()

### Community 26 - "OpenCode Config"
Cohesion: 0.50
Nodes (3): plugin, $schema, .opencode/plugins/graphify.js

### Community 27 - "Logic Engine Design"
Cohesion: 0.50
Nodes (4): Hysteresis for Position Stickiness, 200-Hour SMA Regime Filter, Trading State Machine (flat/long/short), Signal Parity Check (Exact)

### Community 28 - "Find Skills CLI"
Cohesion: 0.12
Nodes (15): Common Skill Categories, Find Skills, How to Help Users Find Skills, Step 1: Understand What They Need, Step 2: Check the Leaderboard First, Step 3: Search for Skills, Step 4: Verify Quality Before Recommending, Step 5: Present Options to the User (+7 more)

### Community 42 - "Feature Table"
Cohesion: 0.22
Nodes (8): Confirmed product decisions, Data model (SQLite), Environment variables, Feature table, Global constraints / rules, MarketMarkovNet — Requirements Index, Overview, Tech stack

### Community 43 - "Tech Stack"
Cohesion: 0.09
Nodes (26): Dataset, build_feature_matrix(), calibrate_barrier_c(), compute_atr(), DataFrame, ndarray, Series, 10-dim feature matrix with robust scaling. Replaces 6-dim (no more llm stub). (+18 more)

### Community 44 - "QQQ Equities Engine — Transition Plan from BTC/Quantitative"
Cohesion: 0.08
Nodes (25): 0 · Why we are switching, 10 · Decision log, 11 · Next milestones, 1 · Target architecture, 2 · Why these features (and not technical indicators alone), 3 · Five Waves (8 weeks total), 4 · Data flow contract, 5 · Reuse from existing engine (+17 more)

### Community 45 - "equities_features.py"
Cohesion: 0.14
Nodes (24): adx_14(), compute_equity_features(), drawdown_from_high(), gap_pct(), generate_synthetic_data(), main(), ndarray, Bucket VIX: <18→0, 18-25→1, >25→2. (+16 more)

### Community 46 - "Chapter 5 - Automated Testing"
Cohesion: 0.09
Nodes (22): 5.1 Tests as Living Documentation, 5.2 Add Test Examples to your Docs, 5.3 Unit Test vs Integration Tests vs Doc tests, 5.4 How to `assert!`, 5.5 Snapshot Testing with `cargo insta`, 5.6 ✅ Snapshot Best Practices, 🚨 `assert!` reminders, Attributes: (+14 more)

### Community 47 - "llm.rs"
Cohesion: 0.14
Nodes (13): config_defaults_sensible(), config_from_env_disabled_without_key(), fetch_regime_prob(), LlmRegimeConfig, parse_bull_prob(), read_write_cache(), Default, Result (+5 more)

### Community 48 - "Chapter 8 - Comments vs Documentation"
Cohesion: 0.10
Nodes (20): 8.1 Comments vs Documentation: Know the Difference, 8.2 When to use comments, 8.3 When comments get in the way, 8.4 Don't Write Living Documentation (living comments), 8.5 Replace Comments with Code, 8.6 `TODO` should become issues, 8.7 When to use doc comments, 8.8 Documentation in Rust: How, When and Why (+12 more)

### Community 49 - "9.2 When to use pointers:"
Cohesion: 0.10
Nodes (19): 9.1 Thread Safety, 9.2 When to use pointers:, [`Arc<T>`](https://doc.rust-lang.org/std/sync/struct.Arc.html) - Atomic Reference Counter (multi-thread), [`Box<T>`](https://doc.rust-lang.org/std/boxed/struct.Box.html) - Heap Allocated, [`Cell<T>`](https://doc.rust-lang.org/std/cell/struct.Cell.html) - Copy-only interior mutability, Chapter 9 - Understanding Pointers, [`*const T/*mut T`](https://doc.rust-lang.org/std/primitive.pointer.html) - Raw pointers, 📌 Language Comparison (+11 more)

### Community 50 - "_random_model"
Cohesion: 0.17
Nodes (7): _random_model(), Predictions for the same input must not change across batch sizes., All predictions must be well within log-return magnitude bounds., Return a randomly-initialised model (no file I/O needed)., Predictions should be in log-return scale (~small floats), not ×100., TestMarketMarkovNetShapes, TestWireContract

### Community 51 - "Deployment Summary - July 14, 2026"
Cohesion: 0.11
Nodes (17): 1. Model Architecture Mismatch, 2. Docker Build Cache Issue, 3. SQLite Database Path, 4. Volume Permissions, 5. Port Conflict, 6. Docker Compose v1 Bug, Current Status, Deployment Procedure (+9 more)

### Community 52 - "mmn-prediction-fix - Work Plan"
Cohesion: 0.11
Nodes (17): Commit strategy, Dependency matrix, Execution strategy, Final verification wave, mmn-prediction-fix - Work Plan, Must have, Must NOT have (guardrails, anti-slop, scope boundaries), Parallel execution waves (+9 more)

### Community 53 - "mmn-training-deployment-hardening - Work Plan"
Cohesion: 0.11
Nodes (17): Commit strategy, Dependency matrix, Execution strategy, Final verification wave, mmn-training-deployment-hardening - Work Plan, Must have, Must NOT have (guardrails, anti-slop, scope boundaries), Parallel execution waves (+9 more)

### Community 54 - "test_equity_model.py"
Cohesion: 0.17
Nodes (14): _free_port(), _handle_request_v3(), Round-trip contract tests for the V3 equities inference service.  Tests the JSON, Call _handle_request directly (bypasses ZMQ) to test the handler logic., _handle_request must return pred_1d, pred_5d, pred_21d., Handler must reject windows with wrong feature dimension., Handler must reject empty feature_window., Start the service, send a V3 request over ZMQ, verify the response. (+6 more)

### Community 55 - "Draft: mmn-prediction-fix"
Cohesion: 0.12
Nodes (15): Approval gate, Bug 1: Scheduler 30-second retry loop, Bug 2: seq_len=1 / insufficient candle data, Bug 3: Data pipeline not receiving new confirmed candles, Bug 4: Dashboard shows one prediction row, Components (topology ledger), Decisions (with rationale), Draft: mmn-prediction-fix (+7 more)

### Community 56 - "equity_model.py"
Cohesion: 0.19
Nodes (13): EquityEnsemble, _handle_request(), _load_ensemble(), main(), QQQ daily equities inference service (Wave C).  ZMQ REP server that loads the TC, TCN + LightGBM ensemble for 1d/5d/21d horizon predictions.      The TCN consumes, Run ensemble prediction on a normalized feature window.          Parameters, Decode one V3 REQ message, run inference, return serialized reply. (+5 more)

### Community 57 - "Chapter 4 - Errors Handling"
Cohesion: 0.14
Nodes (13): 4.1 Prefer `Result`, avoid panic 🫨, 4.2 Avoid `unwrap`/`expect` in Production, 4.3 `thiserror` for Crate level errors, 4.4 Reserve `anyhow` for Binaries, 4.5 Use `?` to Bubble Errors, 4.6 Unit Test should exercise errors, 4.7 Important Topics, 🚨 Alternative ways of handling `unwrap`/`expect`: (+5 more)

### Community 58 - "Chapter 6 - Generics, Dynamic Dispatch and Static Dispatch"
Cohesion: 0.14
Nodes (14): 6.1 [Generics](https://doc.rust-lang.org/book/ch10-00-generics.html), 6.2 Static Dispatch: `impl Trait` or `<T: Trait>`, 6.3 Dynamic Dispatch: `dyn Trait`, 6.4 Trade-off summary, 6.5 Best Practices for Dynamic Dispatch, 6.6 🚨 Trait Objects Ergonomics, ❌ Avoid Dynamic Dispatch When:, ✅ Best when: (+6 more)

### Community 59 - ".__init__"
Cohesion: 0.21
Nodes (7): Any, CausalConv1d, Tensor, QqqTCN, TCN for QQQ daily equities prediction.      Architecture (matches training/train, Causal 1-D convolution (left-only padding, sequence length preserved).      Subc, ResidualBlock

### Community 60 - "moomoo.rs"
Cohesion: 0.19
Nodes (11): backfill(), backfill_returns_clear_error_without_credentials(), kline_to_equity_candle(), kline_to_equity_candle_basic(), MoomooConfig, DbPool, Option, Result (+3 more)

### Community 61 - "fetch_features.py"
Cohesion: 0.29
Nodes (13): compute_basis_z(), compute_ob_imbalance(), fetch_and_merge_features(), fetch_binance_vision_funding(), fetch_binance_vision_klines(), _fetch_zip_url(), ms_to_date(), DataFrame (+5 more)

### Community 62 - "Chapter 3 - Performance Mindset"
Cohesion: 0.15
Nodes (13): 3.1 Flamegraph, 3.2 Avoid Redundant Cloning, 3.3 Stack vs Heap: Be size-smart!, 3.4 Iterators and Zero-Cost Abstractions, A good first steps, ❗ Avoid creating intermediate collections unless it is really needed:, ❗ Be Mindful, Chapter 3 - Performance Mindset (+5 more)

### Community 63 - "yahoo.rs"
Cohesion: 0.29
Nodes (11): Client, backfill(), backfill_many(), fetch_chart(), parse_chart(), parse_chart_extracts_ohlcv(), parse_chart_skips_nan_rows(), DbPool (+3 more)

### Community 64 - "fred.rs"
Cohesion: 0.27
Nodes (11): backfill_all_default_macros(), backfill_macro(), parse_fred_csv(), parse_fred_csv_filters_by_range(), parse_fred_csv_handles_missing_and_header(), DbPool, Option, Result (+3 more)

### Community 65 - "InferenceConfig"
Cohesion: 0.15
Nodes (9): InferenceConfig, Inference service configuration.  Loads the inference-side environment variables, Configuration for the ZMQ inference microservice (legacy V1 protocol)., Fail fast if model artifacts are missing on disk., _build_logger(), _JsonFormatter, Logger, LogRecord (+1 more)

### Community 66 - "CausalConv1d"
Cohesion: 0.21
Nodes (6): CausalConv1d, Tensor, Run a forward pass.          Parameters         ----------         x:, 1-D convolution with causal (left-only) receptive field.      Achieved by paddin, Prediction at step t must not change when future steps change., TestCausalConv1d

### Community 67 - "Chapter 7 - Type State Pattern"
Cohesion: 0.17
Nodes (12): 7.1 What is Type State Pattern?, 7.2 Why use it?, 7.3 Simple Example: File State, 7.4 Real-World Examples, 7.5 Pros and Cons, ❌ Avoid it when:, Builder Pattern with Compile-Time Guarantees, Chapter 7 - Type State Pattern (+4 more)

### Community 68 - "EquityInferenceConfig"
Cohesion: 0.17
Nodes (9): EquityInferenceConfig, Fail fast if any model artifact is missing., Configuration for the QQQ V3 inference microservice (TCN + LightGBM ensemble)., EquityInferenceConfig.from_env must resolve defaults correctly., EquityInferenceConfig.from_env must respect env var overrides., require_artifacts must raise FileNotFoundError for missing files., test_equity_inference_config_defaults(), test_equity_inference_config_env_override() (+1 more)

### Community 69 - "load_lgbm"
Cohesion: 0.21
Nodes (12): load_lgbm(), load_tcn(), Load a trained TCN from a state-dict checkpoint., Load a LightGBM model from a pickle file.      Returns the underlying ``Booster`, Ensemble weights must affect output (verify blending is not a no-op)., Handler must return error for unparseable JSON., EquityEnsemble.predict must return pred_1d, pred_5d, pred_21d., Ensemble must handle windows longer than seq_len (TCN reads last timesteps). (+4 more)

### Community 70 - "Borrowing Over Cloning"
Cohesion: 0.18
Nodes (12): Borrowing Over Cloning, Copy Trait Guidelines, Iterator Patterns, Option/Result Pattern Matching, Prevent Early Allocation, Flamegraph Profiling, Performance Mindset, Stack vs Heap Allocation (+4 more)

### Community 71 - "Quick Reference"
Cohesion: 0.18
Nodes (11): Best Practices Reference, Borrowing & Ownership, Documentation, Error Handling, Generics & Dispatch, Linting, Performance, Quick Reference (+3 more)

### Community 72 - "Wave C — QQQ Equities Model: Planning Prompt for Gemini 3.1 Pro"
Cohesion: 0.18
Nodes (10): 1. PROJECT CONTEXT (locked facts), 2. LOCKED FEATURE CONTRACT (Wave B) — DO NOT REORDER/REMOVE, 3. LOCKED MODEL BACKBONE (port from crypto V2 `training/train_tcn.py`), 4. LOCKED LABEL SCHEME (port from `training/labels.py`), 5. LOCKED WALK-FORWARD + DEPLOY GATE, 6. LOCKED SERVING / STORAGE CONTRACT (alignment target), 7. YOUR TASKS (produce a concrete plan), 8. OPEN DECISIONS (state your recommended default + rationale; user may override) (+2 more)

### Community 73 - "Feature 07: Feature Computation & ZMQ"
Cohesion: 0.22
Nodes (10): Data Volume (SQLite + Parity), CPU-Only Inference Constraint, SQLite Data Model, Feature 01: Repo Scaffold, Feature 02: VPS Hardening, Feature 04: Python Inference, Feature 05: Inference Docker Image, Feature 06: Rust Data Pipeline (+2 more)

### Community 74 - "Feature 10: Axum Telemetry API"
Cohesion: 0.24
Nodes (10): uPlot Chart Library, Vanilla SPA Control Room, GET /api/chart, GET /api/predictions, GET /api/status, API Polling Logic, Feature 08: Logic Engine, Feature 09: Execution Layer (+2 more)

### Community 75 - "Environment variables"
Cohesion: 0.22
Nodes (5): Compose / infra, Engine (Rust), Environment variables, Inference (Python), Notes

### Community 76 - "Provisioning"
Cohesion: 0.22
Nodes (9): Execute, Manual steps, Provisioning, Re-running safely, Step 1: Initial access, Step 2: Run setup.sh, Step 4: Application deployment, Step 5: Filesystem layout (+1 more)

### Community 77 - "MarketMarkovNet Model"
Cohesion: 0.28
Nodes (9): MarketMarkovNet Model, Prediction Payload (pred_1h/4h/24h), ZMQ REP Socket, model.pt (PyTorch Checkpoint), Causal CNN Backbone, Parallel Draft Heads, Low-Rank Markov Heads, ZMQ Bridge (REQ Client) (+1 more)

### Community 78 - "MarketMarkovNet — Wave 5: Edge-First Overhaul (research plan)"
Cohesion: 0.22
Nodes (8): Acceptance / go-no-go (from R1 + corrections), Build order (proposed), CRITIQUE + CORRECTIONS (Hermes, applied), Decision (locked with user), MarketMarkovNet — Wave 5: Edge-First Overhaul (research plan), R1 plan (summary — verbatim structure, see /tmp/hermes-r1-plan.md for full text), References, Why this exists (the proven failure)

### Community 79 - "tests/"
Cohesion: 0.22
Nodes (8): How to run, Ignore policy, Replacing the placeholder, Status, tests/, What gets checked, What goes here, Why it matters

### Community 80 - "Chapter 2 - Clippy and Linting Discipline"
Cohesion: 0.25
Nodes (8): 2.1 Why care about linting?, 2.2 Always run `cargo clippy`, 2.3 Important Clippy Lints to Respect, 2.4 Fix warnings, don't silence them!, 2.5 Configure workspace/package lints, Chapter 2 - Clippy and Linting Discipline, Example:, Handling false positives

### Community 81 - "Correlation Analysis"
Cohesion: 0.25
Nodes (8): Arguments, Correlation Analysis, Dependencies, Examples, Instructions, Interpretation, Output, Timezone

### Community 82 - "Kraken API Key Generation Checklist"
Cohesion: 0.25
Nodes (8): 1. Prerequisites, 2. UI walkthrough, 3. Permissions matrix, 4. IP allowlist (recommended), 5. Storage, 6. Verification, 7. Revocation / rotation, Kraken API Key Generation Checklist

### Community 83 - "Step 3: Verify hardening"
Cohesion: 0.29
Nodes (7): 3.1 Firewall, 3.2 Docker Compose plugin, 3.3 Docker runtime, 3.4 Deploy user, 3.5 SSH hardening, 3.6 Idempotency check, Step 3: Verify hardening

### Community 84 - "deploy/"
Cohesion: 0.29
Nodes (7): deploy/, Layout, Quick start (local dev), Quick start (production VPS), Security notes, See also, Status

### Community 85 - "models/"
Cohesion: 0.29
Nodes (6): ⚠️ Engine integration status (read before wiring up), Feature contract (MUST match `engine/src/features/equities_v2.rs`), Layout, `model_meta_qqq_v1.json`, models/, Retired artifacts (removed)

### Community 86 - "Wave 5 — Gemini 3.1 Pro Implementation Scaffold (committed artifact)"
Cohesion: 0.29
Nodes (6): CORRECTIONS TO GEMINI'S ASSUMPTIONS (verify at build time), NEXT STEPS (build order, per DAG), Scaffold summary (what Gemini built), SECTOR SPLIT (as requested), Wave 5 — Gemini 3.1 Pro Implementation Scaffold (committed artifact), WHO TRAINS THE MODEL (clarification)

### Community 87 - "Technical Analysis"
Cohesion: 0.33
Nodes (6): Arguments, Examples, Instructions, Interpretation, Output, Technical Analysis

### Community 88 - "Troubleshooting"
Cohesion: 0.33
Nodes (6): Docker Compose plugin not found after install, Docker group requires new login, Locked out after SSH hardening, sshd restart fails, Troubleshooting, UFW blocking Docker ports

### Community 89 - "Operations"
Cohesion: 0.33
Nodes (6): Operations, Refresh the parity marker (required weekly when running live), Restart a single service, Tail logs, Tear down, Update the engine after a code change

### Community 90 - "Feature 03: Kraken Credentials"
Cohesion: 0.40
Nodes (6): Kraken API Key Restrictions, Executor Trait, Kraken Executor (REST Orders), Paper Executor (Simulated Fills), Environment Variables, Feature 03: Kraken Credentials

### Community 91 - "Feature 13 — Regression / Parity Harness"
Cohesion: 0.33
Nodes (5): Acceptance Criteria, Feature 13 — Regression / Parity Harness, Live-mode gate integration, Requirements, Technical Implementation Steps

### Community 92 - "Feature 14 — Docker Compose Deploy & Launch"
Cohesion: 0.33
Nodes (5): Acceptance Criteria, Feature 14 — Docker Compose Deploy & Launch, Operational docs, Requirements, Technical Implementation Steps

### Community 93 - "frontend/"
Cohesion: 0.40
Nodes (4): Files, frontend/, Serving, Status

### Community 94 - "_build_logger"
Cohesion: 0.40
Nodes (4): _build_logger(), _JsonFormatter, Logger, LogRecord

### Community 95 - "inference/"
Cohesion: 0.40
Nodes (4): Docker, inference/, Local dev (uv), Status

### Community 96 - "_feature_window"
Cohesion: 0.50
Nodes (3): _feature_window(), Return a synthetic feature window as a plain Python list., TestTensorize

### Community 97 - "Feature 01 — Repo Scaffold & Workspace"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 01 — Repo Scaffold & Workspace, Requirements, Technical Implementation Steps

### Community 98 - "Feature 02 — VPS Hardening & Infra Setup"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 02 — VPS Hardening & Infra Setup, Requirements, Technical Implementation Steps

### Community 99 - "Feature 03 — Kraken Credentials & Config Management"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 03 — Kraken Credentials & Config Management, Requirements, Technical Implementation Steps

### Community 100 - "Feature 04 — Python Inference Microservice"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 04 — Python Inference Microservice, Requirements, Technical Implementation Steps

### Community 101 - "Feature 05 — Inference Docker Image"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 05 — Inference Docker Image, Requirements, Technical Implementation Steps

### Community 102 - "Feature 06 — Rust Data Pipeline (WS Ingestion + SQLite)"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 06 — Rust Data Pipeline (WS Ingestion + SQLite), Requirements, Technical Implementation Steps

### Community 103 - "Feature 07 — Feature Computation & ZMQ Bridge"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 07 — Feature Computation & ZMQ Bridge, Requirements, Technical Implementation Steps

### Community 104 - "Feature 08 — Logic Engine (Hysteresis + Regime State Machine)"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 08 — Logic Engine (Hysteresis + Regime State Machine), Requirements, Technical Implementation Steps

### Community 105 - "Feature 09 — Execution Layer (Paper + Kraken)"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 09 — Execution Layer (Paper + Kraken), Requirements, Technical Implementation Steps

### Community 106 - "Feature 10 — Axum Telemetry API"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 10 — Axum Telemetry API, Requirements, Technical Implementation Steps

### Community 107 - "Feature 11 — Vanilla SPA Control Room"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 11 — Vanilla SPA Control Room, Requirements, Technical Implementation Steps

### Community 108 - "Feature 12 — Paper-Trading Verification"
Cohesion: 0.40
Nodes (4): Acceptance Criteria, Feature 12 — Paper-Trading Verification, Requirements, Technical Implementation Steps

### Community 109 - "Quick start"
Cohesion: 0.40
Nodes (5): Engine (Rust, host-side dev), Frontend, Inference (Python, host-side dev), Production (Docker Compose), Quick start

### Community 110 - "Pointer Types and Thread Safety"
Cohesion: 0.83
Nodes (4): Arc Atomic Reference Counter, Mutex Thread-Safe Mutability, Pointer Types and Thread Safety, Send and Sync Traits

### Community 112 - "Feature Pipeline (log_return, ATR, VWAP)"
Cohesion: 1.00
Nodes (3): norm_stats.json (Z-score Stats), Feature Pipeline (log_return, ATR, VWAP), Z-Score Normalization (Rust-side)

## Knowledge Gaps
- **415 isolated node(s):** `$schema`, `.opencode/plugins/graphify.js`, `views`, `intervalId`, `accuracyIntervalId` (+410 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `compute_features()` connect `Feature Computation` to `Parity Verification`?**
  _High betweenness centrality (0.033) - this node is a cross-community bridge._
- **Why does `run_parity()` connect `Parity Verification` to `Trading Strategy`, `Feature Computation`?**
  _High betweenness centrality (0.023) - this node is a cross-community bridge._
- **Why does `EquityCandle` connect `REST Data Ingestion` to `fred.rs`, `Axum Telemetry API`, `ZMQ Bridge`, `Database Layer`, `moomoo.rs`, `yahoo.rs`?**
  _High betweenness centrality (0.021) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `compute_equity_features()` (e.g. with `.process()` and `rust_vs_python_feature_parity()`) actually correct?**
  _`compute_equity_features()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Are the 13 inferred relationships involving `next_position()` (e.g. with `build_fixture()` and `run_parity()`) actually correct?**
  _`next_position()` has 13 INFERRED edges - model-reasoned connections that need verification._
- **Are the 7 inferred relationships involving `run_parity()` (e.g. with `compute_features()` and `compute_sma()`) actually correct?**
  _`run_parity()` has 7 INFERRED edges - model-reasoned connections that need verification._
- **What connects `$schema`, `.opencode/plugins/graphify.js`, `views` to the rest of the system?**
  _415 weakly-connected nodes found - possible documentation gaps or missing edges._