# **Control Room Platform Redesign for MarketMarkovNet** 

## **1. Executive Summary** 

The transformation of the MarketMarkovNet quantitative trading system from a read-only monitoring dashboard into a comprehensive, professional-grade Control Room represents a critical evolution in the platform's lifecycle. Building strictly upon the immutable contracts of the Wave A data ingest and Wave B feature pipeline, this redesign integrates a highperformance reactive frontend directly into the Rust Axum binary. By incorporating real-time WebSocket telemetry, the system transitions away from inefficient polling mechanisms, enabling instantaneous visibility into market micro-structural shifts and algorithmic execution fills. The introduction of the Strategy Lab, powered by an embedded pure-Rust scripting engine, allows operators to sandbox, backtest, and hot-reload complex trading logic without necessitating backend recompilation. Furthermore, the integration of a Large Language Model (LLM) advisory layer—contextually aware of the system's 8-dimensional feature vectors and temporal predictions—provides nuanced macroeconomic reasoning to augment the purely quantitative signals. Designed for deployment on a Singapore-based OVH virtual private server, the architecture natively leverages the Moomoo OpenD gateway, securing submillisecond local execution latency while introducing mathematically symmetric shorting capabilities through inverse exchange-traded funds. 

## **2. Architecture Diagram** 

The extended system architecture heavily utilizes the existing Dockerized deployment model but introduces sophisticated inter-process communication bridges and a compiled-in frontend to eliminate extraneous web server dependencies. 

+---------------------------------------------------------------------------------------+ 

| Singapore OVH VPS Server | | | | +-------------------------------+ 

+-----------------------------------------+ | | | Data Sources | | Rust Engine (Axum 0.7) | | | | - Yahoo Finance (Daily/EOD) | | | | | | - FRED (Macro/VIX/TLT) | | 

+-----------------------------------+ | | | | | | | Core Strategy & Execution | | | | | 

+-------------------------+ | | | - next_equity_position() | | | | | | futu-opend-docker | |<>| - Rhai Strategy Sandbox (NEW) | | | | | | (Moomoo API Gateway) | | | | - PaperExecutor / LiveExecutor | | | | | | Port 11111: Protobuf/TCP| | | +-----------------------------------+ | | | | | Port 22222: Telnet 

2FA | | | | | | | +-------------------------+ | | +-----------------------------------+ | | | 

+-------------------------------+ | | Axum HTTP + WebSocket Server | | | | | | - REST APIs (/api/...) | | | | +-------------------------------+ | | - WS Telemetry (/api/v1/ws) | | | | | SQLite Database (candles.db) | | | - rust-embed SPA Fallback |<+ | | - equity_candles (Locked) | <====>+-----------------------------------+ | | | | - equity_predictions (Locked) | 

+-----------------------------------------+ | | | - equity_trades (Locked) | ^ | | | - strategy_configs (NEW) | | | | | - backtest_results (NEW) | v | | | - advisor_log (NEW) | +-----------------------------------------+ | | | - mode_switches (NEW) | | Python Inference Microservice | | | +-------------------------------+ | (TCN + LightGBM Ensemble) | | | | tcp://127.0.0.1:5555 (ZMQ REQ/REP V3) | | | +-----------------------------------------+ | +---------------------------------------------------------------------------------------+ | +---------------------------------------------------------------------------------------+ 

| Web Browser Client | | | | 

+---------------------------------------------------------------------------------+ | | | Vite + 

Svelte SPA (Compiled to static assets via rust-embed) | | | | - Live PnL/Chart Widget | Strategy Lab (Backtesting) | LLM Advisor Chat | | | 

+---------------------------------------------------------------------------------+ | 

+---------------------------------------------------------------------------------------+ 

The Moomoo OpenD gateway, running as a local container, exposes a highly optimized Protobuf TCP interface on port 11111<sup>1</sup> . This effectively bypasses public internet latency for order routing, dropping execution delays to roughly 1.4 milliseconds<sup>3</sup> . The Svelte frontend is compiled during the Rust build process and embedded into the final binary via rust-embed, guaranteeing that the deployment remains a tightly coupled, single-executable engine<sup>5</sup> . 

## **3. New API Endpoints** 

