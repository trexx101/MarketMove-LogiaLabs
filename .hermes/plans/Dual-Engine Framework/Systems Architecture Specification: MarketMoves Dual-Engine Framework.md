# Systems Architecture Specification: MarketMarkovNet Dual-Engine Framework

**System:** MarketMarkovNet (`feature/options-momentum-engine`)
**Target Environment:** Singapore OVH VPS · Rust (Axum 0.7, Tokio) · Local Moomoo OpenD Protobuf TCP · SvelteKit SPA (rust-embed)[cite: 2, 6]
**Objective:** Architecture definition for concurrently executing a Directional Momentum Engine (single-leg) and a Volatility Regime Engine (multi-leg) while adhering strictly to API quotas, database locking constraints, and state machine integrity.

---

## 1. Executive Summary
The MarketMarkovNet system is evolving from a pure swing-trading equities platform into a sophisticated, dual-engine options control room[cite: 2, 6]. The architecture seamlessly bridges **Engine 1 (Directional Momentum)**, which leverages a TCN/LightGBM ensemble predicting 1D/5D/21D trajectories, with **Engine 2 (Multi-Leg Volatility)**, which exploits $RV - IV$ (Realized vs. Implied Volatility) mispricings based on macroeconomic regimes. Both engines are governed by a unified Macro Risk layer, a Write-Ahead Execution Arbiter, and a staggered API polling scheduler designed to prevent broker quota exhaustion.

---

## 2. Architectural Topology & Separation of Concerns

The architecture runs as a single Rust binary containing the HTTP/WebSocket server, with a compiled-in Svelte frontend using `rust-embed`[cite: 2, 6]. The Python ML inference runs locally as a decoupled microservice via ZMQ V3[cite: 2, 6].

