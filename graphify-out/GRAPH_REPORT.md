# Graph Report - .  (2026-07-15)

## Corpus Check
- 86 files · ~56,312 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 652 nodes · 1227 edges · 44 communities (34 shown, 10 thin omitted)
- Extraction: 93% EXTRACTED · 7% INFERRED · 0% AMBIGUOUS · INFERRED: 90 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

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

## God Nodes (most connected - your core abstractions)
1. `next_position()` - 26 edges
2. `clear_engine_env()` - 22 edges
3. `run_parity()` - 22 edges
4. `compute_features()` - 19 edges
5. `_random_model()` - 16 edges
6. `build_fixture()` - 15 edges
7. `Position` - 15 edges
8. `MarketMarkovNet` - 15 edges
9. `CausalConv1d` - 14 edges
10. `_handle_request()` - 13 edges

## Surprising Connections (you probably didn't know these)
- `ATR Indicator` --semantically_similar_to--> `Feature Engineering Pipeline`  [INFERRED] [semantically similar]
  .agents/skills/technical-analysis/SKILL.md → Training_model_Design.md
- `Rolling Z-Score Normalization` --semantically_similar_to--> `Rolling Feature Pipeline`  [INFERRED] [semantically similar]
  Training_model_Design.md → README.md
- `SMA Indicator` --semantically_similar_to--> `Regime-Filtered Swing Backtester`  [INFERRED] [semantically similar]
  .agents/skills/technical-analysis/SKILL.md → Training_model_Design.md
- `MarketMarkovNet Platform` --conceptually_related_to--> `MarketMarkovNet Neural Architecture`  [INFERRED]
  README.md → Training_model_Design.md
- `Inference Service (Python/PyTorch)` --conceptually_related_to--> `MarketMarkovNet Neural Architecture`  [INFERRED]
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

## Communities (44 total, 10 thin omitted)

### Community 0 - "Inference Service"
Cohesion: 0.05
Nodes (42): device, InferenceConfig, Configuration for the ZMQ inference microservice., Fail fast if model artifacts are missing on disk., _build_logger(), _handle_request(), _JsonFormatter, Tensor (+34 more)

### Community 1 - "Kraken Execution"
Cohesion: 0.07
Nodes (36): convert_symbol(), kraken_signature_matches_documented_scheme(), KrakenExecutor, Client, Result, Self, String, Vec (+28 more)

### Community 2 - "Parity Verification"
Cohesion: 0.10
Nodes (40): build_fixture(), Candle, golden_fixture_round_trips_json(), golden_fixture_sha256_is_stable(), GoldenCandle, GoldenFeature, GoldenFixture, GoldenPrediction (+32 more)

### Community 3 - "Deployment Infrastructure"
Cohesion: 0.05
Nodes (49): Data Volume (SQLite + Parity), uPlot Chart Library, Vanilla SPA Control Room, MarketMarkovNet Model, Prediction Payload (pred_1h/4h/24h), ZMQ REP Socket, model.pt (PyTorch Checkpoint), norm_stats.json (Z-score Stats) (+41 more)

### Community 4 - "Configuration Management"
Cohesion: 0.11
Nodes (36): clear_engine_env(), Config, config_round_trips_through_serde(), custom_env_overrides_defaults(), defaults_load_when_env_unset(), empty_parity_marker_path_rejected(), empty_symbol_rejected(), env_or() (+28 more)

### Community 5 - "Trading Strategy"
Cohesion: 0.14
Nodes (37): compute_sma(), compute_sma_empty_returns_false(), compute_sma_full_window(), compute_sma_partial_window(), next_position(), next_position_flips_long_to_short(), next_position_holds_long_on_neutral_signal(), next_position_holds_short_on_neutral_signal() (+29 more)

### Community 6 - "Feature Computation"
Cohesion: 0.12
Nodes (30): candle(), compute_features(), compute_features_atr_rolling_mean(), compute_features_atr_window_clamp(), compute_features_empty_returns_empty(), compute_features_log_return_correct(), compute_features_single_candle(), FeatureRow (+22 more)

### Community 7 - "Axum Telemetry API"
Cohesion: 0.17
Nodes (31): ApiResult, AppState, candle_to_dto(), CandleDto, chart_computes_rolling_sma(), ChartResponse, handle_chart(), handle_predictions() (+23 more)

### Community 8 - "Frontend SPA"
Cohesion: 0.12
Nodes (25): fetchChart(), fetchPredictions(), fetchStatus(), request(), intervalId, tick(), updateModeBadge(), views (+17 more)

### Community 9 - "Rust Best Practices"
Cohesion: 0.09
Nodes (28): Rust Best Practices Skill, Borrowing Over Cloning, Copy Trait Guidelines, Iterator Patterns, Option/Result Pattern Matching, Prevent Early Allocation, Clippy Linting Discipline, Important Clippy Lints (+20 more)

### Community 10 - "ZMQ Bridge"
Cohesion: 0.14
Nodes (17): Duration, Prediction, Result, Self, ZmqBridge, ExecutorKind, DbPool, Option (+9 more)