The existing /api/* endpoints remain entirely undisturbed, preserving the read-only integrity of the underlying system. The following new endpoints extend the Control Room's interactive capabilities. All endpoints implement robust JSON schema validation and utilize Tokio's asynchronous handlers to prevent blocking the core trading loop. 

|**Route**|**Method**|**Request**<br>**Schema**|**Response**<br>**Schema**|**Owning**<br>**Module**|**Purpose**|
|---|---|---|---|---|---|
|/api/mode|GET|_None_|{ "mode":<br>"paper" |<br>"live",<br>"last_switch<br>":<br>171892000<br>0,<br>"parity_valid<br>": bool }|engine/<br>src/api/<br>mode.rs|Retrieves<br>the current<br>state of<br>Arc<RwLoc<br>k<TradingM<br>ode>> and<br>verifes the<br>age of the<br>parity<br>marker fle.|



|/api/mode|POST|{ "mode":<br>"live",<br>"auth_token<br>":<br>"TOTP_CO<br>DE" }|{ "success":<br>bool,<br>"message":<br>"..." }|engine/<br>src/api/<br>mode.rs|Executes a<br>runtime<br>switch to<br>live trading.<br>Returns<br>403 if the<br>TOTP is<br>invalid or<br>the parity<br>marker<br>exceeds<br>604,800<br>seconds.|
|---|---|---|---|---|---|
|/api/<br>backtest|POST|{ "strategy_i<br>d": "uuid",<br>"start_ts":<br>16000000<br>00,<br>"end_ts":<br>170000000<br>0, "params":<br>{ ... } }|{ "equity_cu<br>rve": [...],<br>"metrics":<br>{ "cagr":<br>0.15,<br>"sharpe":<br>1.2, "mdd":<br>-0.1,<br>"win_rate":<br>0.54 } }|engine/<br>src/api/<br>backtest.rs|Dispatches<br>historical<br>predictions<br>through the<br>selected<br>strategy<br>engine<br>(compiled<br>or Rhai).|
|/api/<br>strategies|GET|_None_|[ { "id":<br>"uuid",<br>"name": "...",<br>"type":<br>"threshold" |<br>"rhai",<br>"params":<br>{...} } ]|engine/<br>src/api/<br>strategy_la<br>b.rs|Fetches<br>saved user<br>strategy<br>profles<br>from the<br>SQLite<br>database.|
|/api/<br>strategies|POST|{ "name":<br>"...", "type":<br>"rhai",<br>"script": "...",|{ "id": "uuid",<br>"success":<br>true }|engine/<br>src/api/<br>strategy_la|Persists a<br>new<br>strategy<br>confgurati|



|||"params":<br>{...} }||b.rs|on or Rhai<br>plugin<br>script to<br>the<br>database.|
|---|---|---|---|---|---|
|/api/<br>advisor/<br>briefng|GET|_None_|{ "timestam<br>p":<br>171892000<br>0, "action":<br>"hold",<br>"confdence<br>": 0.8,<br>"reasoning":<br>"..." }|engine/<br>src/api/<br>advisor.rs|Returns the<br>pre-<br>computed,<br>hourly-<br>cached LLM<br>market<br>briefng.|
|/api/<br>advisor/ask|POST|{ "question":<br>"Why did<br>the model<br>exit the<br>long<br>position?" }|_Server-_<br>_Sent Events_<br>_(SSE)_<br>_Stream_|engine/<br>src/api/<br>advisor.rs|Facilitates<br>conversatio<br>nal<br>interrogatio<br>n of the<br>model's<br>logic using<br>current DB<br>context.|
|/api/v1/ws|GET|_WebSocket_<br>_Upgrade_<br>_Request_|_Streams:_<br>{ "type":<br>"pnl_tick",<br>"data": {...} }|engine/<br>src/api/<br>ws.rs|Establishes<br>a<br>persistent,<br>bi-<br>directional<br>WebSocket<br>connection<br>for real-<br>time<br>telemetry.|



## **4. New DB Tables** 

To facilitate the Strategy Lab, audit trails for live-mode transitions, and LLM advisory logs, the following tables are introduced into data/candles.db. The core equity_candles, equity_predictions, and equity_trades tables remain structurally locked to ensure backward compatibility. 

<mark>SQL -- Defnes user-created trading strategies, supporting both basic parameter overrides and complex Rhai scripts. CREATE TABLE strategy_confgs ( id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE</mark> , <mark>strategy_type TEXT NOT NULL</mark> , <mark>-- Constrained to 'threshold' or 'rhai' script_body TEXT,            -- Populated exclusively if strategy_type = 'rhai' params_json TEXT NOT NULL</mark> , <mark>-- Serialized representation of EquityStrategyParams is_active BOOLEAN NOT NULL DEFAULT</mark> 0, <mark>created_at INTEGER NOT NULL</mark> , <mark>updated_at INTEGER NOT NULL</mark> ); 

<mark>-- Maintains a strict audit log of all transitions between Paper and Live modes to satisfy risk compliance. CREATE TABLE mode_switches ( id INTEGER PRIMARY KEY AUTOINCREMENT, previous_mode TEXT NOT NULL</mark> , <mark>new_mode TEXT NOT NULL</mark> , <mark>parity_marker_age_secs INTEGER NOT NULL</mark> , <mark>authorized_by TEXT NOT NULL</mark> , <mark>timestamp INTEGER NOT NULL</mark> ); <mark>-- Archives all interactions with the LLM to analyze the advisor's long-term accuracy and reasoning drif. CREATE TABLE advisor_log ( id INTEGER PRIMARY KEY AUTOINCREMENT,</mark> 



<!-- Start of picture text -->
    interaction_type TEXT NOT NULL , -- 'briefng' or 'chat'<br>    prompt_context_json TEXT NOT NULL ,<br>    model_used TEXT NOT NULL ,<br>    response_json TEXT NOT NULL ,<br>    suggested_action TEXT,          -- Extracted structured action (e.g., 'reduce_exposure')<br>timestamp INTEGER NOT NULL<br>);<br>-- Caches computationally heavy backtest metrics to prevent redundant CPU cycles during UI<br>refreshes.<br>CREATE TABLE backtest_results (<br>    id TEXT PRIMARY KEY,<br>    strategy_id TEXT NOT NULL ,<br>    start_ts INTEGER NOT NULL ,<br>    end_ts INTEGER NOT NULL ,<br>    metrics_json TEXT NOT NULL ,<br>    equity_curve_json TEXT NOT NULL ,<br>timestamp INTEGER NOT NULL ,<br>FOREIGN KEY(strategy_id) REFERENCES strategy_confgs(id) ON DELETE CASCADE<br>);<br><!-- End of picture text -->

SQLite handles these append-heavy tables exceptionally well in Write-Ahead Logging (WAL) mode. Because the historical backtesting involves intensive read operations over equity_predictions, utilizing a single SQLite connection pool with appropriate pragmas (e.g., PRAGMA synchronous = NORMAL; PRAGMA cache_size = -64000;) prevents database locks from stalling the asynchronous Axum HTTP handlers. 

## **5. Frontend Plan** 

### **5.1 Framework Selection (OD1 Resolution)** 

The recommended frontend architecture replaces the existing Vanilla JS ES modules with a **Vite + Svelte** stack. _Rationale:_ As a trading dashboard scales to include multi-threaded WebSockets, complex form states for strategy tuning, and interactive charting, Vanilla JS becomes highly brittle and prone to DOM-syncing bugs. Svelte offers an optimal paradigm: it compiles away the framework overhead, delivering surgically targeted DOM updates without shipping a heavy virtual DOM runtime to the browser<sup>7</sup> . This preserves the lightweight ethos of the current application while drastically improving developer experience (DX) and component modularity. 

### **5.2 File Structure and Layout** 

The dashboard transitions from a fixed 4-panel grid to a flexible, widget-driven layout, ideal for 

a multi-monitor trading desk. 

frontend/ ├── package.json ├── ├── src/  │├── main.js # Mounts the Svelte vite.config.js application  │├── App.svelte # Root shell containing sidebar navigation  │├── lib/   ││├── api.js # REST wrapper and WebSocket telemetry store   ││├── stores.js # Reactive global state (Live PnL, Engine Status)   ││├── components/ # Granular UI elements (ChartContainer, ParamInput)  │├── views/   ││├── Dashboard.svelte # Primary view: Live chart, PnL, Feature Inspector   ││├── ││ StrategyLab.svelte # Backtest configuration, Rhai editor, metrics tables ├── Ledger.svelte # Historical trades and account equity tracking   ││├── Advisor.svelte # LLM chat interface and daily briefings The Dashboard view centers around an expanded uPlot candlestick chart overlaying the 200SMA. Crucially, the chart will render predicted trajectory cones visualizing the 1D, 5D, and 21D model outputs against the live price. Below the chart, the Feature Inspector utilizes horizontal sparklines to represent the 8-dimensional live feature array (e.g., trend_adx, vix_regime), clearly indicating where current values sit relative to their median/MAD normalized baselines dictated by norm_stats_qqq_v1.json. 

### **5.3 Axum Static Serving Integration** 

Serving a compiled Vite SPA via Axum requires specific fallback logic, as SPAs handle their own client-side routing. If a user navigates to /strategy-lab and refreshes the browser, Axum must not return a 404 error; it must serve index.html and allow Svelte's router to take over. 

The solution integrates the rust-embed crate with a custom handler, eliminating the need for complex directory serving constraints<sup>5</sup> . 

<mark>Rust // engine/src/api/static_serve.rs use rust_embed::RustEmbed; use axum::{response::{Html, IntoResponse, Response}, htp::{StatusCode, header, Uri}}; #[derive(RustEmbed)] #[folder = "../frontend/dist/"] struct Assets</mark> ; <mark>pub async</mark> fn <mark>spa_fallback_handler(uri: Uri) -> Response {</mark> le <mark>t path = uri.path().trim_start_matches(</mark> '/'); <mark>// Atempt to serve the exact asset (e.g., CSS, JS, images)</mark> 

if let <mark>Some(content) = Assets::get(path) {</mark> le <mark>t mime = mime_guess::from_path(path).frst_or_octet_stream(); ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response() } else</mark> { <mark>// Fallback to index.html for client-side routing</mark> if let <mark>Some(index) = Assets::get("index.html"</mark> ) { <mark>Html(index.data).into_response() } else</mark> { <mark>(StatusCode::NOT_FOUND, "404 Not Found").into_response() }</mark> } } 

## **6. Strategy Lab Design** 

### **6.1 Backtest Engine (OD2 Resolution)** 

The backtest engine will run as a **Rust-compiled strategy replay** rather than a Python notebook. _Rationale:_ Relying on Python for backtesting introduces a severe risk of execution logic drift between the simulation environment and the actual production Rust PaperExecutor. By loading historical predictions from the equity_predictions table into memory and feeding them through the exact Rust state machine utilized in live trading, the backtester guarantees parity. The engine executes natively, processing years of daily predictions in milliseconds. 

### **6.2 Plugin System (OD8 Resolution)** 

The recommended extensibility approach is **Embedded Scripting via Rhai** . _Rationale:_ Rhai is a pure Rust scripting engine. Unlike mlua (Lua), which requires complex C-bindings and introduces the cognitive dissonance of 1-based indexing<sup>9</sup> , Rhai compiles effortlessly alongside the Axum application. It provides absolute sandboxing out of the box—scripts cannot access . This ensures the file system, network, or environment variables unless explicitly permitted<sup>10</sup> that arbitrary user strategies cannot compromise the underlying VPS. 

### **6.3 Strategy Interface Specification** 

The Rust host engine passes a strictly typed SignalInput struct into the Rhai evaluation scope. The user's script computes the logic and must return an integer representing the desired position (1 for Long, 0 for Flat, -1 for Short). 



<!-- Start of picture text -->
Rust<br><!-- End of picture text -->

<mark>// engine/src/strategy_lab/rhai_plugin.rs use rhai::{Engine, Scope, EvalAltResult}; pub struct EquitySignalInput</mark> { <mark>pub pred_1d: f64</mark> , <mark>pub pred_5d: f64</mark> , <mark>pub pred_21d: f64</mark> , <mark>pub current_close: f64</mark> , <mark>pub sma: f64</mark> , <mark>pub sma_valid: bool</mark> , <mark>pub vix_regime: i64</mark> , } 

<mark>pub</mark> fn <mark>evaluate_rhai_strategy(script: &</mark> st <mark>r, input: &EquitySignalInput, current_pos: i64) -> Result<i64</mark> , <mark>Box<EvalAltResult>> {</mark> let <mark>mut engine = Engine::new(); // Strict sandboxing: Limit execution cycles and AST depth engine.set_max_expr_depths(</mark> 50, 50); <mark>engine.set_max_operations(10_000</mark> ); let <mark>mut scope = Scope::new(); scope.push("pred_1d", input.pred_1d); scope.push("pred_5d", input.pred_5d); scope.push("pred_21d", input.pred_21d); scope.push("current_close", input.current_close); scope.push("sma", input.sma); scope.push("sma_valid", input.sma_valid); scope.push("vix_regime", input.vix_regime); scope.push("current_pos", current_pos); engine.eval_with_scope::<i64>(&mut scope, script)</mark> } 

**Example User-Defined Rhai Strategy in the UI:** 

<mark>Rust // Strategy: Asymmetric Momentum with VIX fltering</mark> i <mark>f sma_valid && current_close > sma { // Bullish Macro Regime</mark> i <mark>f pred_1d > 0.003 && pred_5d > 0.0</mark> { <mark>return</mark> 1; <mark>// Enter Long } else</mark> i <mark>f pred_1d < -0.001</mark> { <mark>return</mark> 0; <mark>// Exit to Flat</mark> } } <mark>else</mark> { <mark>// Bearish Macro Regime</mark> i <mark>f pred_1d < -0.005 && vix_regime >=</mark> 1 { <mark>return</mark> -1; <mark>// Enter Short } else</mark> i <mark>f pred_1d > 0.002</mark> { <mark>return</mark> 0; <mark>// Exit Short</mark> } } <mark>return current_pos; // Hold current position</mark> 

### **6.4 Strategy Lab UI** 

The Strategy Lab interface offers two primary modalities. The "Standard Mode" exposes simple sliders and numeric inputs mapped to the extended EquityStrategyParams. The "Advanced Mode" reveals a Monaco editor for writing Rhai scripts. Users specify a date range, click "Run Backtest," and the Svelte application renders an interactive equity curve comparing the strategy's hypothetical performance against a pure Buy-and-Hold baseline. A secondary metrics table highlights maximum drawdown (MDD), win rate, and total fee drag. 

## **7. Shorting Design** 

### **7.1 Execution Mechanism (OD7 Resolution)** 

The recommended approach for executing short exposure is the utilization of **Inverse ETFs** . _Rationale:_ Initiating direct short positions via a brokerage API **(specifically PSQ for QQQ)** introduces significant friction, including hard-to-borrow fees, margin account minimums, and complex liquidation risk management<sup>12</sup> . Options strategies (Puts) introduce time decay (Theta) and implied volatility volatility crush, which dilutes the clean directional magnitude forecasted by the TCN model. Purchasing an unleveraged inverse ETF (PSQ) behaves symmetrically to a long purchase of QQQ, requiring only a standard cash account while perfectly mirroring the model's 1-to-21 day predictive horizons. 

### **7.2 Strategy Logic and State Machine Updates** 

The system's foundational next_equity_position function is extended to handle shorting logic natively, governed by a strict user configuration flag. 

<mark>Rust pub struct EquityStrategyParams</mark> { <mark>pub entry_threshold: f64</mark> , <mark>pub exit_threshold: f64</mark> , <mark>pub sma_window: usize</mark> , <mark>pub enable_shorting: bool,        // Safely defaults to false pub short_entry_threshold: f64</mark> , <mark>// e.g., -0.004 pub short_exit_threshold: f64</mark> , <mark>// e.g., 0.001</mark> } 

When enable_shorting is true, a prediction falling below the short_entry_threshold in a bearish regime (Price < SMA200) triggers a transition to Position::Short. 

##### **Execution Flow:** 

1. The engine calculates the target position as Position::Short. 

2. The PaperExecutor (and eventually the Live Executor) interprets this state transition. 

3. If the current position is Long (QQQ), it issues a Sell order for QQQ to achieve a Flat state. 

4. It immediately issues a Buy order for the target dollar-allocation of PSQ. 

5. PnL tracking for a short trade mathematically measures the profit generated by the PSQ position, abstracting the inverse relationship away from the user interface. 

## **8. Paper/Live Toggle Design** 

### **8.1 Runtime Mode Management** 

Transitioning from simulated trading to live execution requires mutating global state within the Axum application without halting the asynchronous data ingestion scheduler. The TradingMode enum is encapsulated within Tokio's RwLock inside the application state. 

<mark>Rust // engine/src/state.rs pub struct AppState</mark> { <mark>pub trading_mode: Arc<RwLock<TradingMode>>, pub db_pool: SqlitePool, pub parity_marker_path: String</mark> , <mark>pub totp_secret: String</mark> , } 

When the execution engine evaluates an order, it acquires a brief read-lock on trading_mode to determine whether to route the instruction to the PaperExecutor or the MoomooLiveExecutor. 

### **8.2 Safety Mechanisms (OD6 Resolution)** 

The recommended safety mechanism for live toggling is a combination of a **Time-based OneTime Password (TOTP) and a strict Parity Marker Validation** . _Rationale:_ A simple click-toconfirm modal is insufficient for a system managing real capital. A TOTP (via Google Authenticator) enforces physical operator presence and intent. 

When a user initiates the toggle via POST /api/mode, the handler executes two strict checks: 

1. **Parity Validation:** It reads parity_verified.json. If the timestamp is older than 

PARITY_MAX_AGE_SECS (7 days), the transition is rejected with a 403 status code, enforcing the locked deploy gate rules. 

2. **TOTP Validation:** It verifies the submitted 6-digit code against the server's securely 

stored totp_secret. 

Every transition state (e.g., Paper -> Live -> Paper) writes an immutable record to the mode_switches audit table. 

## **9. PnL Tracking Design** 

### **9.1 Mark-to-Market Real-time Metrics** 

The existing equity_trades table stores realized_pnl exclusively when a trade cycle closes. To operate a professional control room, operators require instantaneous Mark-to-Market (MTM) calculations for open positions. 

The frontend Svelte store merges static database reads with the live data stream. When the system is long QQQ:Unrealized PnL = (Current QQQ Price - Average Entry Price) * Quantity When the system is short (Long PSQ):Unrealized PnL = (Current PSQ Price - Average Entry Price) * Quantity 

### **9.2 Analytics and Charting** 

The historical ledger computes advanced portfolio metrics using standard quantitative formulas. These are processed dynamically in Rust and cached to prevent CPU bloat: 

- **Cumulative Equity Curve:** An array of historical account balances, summing initial 

capital, aggregate realized PnL, and current unrealized PnL. 

- **Maximum Drawdown (MDD):** A rolling calculation tracking the deepest peak-to-trough decline in the equity curve. 

- **Sharpe & Sortino Ratios:** Evaluating the annualized return against standard deviation and downside deviation, respectively. 

The Trade History widget provides an expandable ledger. Clicking a row reveals the exact prediction parameters (1D, 5D, 21D) that generated the entry signal, providing a transparent audit trail of algorithmic behavior. 

## **10. AI Trading Advisor Design** 

### **10.1 Model Selection (OD3 Resolution)** 

The recommended primary model for the advisory layer is **DeepSeek V4 Flash** via OpenRouter, utilizing **Claude 3.5 Sonnet** as an optional analytical fallback. _Rationale:_ DeepSeek V4 Flash offers remarkable financial reasoning at a fraction of the cost of frontier models, priced at approximately $0.14 input / $0.28 output per million tokens<sup>13</sup> . Because the advisor runs an automated hourly briefing parsing dense arrays of features, keeping operational costs low is paramount. OpenRouter charges a 5.5% platform fee on credit-card top-ups, which must be factored into operational budgets, yet it remains the most efficient aggregate gateway<sup>14</sup> . 

### **10.2 Prompt Architecture and State Injection** 

The engine/src/advisor.rs module compiles a textual representation of the current system state, stripping out proprietary execution code but providing total statistical transparency. **Prompt Template Payload:** You are an expert quantitative advisor analyzing the MarketMarkovNet daily equities model. CURRENT STATE: 

- Market Regime: {sma_valid ? (current_close > sma ? "Bullish" : "Bearish") : "Unknown"} 

- Current Position: {position} 

- Model Predictions: 1D: {pred_1d}, 5D: {pred_5d}, 21D: {pred_21d} 

- Live Feature Vector: [Slope: {f0}, ADX: {f1}, RSI: {f2}, VIX Regime: {f3}, TLT Corr: {f4}, RVol: {f5}, Gap: {f6}, DD: {f7}] 

Analyze the features against the predictions. Does the volatility context (VIX) or momentum (RSI) contradict the model's directional forecast? Output your response in valid JSON matching this schema: { "action": "hold" | "exit" | "alert", "confidence": 0.0-1.0, "reasoning": "...", "suggested_params": {} } 

### **10.3 Caching and Safety Constraints** 

LLM API calls introduce variable latency ranging from 1 to 5 seconds. Therefore, the hourly briefing generation is strictly decoupled from the core 5-minute EquityScheduler polling loop. The generated JSON is stored in an RwLock cache and written to the advisor_log table. **Absolute Safety Constraint:** The AI Advisor possesses zero authority to execute trades or 

alter parameters autonomously. It acts strictly as a diagnostic overlay. If it provides a suggested_params output (e.g., suggesting a tightening of the entry_threshold), the Svelte UI renders a one-click "Test Configuration" button, porting those parameters directly into the Strategy Lab for user-driven historical backtesting. 

## **11. Real-Time Updates** 

### **11.1 Transport Strategy (OD4 Resolution)** 

The architecture must deprecate the legacy 5-second polling loop in favor of **WebSockets** via the tokio-tungstenite crate. _Rationale:_ High-frequency polling places unnecessary overhead on the Axum server and yields a sluggish user experience during volatile market moments. WebSockets establish a persistent, full-duplex TCP connection, allowing the Rust engine to push state changes to the UI instantaneously. 

### **11.2 Implementation Path** 

1. **Rust Backend:** A tokio::sync::broadcast channel is instantiated within the AppState. As new candles arrive or positions shift, the system publishes a serialized TelemetryEvent enum to this channel. 

2. **WebSocket Upgrade:** The /api/v1/ws endpoint upgrades incoming HTTP requests. A dedicated Tokio task consumes the broadcast channel and forwards the messages down the WebSocket stream. 

3. **Frontend Integration:** The Svelte application initializes the WebSocket upon mounting. The connection logic implements exponential backoff for automatic reconnection. Realtime messages update Svelte stores, instantly propagating UI changes across the PnL metrics and uPlot charts without requiring explicit component refreshes. 

## **12. API / Third-Party Service Recommendations** 

Optimizing the data architecture for a Singapore-based OVH VPS demands careful evaluation of API latency, reliability, and Rust integration capabilities. 

|**Category**|**Recommende**<br>**d Service**|**Cost**<br>**Structure**|**Integration**<br>**Strategy**|**Rationale &**<br>**Priority**|
|---|---|---|---|---|
|**Live**<br>**Execution &**<br>**Intraday**<br>**Quotes (OD9)**|Moomoo<br>OpenAPI<br>(OpenD)|Free with<br>Futu/Moomoo<br>Brokerage<br>Account<sup>3</sup>|Custom TCP<br>Protobuf client<br>over Port<br>11111<sup>1</sup>.|**Priority 1**<br>**(MVP):**<br>Moomoo's<br>OpenD<br>gateway<br>daemon runs<br>locally on the|



|||||VPS,<br>communicatin<br>g with<br>Moomoo's<br>servers. This<br>provides<br>robust<br>institutional-<br>grade latency<br>(down to<br>1.4ms)<sup>3</sup>and<br>natively<br>supports the<br>TrdEnv_Simula<br>te and<br>TrdEnv_Real<br>environments<br>essential for<br>our Paper/Live<br>logic<sup>16</sup>.|
|---|---|---|---|---|
|**News /**<br>**Sentiment**<br>**Context**|Finnhub|Free tier (60<br>requests/minut<br>e)|Hourly REST<br>polling via<br>reqwest.|**Priority 2:**<br>Provides<br>exceptionally<br>clean,<br>structured<br>JSON<br>company and<br>macroeconomi<br>c news. This<br>serves as<br>critical<br>qualitative<br>context to<br>inject into the<br>LLM Advisor's<br>prompt<br>alongside the<br>numeric|



|||||feature<br>vectors.|
|---|---|---|---|---|
|**Alternative**<br>**Data**<br>**Enrichment**|FRED (Existing)|Free|Retained via<br>existing fred.rs<br>module.|**Priority 3:**The<br>current model<br>heavily relies<br>on FRED data<br>for VIX and TLT<br>correlation. It<br>is sufcient for<br>the daily<br>horizon and<br>requires no<br>immediate<br>alteration.|
|**Options/**<br>**Derivatives**<br>**Data**|Polygon.io|$29/mo (Basic<br>Paid Tier)|WebSocket<br>streaming for<br>deep liquidity<br>data.|**Priority 4**<br>**(Future):**If the<br>strategy<br>graduates<br>from inverse<br>ETFs to<br>complex<br>options<br>spreads,<br>Polygon ofers<br>superior<br>WebSocket<br>data density<br>compared to<br>Yahoo<br>Finance's<br>delayed feeds.|



## **13. Implementation Phases** 

The deployment strategy ensures that each foundational component is tested and stabilized before introducing higher-order complexity. **Phase 1: Reactive Dashboard & Telemetry (Effort: 2 Weeks)** 

   - Scaffold the Vite + Svelte repository. 

   - Integrate rust-embed within Axum for unified binary serving and SPA routing fallback. 

   - Replace the REST polling loop with a tokio-tungstenite WebSocket broadcast channel. 

   - Overhaul the dashboard UI into a flexible widget layout, visualizing real-time PnL and the 8-dim feature vectors. 

- **Phase 2: Strategy Lab & Rhai Integration (Effort: 3 Weeks)** 

   - Implement the SQLite schema for strategy_configs and backtest_results. 

   - Construct the Rust historical replay engine. 

   - Integrate the Rhai scripting environment, strictly configuring its security sandbox and SignalInput mapping. 

   - Build the frontend backtesting interface, implementing uPlot overlays for equity curves and A/B comparison metrics. 

- **Phase 3: Execution Overhaul & Shorting Mechanics (Effort: 2 Weeks)** ● Refactor next_equity_position to recognize and respect the enable_shorting flag. 

- ● Develop the PSQ (Inverse ETF) purchasing logic within the existing execution state machine. 

   - Implement the TOTP-secured Paper/Live runtime toggle in Axum AppState. 

- **Phase 4: AI Trading Advisor (Effort: 2 Weeks)** 

   - Create the engine/src/advisor.rs module and advisor_log audit table. 

   - Develop the prompt engineering pipeline, mapping Rust state variables into the text payload. 

   - Integrate the OpenRouter API (DeepSeek V4 Flash), ensuring the hourly briefing process operates asynchronously without blocking the execution threads. 

## **14. Risks & Open Questions** 

**OpenD Connectivity and Authentication:** The Moomoo OpenD daemon occasionally requires two-factor authentication (2FA) or image CAPTCHA solving upon restart<sup>2</sup> . While the futu-opend-docker setup allows for persistent volume mapping (futu-opend-data) to retain login sessions, unexpected daemon disconnects will blind the execution engine. _Mitigation:_ The Rust system must implement a strict watchdog ping over the port 11111 TCP connection. If the connection drops, the system must trigger an immediate fail-safe protocol, alerting the operator and halting new order generation until the session is manually restored via the port 22222 Telnet interface<sup>2</sup> . 

**Inverse ETF (PSQ) Tracking Error and Liquidity:** While an inverse ETF neatly sidesteps the complexities of margin borrowing, it is subject to compounding tracking errors over long durations and potential bid-ask spread widening during extreme market stress. _Mitigation:_ The strategy operates on a daily to monthly horizon (1D/5D/21D), which limits long-term compounding decay. However, the execution engine must evaluate the bid-ask spread via the Moomoo API prior to entry. If the spread exceeds a defined volatility threshold, the engine must abort the short entry and default to a safe Flat position. 

**LLM Hallucination Risk:** The AI Advisor may occasionally generate highly confident but _Mitigation:_ The logically flawed strategic adjustments, suffering from context hallucination. absolute decoupling of the advisory layer from the execution layer ensures safety. The LLM can only suggest parameters to be parsed by the Strategy Lab's backtester; it cannot directly write to the strategy_configs active state or influence the PaperExecutor. 

#### **Works cited** 

- - 

- 1. Protocol Introduction | Futu API Doc v10.8, htps://openapi.futunn.com/futu <u>api doc/en/fapi/protocol.html</u> 

- 

- 2. manhinhang/futu-opend-docker - GitHub, htps://github.com/manhinhang/futu - 

- <u>opend docker</u> 

3. Online Trading Platform, Commission-free Investment App & Brokerage - Moomoo, htps://www.moomoo.com/OpenAPI - - 

- - 

- 4. Introduction | Futu API Doc v10.9, htps://openapi.futunn.com/futu <u>api doc/en/</u> 

5. jasper/CLAUDE.md at main - GitHub, 

- <u>htps://github.com/xVanTuring/jasper/blob/main/CLAUDE.md</u> 

- 6. How to host SPA files and embed too with axum and rust-embed - Stack htps://stackoverfow.com/questions/73464479/how-to-host-spaOverflow, fles-and-embed-too-with-axum-and-rust-embed 

- 7. Axum cannot serve svelte files - help - The Rust Programming Language Forum, htps://users.rust-lang.org/t/axum-cannot-serve-svelte-fles/130509 

- 8. Best HTMX stack for Rust? - Reddit, <u>htps://www.reddit.com/r/htmx/comments/1d6m1f2/best_htmx_stack_for_rust/</u> 

- 9. Rhai: An embedded scripting language for Rust | Hacker News, <u>htps://news.ycombinator.com/item?id=42738753</u> 

10. I've tried integrating with mlua. It works, but Rhai is simpler to embed. Simp... | Hacker News, htps://news.ycombinator.com/item?id=42768533 

11. WebAssembly — list of Rust libraries/crates // Lib.rs, htps://lib.rs/wasm - - 

12. 查询持仓| Moomoo API 文档 v10.9, htps://openapi.moomoo.com/moomoo <u>api</u> - - 

<u>doc/trade/get position list.html</u> 

13. OpenRouter Pricing 2026: the Hidden 5.5% Fee, Itemized (Every Real Charge) - - - - - 

OfoxAI, htps://ofox.ai/blog/openrouter <u>pricing hidden markup</u> - 

<u>breakdown 2026/</u> 

14. OpenRouter API Pricing 2026: Full Breakdown of Rates, Tiers, and Usage Costs - - - - - - - 

Zenmux.ai, htps://zenmux.ai/blog/openrouter <u>api pricing 2026 full breakdown</u> - - - - - 

<u>of rates tiers and usage costs</u> 

15. OpenRouter Pricing 2026: Plans, Costs & Real Fees - CheckThat.ai, <u>htps://checkthat.ai/brands/openrouter/pricing</u> - - 

16. <u>htps://openapi.futunn.com/futu api</u> Trading Definitions | Futu API Doc v10.9, doc/en/trade/trade.html 