```text
                               ┌─────────────────────────────────────────────────────────┐
                               │           Moomoo OpenD Gateway (Local Docker)           │
                               │      Port 11111: Protobuf/TCP · Port 22222: Telnet      │
                               └───────────────────────────┬─────────────────────────────┘
                                                           │
                                   ┌───────────────────────┴───────────────────────┐
                                   │       Staggered Polling & Quota Governor      │
                                   │     (Max 30 requests / 30-second rolling)     │
                                   └───────────┬────────────────────────┬──────────┘
                                               │                        │
                                   ┌───────────┴────────────────────────┴──────────┐
                                   │      Macro Risk & Capital Arbiter (Global)    │
                                   │    (VIX level + 5d slope, FOMC Blackouts)     │
                                   └───────────┬────────────────────────┬──────────┘
                                               │                        │
                   ┌───────────────────────────┴─────┐    ┌─────────────┴───────────────────────────┐
                   │ ENGINE 1: DIRECTIONAL MOMENTUM  │    │ ENGINE 2: MULTI-LEG VOLATILITY          │
                   ├─────────────────────────────────┤    ├─────────────────────────────────────────┤
                   │ • Universe: QQQ, SMH, XLF       │ │ • Universe: Top-K Vol Equities (Tech/Disc)│
                   │ • Structure: Single-leg Call/Put│    │ • Structure: Straddle, Strangle, Condor │
                   │ • Alpha: TCN + LightGBM (1d/5d) │ │ • Alpha: IV vs HV Spread, Vol Skew        │
                   │ • Delta Target: ~0.45           │ │ • Delta Target: Delta-Neutral (0.00)      │
                   └──────────────────┬──────────────┘    └─────────────────────┬───────────────────┘
                                      │                                         │
                   ┌──────────────────┴─────────────────────────────────────────┴───────────────────┐
                   │                      ExitArbiter & State Machine                               │
                   │   (Write-Ahead Intent Log, Staged Ladder Exits, Circuit Breaker overrides)     │
                   └───────────────────────────────────┬────────────────────────────────────────────┘
                                                       │
                   ┌───────────────────────────────────┴────────────────────────────────────────────┐
                   │                        Persistence Layer (SQLite + Parquet)                    │
                   │       SQLite WAL: candles.db (Positions, Intent Logs, Mode Switches)           │[cite: 2, 6]
                   │       Parquet: 15s Ring-Buffer Tape Logs (Snappy/ArrowWriter)                  │
                   └────────────────────────────────────────────────────────────────────────────────┘

Module Boundary Layout (Rust Source Tree)
engine/src/
├── core/
│   ├── app_state.rs             // Global state, RwLock<TradingMode>, atomic shutdown[cite: 2]
│   ├── arbiter.rs               // Global Capital Arbiter (w_dir vs w_vol sizing)
│   └── calendar.rs              // FOMC / CPI / Earnings Blackout Calendar
├── data/
│   ├── moomoo/                  // Protobuf TCP Client, Rate Limiter, Staggered Poller
│   ├── db/                      // SQLite Connection Pool (WAL Pragmas, Migrations)
│   └── tape/                    // Parquet Arrow Writer, Ring Buffer
├── directional/                 // ENGINE 1 INTERNALS
│   ├── features.rs              // Exact 8-dim feature vector (indices 0..7)[cite: 7]
│   ├── model_bridge.rs          // ZMQ V3 IPC to Python TCN+LightGBM microservice[cite: 2, 7]
│   ├── chain_selector.rs        // 0.45 Delta single-leg Call/Put selector
│   ├── executor.rs              // Single-leg Staged Limit Order Ladder (Stage 1->3)
│   └── exit_arbiter.rs          // Trailing stop, Delta drift [0.15, 0.70], DTE < 7
├── volatility/                  // ENGINE 2 INTERNALS
│   ├── features.rs              // Volatility vector: IV Rank, IV-HV spread, Term Structure
│   ├── regime_detector.rs       // 4-Regime Classifier (Crisis, Recovery, Range, Complacent)
│   ├── scanner.rs               // Cross-Sector Volatility Ranking & Universe Pruning
│   ├── spread_builder.rs        // Multi-leg strike mapping (Straddle/Strangle/Butterfly/Condor)
│   ├── complex_router.rs        // Atomic / Staged Multi-Leg Order Router & Rollback Engine
│   └── exit_arbiter.rs          // IV Crush force-close, Vega decay, Leg-level risk manager
└── hyperopt/
    ├── directional_eval.rs      // Walk-forward Spearman Rank IC with Block Bootstrapping
    └── volatility_eval.rs       // Regime-conditioned Profit Factor & Max Gain/Loss Ratiom

    2. Universe & Asset Expansion FrameworkThe system dynamically partitions the trading universe into static directional index ETFs and a rotation pool of high-beta single-name equities and sector ETFs.  Asset Universe Partitioning

Total Active Universe = Directional Universe (Fixed 3 ETFs) + Volatility Universe (Active Top-K Pool)

| **Engine**      | **Segment**              | **Tickers**                                                 | **Selection Philosophy**                                                     |
| --------------- | ------------------------ | ----------------------------------------------------------- | ---------------------------------------------------------------------------- |
| **Directional** | Macro Index ETFs         | US.QQQ, US.SMH, US.XLF                                      | Structural liquidity, high open interest (OI), continuous trend persistence. |
| **Volatility**  | Sector Anchor ETFs       | US.XLK (Tech), US.XLY (Discretionary), US.XLE (Energy)      | Capture macroeconomic sector-rotation volatility spikes.                     |
| **Volatility**  | Tech / Growth Equities   | US.NVDA, US.AMD, US.AAPL, US.MSFT, US.NFLX                  | High historical kurtosis, major earning                                      |
| **Volatility**  | Cyclical / Beta Equities | US.BAC (Financials), US.BA (Industrials), US.F (Automotive) | Large mean-reverting regime shocks identified in the research paper.         |

Dynamic Universe Pruning & Cross-Sector Rotation Logic
To adhere strictly to broker connection quotas, the system does not record or evaluate all 12+ volatility equities simultaneously. At daily market close ($T_{\text{close}} - 15\text{m}$), the volatility::scanner executes the following algorithm:

1. Calculate Volatility Dispersion Metrics: For each candidate $s \in S_{\text{universe}}$, compute:$$\text{IV\_Rank}_{30\text{d}}(s) = \frac{\text{IV}_{30\text{d}}(s) - \min(\text{IV}_{252\text{d}}(s))}{\max(\text{IV}_{252\text{d}}(s)) - \min(\text{IV}_{252\text{d}}(s))}$$$$\Delta\text{Vol}(s) = \text{HV}_{20\text{d}}(s) - \text{IV}_{30\text{d}}(s)$$
2. Compute Catalyst Proximity: Identify if an earnings announcement or product release is scheduled within $t \in [3, 10]$ days. (Entries within $\le 2$ days remain blacklisted to prevent instant post-announcement IV collapse). 
3. Rank & Select Top-$K$ ($K=3$):$$\text{Score}(s) = w_1 \cdot \text{IV\_Rank}(s) + w_2 \cdot \left(\frac{\text{HV}_{20\text{d}}(s)}{\text{IV}_{30\text{d}}(s)}\right) + w_3 \cdot \text{CatalystScore}(s)$$
4. Active Slot Binding: The top 3 ranked assets are bound to the Volatility Engine's tape polling and execution slots for the subsequent trading session.
3. OpenD Quota & Network Budgeting
The Moomoo OpenD gateway enforces a strict account-tier subscription quota (assumed Tier 60 for accounts funded $> 100\text{k HKD}$) and a hard API rate limit of 30 requests per 30-second rolling window.

Protocol Strategy: Polling vs. Push
To bypass the subscription quota entirely and guarantee reliable Greeks retrieval (which are often omitted in UDP/TCP streaming pushes), the architecture uses Staggered Request/Response Polling via get_option_quote().

Request Allocation Budget (30-Second Window)

Total Capacity: 30 requests / 30 seconds (1.0 req/sec limit)
Target Operational Floor: <= 18 requests / 30 seconds (60% utilization, 40% safety headroom)

30-SECOND POLLING CYCLE
 0s                  5s                 10s                 15s                 20s                 25s                 30s
 ├───────────────────┼───────────────────┼───────────────────┼───────────────────┼───────────────────┼───────────────────┤
 [Group A: Dir ETFs]                     [Group B: Vol Assets]                   [Group A: Dir ETFs]
 (QQQ, SMH, XLF)                         (Top-3 Ranked Assets)                   (QQQ, SMH, XLF)
 6 Contracts Polled                      6 to 12 Contracts Polled                6 Contracts Polled
 (3 Calls + 3 Puts)                      (Spreads / Wings)                       (3 Calls + 3 Puts)
 (6 reqs)                                (6–12 reqs)                             (6 reqs)

 Group A (Directional Index ETFs): 3 underlyings $\times$ 2 contracts (1 Call + 1 Put at 0.45 Delta) = 6 contracts. Polled every 15 seconds = 12 requests / 30 seconds[cite: 4].
 Group B (Active Top-$K$ Volatility Assets): 3 active assets $\times$ up to 4 contracts per structure = up to 12 contracts. Polled once every 60 seconds (interleaved during Group A off-cycle ticks) = 6 requests / 30 seconds.
 Peak Utilization: $12 + 6 = 18\text{ requests} / 30\text{ seconds}$, leaving a buffer of 12 requests for real-time order placements, cancellations, and heartbeat checks[cite: 4].

 Staggered Scheduler Implementation (Rust Pattern)
 // engine/src/data/moomoo/poller.rs
use tokio::time::{interval, Duration};

pub async fn start_staggered_polling(
    client: Arc<MoomooClient>,
    directional_contracts: Arc<RwLock<Vec<String>>>,
    volatility_contracts: Arc<RwLock<Vec<String>>>,
) {
    let mut ticker = interval(Duration::from_secs(15));
    let mut cycle_counter: u64 = 0;

    loop {
        ticker.tick().await;
        cycle_counter += 1;

        // Group A: Directional Contracts (Every 15 seconds)
        let dir_batch = { directional_contracts.read().await.clone() };
        if !dir_batch.is_empty() {
            client.request_option_quotes_batched(&dir_batch).await;
        }

        // Group B: Volatility Multi-Leg Contracts (Staggered every 60s -> cycle % 4 == 0)
        if cycle_counter % 4 == 0 {
            tokio::time::sleep(Duration::from_millis(1500)).await; // 1.5s phase offset
            let vol_batch = { volatility_contracts.read().await.clone() };
            if !vol_batch.is_empty() {
                client.request_option_quotes_batched(&vol_batch).await;
            }
        }
    }
}

4. Data Storage, Database Schema & Retention (SQLite + Parquet)
To prevent SQLITE_BUSY deadlocks between high-frequency telemetry, trade operations, and background hyperopt analytics, persistence is strictly bifurcated between SQLite (relational state) and Parquet (dense tick storage).

Relational Schema Additions (data/candles.db)
All tables are created using strict Write-Ahead Logging (PRAGMA journal_mode = WAL;) and memory pragmas (PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;). Existing tables (equity_candles, equity_predictions, equity_trades, option_positions) remain structurally locked[cite: 2, 4].

-- DDL for Volatility Engine Relational Structures

-- Tracks composite multi-leg spread lifecycle
CREATE TABLE IF NOT EXISTS option_spread_positions (
    spread_id TEXT PRIMARY KEY,               -- UUID v4
    underlying TEXT NOT NULL,                 -- e.g., 'US.NVDA'
    strategy_type TEXT NOT NULL,              -- 'STRADDLE', 'STRANGLE', 'REV_BUTTERFLY', 'REV_CONDOR'
    regime_at_entry TEXT NOT NULL,            -- 'CRISIS', 'RECOVERY', 'RANGE_BOUND', 'COMPLACENT'
    dte_at_entry INTEGER NOT NULL,
    net_debit_credit REAL NOT NULL,           -- Positive = Debit Paid, Negative = Credit Received
    status TEXT NOT NULL,                     -- 'OPEN', 'EXITING_STAGED', 'CIRCUIT_BREAKER', 'CLOSED'
    entry_underlying_px REAL NOT NULL,
    entry_implied_vol REAL NOT NULL,
    entry_historical_vol REAL NOT NULL,
    unrealized_pnl REAL DEFAULT 0.0,
    realized_pnl REAL DEFAULT 0.0,
    exit_trigger_source TEXT,                 -- 'IV_CRUSH', 'WING_BREAKEVEN', 'DTE_OVERRIDE', 'STOP'
    opened_at INTEGER NOT NULL,               -- Unix Timestamp (ms)
    closed_at INTEGER                         -- Unix Timestamp (ms)
);

-- Tracks individual legs belonging to an active spread
CREATE TABLE IF NOT EXISTS option_spread_legs (
    leg_id TEXT PRIMARY KEY,                  -- UUID v4
    spread_id TEXT NOT NULL,                  -- Foreign Key -> option_spread_positions.spread_id
    contract_code TEXT NOT NULL,              -- e.g., 'NVDA260918C00120000'
    side TEXT NOT NULL,                       -- 'BUY' or 'SELL'
    option_type TEXT NOT NULL,                -- 'CALL' or 'PUT'
    strike REAL NOT NULL,
    ratio INTEGER NOT NULL,                   -- e.g., 2 for Butterfly body, 1 for wings
    fill_price REAL NOT NULL,
    current_bid REAL,
    current_ask REAL,
    current_delta REAL,
    current_vega REAL,
    is_closed BOOLEAN NOT NULL DEFAULT 0,
    FOREIGN KEY(spread_id) REFERENCES option_spread_positions(spread_id) ON DELETE CASCADE
);

-- Audit log for cross-sector volatility scanning & regime classifications
CREATE TABLE IF NOT EXISTS volatility_regime_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    underlying TEXT NOT NULL,
    vix_level REAL NOT NULL,
    vix_slope_5d REAL NOT NULL,
    implied_vol_30d REAL NOT NULL,
    historical_vol_20d REAL NOT NULL,
    iv_hv_spread REAL NOT NULL,
    detected_regime TEXT NOT NULL,
    action_taken TEXT NOT NULL                -- 'ALLOCATE_STRADDLE', 'ALLOCATE_STRANGLE', 'SKIPPED_EXPENSIVE'
);

Parquet Tape Storage Topology
Dense 15-second tick and quote logs bypass SQLite entirely and are flushed via an Arrow ring-buffer directly to disk in Apache Parquet format using Snappy compression[cite: 4].

data/options_tape/
├── directional/
│   ├── US.QQQ/
│   │   └── US.QQQ260918/
│   │       └── 2026-08-20.parquet[cite: 4]
│   └── US.SMH/
│       └── US.SMH260918/
│           └── 2026-08-20.parquet[cite: 4]
└── volatility/
    ├── US.NVDA/
    │   └── US.NVDA260918/
    │       └── 2026-08-20.parquet
    └── US.XLK/
        └── US.XLK260918/
            └── 2026-08-20.parquet


Partition Schema: 
data/options_tape/{engine_family}/{underlying}/{chain_code}/{YYYY-MM-DD}.parquet[cite: 4].
Row Structure (15s Resolution): 
timestamp_ns, bid, ask, last, volume, open_interest, implied_vol, delta, gamma, theta, vega, underlying_px[cite: 4].
Storage Footprint: 
Each contract produces $\approx 1{,}560\text{ ticks/day}$. Compressed row-group size $\approx 110\text{ KB}$ per contract/day. Tracking 18 active contracts consumes $\approx 1.98\text{ MB/day}$ ($\approx 495\text{ MB/year}$), remaining well within the VPS storage threshold.
Garbage Collection (GC) Policy: 
A weekly cron process archives raw tick Parquet files older than 90 days to compressed cold tarballs while retaining aggregated 5-minute candle aggregates indefinitely.

5. Feature Engineering & Regime Detection PipelineThe Volatility Engine does not predict directional price trajectory ($\text{sign}(\Delta S)$). It predicts volatility dispersion, kurtosis, and $RV - IV$ divergence to identify when options misprice the forward distribution of price returns.

Volatility Feature Vector Formulation
The volatility::features module computes a stationary 6-dimensional feature vector for each candidate symbol on every confirmed daily candle:

V(t) = [ f0: IV_Rank_252d, f1: Realized_Vol_Ratio, f2: Term_Structure_Slope, f3: VIX_Regime_Level, f4: VIX_Slope_5d, f5: Vega_Convexity_Ratio ]

// Feature definitions & mathematical representations
pub struct VolatilityFeatureRow {
    pub iv_rank: f64,               // (IV_30d - Min_IV) / (Max_IV - Min_IV) over 252 days
    pub rv_iv_spread: f64,          // (HV_20d - IV_30d) / IV_30d
    pub term_structure_slope: f64,  // ln(IV_60d / IV_30d) [Contango > 0, Backwardation < 0]
    pub vix_level: f64,             // Spot ^VIX close[cite: 7]
    pub vix_slope_5d: f64,          // (VIX_t - VIX_{t-5}) / VIX_{t-5}[cite: 4, 7]
    pub normalized_atr_14d: f64,    // ATR(14) / Close
}

Regime Classification & Strategy Mapping Matrix
Using empirical findings from the research paper, market environments are categorized into four operational volatility regimes, each mapped to a specific multi-leg payoff structure:

                                        REGIME CLASSIFICATION TREE
                                                  │
                             ┌────────────────────┴────────────────────┐
                             ▼                                         ▼
                     VIX >= 25.0                                  VIX < 25.0[cite: 7]
                             │                                         │
                 ┌───────────┴───────────┐                 ┌───────────┴───────────┐
                 ▼                       ▼                 ▼                       ▼
            VIX >= 30.0             VIX < 30.0        IV-HV >= 0.05           IV-HV < 0.05
         5d Slope > +10%         5d Slope <= 0%        (Persistent)           (Compressed)
                 │                       │                 │                       │
                 ▼                       ▼                 ▼                       ▼
          [ REGIME I ]            [ REGIME II ]     [ REGIME III ]          [ REGIME IV ]
        Crisis / Spike         Moderate / Recovery    Range-Bound             Complacent
                 │                       │                 │                       │
                 ▼                       ▼                 ▼                       ▼
           LONG STRADDLE           LONG STRANGLE    REVERSE BUTTERFLY      REVERSE CONDOR

| **Volatility Regime**                      | **Quantitative Detection Triggers**                          | **Strategy Deployed**                | **Exact Strike Structure**                                                                                                                                                                     | **Theoretical Edge**                                                                                            |
| ------------------------------------------ | ------------------------------------------------------------ | ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| **Regime I: Crisis / Outsized Volatility** | VIX $\\ge 30.0$ AND 5d VIX Slope $> +10\\%$                  | **Long Straddle**                    | • Buy 1 ATM Call ($K = S_0$)<br><br>• Buy 1 ATM Put ($K = S_0$)                                                                                                                                | Straddle yielded only positive net return in 2008 crash; tightest breakeven points overcome massive tail jumps. |
| **Regime II: Moderate / Recovery**         | $20.0 \\le \\text{VIX} < 30.0$ AND 5d VIX Slope $\\le 0.0$   | **Long Strangle**                    | • Buy 1 OTM Call ($K = S_0 \\times 1.05$)<br><br>• Buy 1 OTM Put ($K = S_0 \\times 0.95$)                                                                                                      | Maximizes reward-to-risk ratio (Avg Gain/Avg Loss = 165%) while keeping initial capital outlay low.             |
| **Regime III: Range-Bound / Stable**       | $12.0 \\le \\text{VIX} < 20.0$ AND IV/HV Spread $> 0.05$     | **Reverse Butterfly**                | • Sell 1 ITM Call ($K = S_0 \\times 0.95$)<br><br>• Buy 2 ATM Calls ($K = S_0$)<br><br>• Sell 1 OTM Call ($K = S_0 \\times 1.05$)                                                              | Highest frequency of positive periods ($324\\%$ win-to-loss ratio) due to narrow breakeven zones.               |
| **Regime IV: Complacent / Compressed**     | VIX $< 12.0$ (historically low) AND Realized Vol Compression | **Reverse Condor (or OTM Strangle)** | • Sell 1 ITM Call ($K = S_0 \\times 0.90$)<br><br>• Buy 1 ITM Call ($K = S_0 \\times 0.95$)<br><br>• Buy 1 OTM Call ($K = S_0 \\times 1.05$)<br><br>• Sell 1 OTM Call ($K = S_0 \\times 1.10$) | Provides strictly defined maximum loss when purchasing premium during extreme volatility compression.           |

6. Multi-Leg Strategy Execution & Microstructure Arbiter
Synthetic Atomic Order Router & Legging Mitigation
Retail brokerage interfaces like OpenD often lack native complex order routing (e.g., executing a 4-leg Butterfly in a single atomic fill). If one leg fills and liquidity suddenly dries up on adjacent legs, the portfolio is exposed to directional risk (unhedged Delta).

The volatility::complex_router executes an All-or-None Multi-Stage Legging Protocol:

MULTI-LEG ATOMIC EXECUTION PROTOCOL
                                        │
           Step 1: Compute Mid-Prices for All Legs (L1, L2, L3, L4)
                                        │
                                        ▼
           Step 2: Submit Limit Orders for Primary Bought Legs First
                   (Aggressive Mid + 0.01 Limit, IOC Timeout = 1500ms)
                                        │
                       ┌────────────────┴────────────────┐
                       ▼                                 ▼
                 [ALL FILLED]                     [PARTIAL / TIMEOUT]
                       │                                 │
                       ▼                                 ▼
           Step 3: Submit Limit Orders            Step 4: Emergency Rollback
                   for Sold Wings / Legs                  Immediately Unwind / Market-Close
                   (IOC Timeout = 1500ms)                 Filled Legs at Bid[cite: 4]
                       │                                 │
                       ▼                                 ▼
           [COMPLETE SPREAD LOGGED]               [LOG 'LEGGING_ABORT']

Dedicated Volatility Exit Arbiter (volatility::exit_arbiter)Unlike the directional engine (which tracks underlying trailing stops and Delta drift $[0.15, 0.70]$)[cite: 4], the Volatility Engine exits positions based on structural volatility criteria:

// Priority ranking for Volatility Exit Arbiter
pub enum VolatilityExitTrigger {
    ForceClose = 1,       // Operator manual deep-limit override[cite: 4]
    CircuitBreaker = 2,   // OpenD broker connection loss or stage-3 failure[cite: 4]
    HardExpiry = 3,       // Hard rule: Force-close when DTE < 7 to prevent final-week theta burn
    VolatilityCrush = 4,  // Implied Volatility drops > 30% from entry (post-event collapse)
    MaxWingTarget = 5,    // Underlying exceeds outer strikes (Maximum profit achieved)
    MaxLossExhaustion = 6 // Spread value has lost >= 80% of initial net debit
}

7. Portfolio Sizing, Capital Routing & Nightly Hyperopt
Dynamic Capital Allocation ModelThe global capital arbiter controls account allocation by dynamically shifting capital weights ($w_{\text{dir}}$ vs $w_{\text{vol}}$) based on systemic market stress ($VIX$)[cite: 4]:$$\text{Portfolio Cap} \le 30\% \text{ of Total Net Account Equity}$$

// engine/src/core/arbiter.rs
pub fn compute_position_budget(
    account_equity: f64,
    vix_level: f64,
) -> (f64, f64) {
    let max_deployed_capital = account_equity * 0.30; // Max 30% total deployed premium
    
    let (w_dir, w_vol) = if vix_level < 20.0 {
        (0.80, 0.20) // Normal/Bullish -> Focus on directional momentum
    } else if vix_level < 25.0 {
        (0.50, 0.50) // Transition/Moderate -> Balanced
    } else {
        (0.20, 0.80) // High Stress / Crash -> Focus on long volatility breakout
    };

    (max_deployed_capital * w_dir, max_deployed_capital * w_vol)
}

**Nightly Hyperopt & Decoupled Objectives**                                                                                                                                                  

The nightly hyperopt batch runs on idle CPU cores post-market and preserves strict mathematical boundaries between the two engines
**Directional Engine Evaluator (hyperopt/directional_eval.rs):**                                                                                                                             
Evaluates $1\\text{D}, 5\\text{D}, 21\\text{D}$ predicted vs realized returns                     
Enforces an Effective Sample Size (ESS) penalty to adjust for the 0.99 autocorrelation of the 200-day SMA.                                                                                   
Hard Deploy Gate: Mean Out-of-Sample Spearman Rank $\\text{IC} \\ge 0.03$ with all positive folds   
                                                                        
**Volatility Engine Evaluator (hyperopt/volatility_eval.rs):**                                                                                                                               
Evaluates multi-leg simulated trade trajectories against historical regimes.                                                                                                                 
Rejects Information Coefficient entirely (since non-directional spread returns have zero linear correlation with directional price returns).                                                 
 Hard Deploy Gate: $\\text{Profit Factor} \\ge 1.40$, $\\text{Max Gain / Max Loss Ratio} \\ge 1.50$, and $\\text{Positive Period Frequency} \\ge 50\\%$ across historical walk-forward folds. 

 8. Performance Data & Verification Metrics
Based on the research paper's empirical findings across 20 liquid US equities and 4 volatility regimes, the table below presents the simulated performance baseline expected when running the Dual-Engine Framework across distinct market conditions:

| **Market Regime**                              | **Dominant Engine**      | **Preferred Option Structure**                   | **Expected Win Rate (% Positive Periods)** | **Max Gain / Max Loss Ratio** | **Expected Sharpe Range** | **Primary Risk / Failure Mode**                                   |
| ---------------------------------------------- | ------------------------ | ------------------------------------------------ | ------------------------------------------ | ----------------------------- | ------------------------- | ----------------------------------------------------------------- |
| **Regime I: Crisis / High Vol<br>[cite: 1]**   | **Volatility Engine**    | Long Straddle[cite: 1]                           | 77.20%[cite: 1]                            | 163.51%[cite: 1]              | 1.80 – 2.20               | Extreme initial premium cost; rapid IV crush post-shock[cite: 1]. |
| **Regime II: Moderate Recovery<br>[cite: 1]**  | **Balanced Dual-Engine** | Long Strangle / 1D Momentum                      | 48.10% (Vol) / 54.0% (Dir)[cite: 1]        | 202.98%[cite: 1]              | 1.40 – 1.65               | False breakout whipsaws in directional engine[cite: 4].           |
| **Regime III: Range-Bound<br>[cite: 1]**       | **Directional Engine**   | Reverse Butterfly / Single-Leg Calls             | 68.00% – 117.47%\*[cite: 1]                | 62.32%[cite: 1]               | 1.10 – 1.35               | Slow theta bleed on unhedged options legs.                        |
| **Regime IV: Complacent Low Vol<br>[cite: 1]** | **Directional Engine**   | Single-Leg Momentum / Reverse Condor[cite: 1, 4] | 47.62% (Vol) / 58.0% (Dir)[cite: 1]        | 180.82% – 229.76%[cite: 1]    | 1.25 – 1.50               | Prolonged sideways chop underperforming cash[cite: 1].            |