### Community 11 - "Database Layer"
Cohesion: 0.21
Nodes (24): Candle, count_candles(), fetch_entry_trade_price(), fetch_latest_candle(), fetch_recent_candles(), fetch_recent_predictions(), fetch_recent_trades(), insert_position_event() (+16 more)

### Community 12 - "Inference Config & Tests"
Cohesion: 0.16
Nodes (21): CaptureFixture, _get(), Inference service configuration.  Loads the inference-side environment variables, main(), clean_env(), Path, Tests for ``inference.config.InferenceConfig``., Engine reaches run_service when config and artifacts are valid. (+13 more)

### Community 13 - "Trading Modes"
Cohesion: 0.21
Nodes (13): Environment Variable Reference, MAGNITUDE_THRESHOLD Configuration, SMA_WINDOW Configuration, TRADING_MODE Configuration, ZMQ_ENDPOINT Configuration, Parity Marker Refresh, Live Trading Mode, Paper Trading Mode (+5 more)

### Community 14 - "REST Data Ingestion"
Cohesion: 0.29
Nodes (11): backfill(), fetch_ohlc(), parse_row(), Candle, Client, DbPool, Result, String (+3 more)

### Community 15 - "API Keys & Security"
Cohesion: 0.24
Nodes (11): Kraken API Key Management, IP Allowlist, Key Rotation Policy, Axum Telemetry Server, Engine Service (Rust/Tokio), Frontend SPA, Inference Service (Python/PyTorch), Kraken v2 API (+3 more)

### Community 16 - "Parity Fixture Generator"
Cohesion: 0.29
Nodes (10): build_predictions(), build_signals(), compute_features(), compute_sma(), main(), next_position(), Synthetic recorded predictions: bullish drift above threshold most of the time., Mirror of engine::features::compute_features (Colab parity). (+2 more)

### Community 17 - "Docker Compose Services"
Cohesion: 0.33
Nodes (10): Caddy TLS Reverse Proxy, Engine Service (Compose), Inference Service (Compose), MMN Internal Network, Models Volume (Read-Only), Proxy Service (Compose), UFW Firewall Rules, Caddy Reverse Proxy (+2 more)

### Community 18 - "VPS Hardening"
Cohesion: 0.24
Nodes (10): Kraken Permissions Matrix, Deploy User Setup, setup.sh Hardening Script, SSH Hardening, UFW Firewall Configuration, Backup Strategy, Caddy Reverse Proxy, Docker Compose Stack (+2 more)

### Community 19 - "WebSocket Ingest"
Cohesion: 0.44
Nodes (9): handle_text(), parse_candle(), parse_iso_ts(), Candle, DbPool, Result, Value, run_loop() (+1 more)

### Community 20 - "Technical Analysis Indicators"
Cohesion: 0.22
Nodes (9): Technical Analysis Skill, ADX Indicator, Bollinger Bands, Correlation Analysis, EMA Indicator, MACD Indicator, pandas-ta Library, RSI Indicator (+1 more)

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
Cohesion: 0.50
Nodes (4): Plan-to-Features Skill, REQUIREMENTS.md Index, Feature File Template, Plan-to-Features Prompt

### Community 25 - "Data Module"
Cohesion: 0.83
Nodes (3): DbPool, Result, run()

### Community 26 - "OpenCode Config"
Cohesion: 0.50
Nodes (3): plugin, $schema, .opencode/plugins/graphify.js

### Community 27 - "Logic Engine Design"
Cohesion: 0.50
Nodes (4): Hysteresis for Position Stickiness, 200-Hour SMA Regime Filter, Trading State Machine (flat/long/short), Signal Parity Check (Exact)

### Community 28 - "Find Skills CLI"
Cohesion: 0.67
Nodes (3): Find Skills Skill, Skills CLI (npx skills), Open Agent Skills Ecosystem

## Knowledge Gaps
- **58 isolated node(s):** `$schema`, `.opencode/plugins/graphify.js`, `views`, `intervalId`, `market-markov-net-inference` (+53 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Position` connect `Kraken Execution` to `Trading Strategy`?**
  _High betweenness centrality (0.074) - this node is a cross-community bridge._
- **Why does `PredictionRow` connect `Database Layer` to `Axum Telemetry API`?**
  _High betweenness centrality (0.042) - this node is a cross-community bridge._
- **Why does `next_position()` connect `Trading Strategy` to `Kraken Execution`, `Parity Verification`?**
  _High betweenness centrality (0.041) - this node is a cross-community bridge._
- **Are the 13 inferred relationships involving `next_position()` (e.g. with `build_fixture()` and `run_parity()`) actually correct?**
  _`next_position()` has 13 INFERRED edges - model-reasoned connections that need verification._
- **Are the 7 inferred relationships involving `run_parity()` (e.g. with `compute_features()` and `compute_sma()`) actually correct?**
  _`run_parity()` has 7 INFERRED edges - model-reasoned connections that need verification._
- **Are the 10 inferred relationships involving `compute_features()` (e.g. with `build_fixture()` and `run_parity()`) actually correct?**
  _`compute_features()` has 10 INFERRED edges - model-reasoned connections that need verification._
- **What connects `$schema`, `.opencode/plugins/graphify.js`, `views` to the rest of the system?**
  _58 weakly-connected nodes found - possible documentation gaps or missing edges._