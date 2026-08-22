# <TASK> Planning Prompt for <EXTERNAL MODEL>

You are a senior <role>. Your job is to FLESH OUT the implementation plan for **<WAVE/SCOPE>** of
<PROJECT>. Read every constraint below as a HARD requirement unless explicitly marked "open decision"
or "replaceable". Do not invent data, APIs, or code that contradicts the locked contracts. Where you
propose changes, show exactly how they preserve the existing design.

LABEL YOUR SECTIONS CLEARLY:
- `(locked)` = true contract — must not break (DB schema, feature pipeline, normalization, inference
  protocol, deploy gate, serving DTO).
- `(reference — rewrite freely)` = current implementation for context; can be redesigned, replaced,
  or repurposed. Do NOT treat as a constraint to preserve.
- `(NEW — evaluate and recommend)` = capabilities the platform needs; recommend specific services/
  approaches with justification.

## 1. Project Context (locked facts)
- ...

## 2. LOCKED <CONTRACT NAME> (locked) — DO NOT REORDER/REMOVE
`function_or_type(...)` returns one row per <unit>. Fixed-order vector (indices 0..N), produced by
`to_array()`:

| idx | name | definition | range |
|-----|------|-----------|-------|
| 0 | ... | ... | ... |

- Normalization: <method, and that it must match at train+inference>. Serialized to <path>.
- Warmup: <missing lookback ⇒ 0.0 / NaN policy>.
- `<DIM_CONST> = N`. Training/inference MUST consume this exact vector in this order.

## 3. LOCKED MODEL BACKBONE (locked — port from <file>)
Reuse the proven architecture; only change <params>:
- ...

## 4. LOCKED LABEL / OBJECTIVE SCHEME (locked — port from <file>)
- ...

## 5. LOCKED WALK-FORWARD + DEPLOY GATE (locked)
- scheme: <TimeSeriesSplit / rolling / expanding> with embargo = <value>.
- Evaluate each fold OOS: <metric, e.g. IC = Pearson(pred, true)>.
- Deploy gate (HARD): train final model ONLY if <condition, e.g. mean OOS IC > gate AND equity > 0>.
  Gate value: <LOCKED value> — matches committed <file>.
- If gate fails: print "<NO EDGE>" and STOP.

## 6. LOCKED SERVING / STORAGE CONTRACT (locked — alignment target)
- DB columns: <...>. `source` should be like <...>.
- Existing bridge/serving path: <describe dormant vs active>. Wave adds <new path>.
- Current mismatch: <e.g. crypto 1h/4h/24h vs equities 1d/5d/21d> — resolve WITHOUT mid-flight break.

## 7. CURRENT IMPLEMENTATION (reference — rewrite freely)
- Current strategy params/logic: <describe defaults, e.g. "long/flat-only configured via
  entry_threshold/exit_threshold">. NOTE: these are defaults, not contracts. State what the type
  already supports (e.g. "Position enum supports Short but it's not wired into the equities path").
- Current UI/layout: <describe for context>. Can be fully redesigned.
- What to reuse vs replace: <e.g. uPlot charting, ES module pattern, color palette>.

## 8. API / THIRD-PARTY RECOMMENDATIONS (NEW — evaluate and recommend)
For each category the platform needs, recommend specific services with cost, free tier, Rust
integration approach (new module under `engine/src/data/`), and priority (MVP vs later):
- Real-time/intraday market data: <Polygon, Finnhub, Alpaca, Tiingo, etc.>
- Brokerage/execution: <Alpaca, IBKR, Moomoo, etc. — note shorting support>
- News/sentiment: <Alpha Vantage, Finnhub, Polygon, GDELT, etc.>
- Alternative data (optional): <options flow, social sentiment, earnings calendars>

## 9. YOUR TASKS (produce a concrete plan)
A) **Architecture**: specify <model A> + <model B> port; show how both consume the EXACT contract.
B) **Ensemble**: default = <method>; confirm or revise; show the math.
C) **Accuracy improvements that DO NOT break the design** (core ask). For each, state: what it
   changes, expected gain, and the EXACT mechanism that keeps the locked contract intact. Evaluate
   (accept/reject/extend) each candidate; add your own but they must preserve the base contract or be
   an additive EXTENSION.
D) **Alignment guarantees**: enumerate the locks keeping train↔inference↔serving consistent; call out
   current mismatches → resolution.
E) **Reproducibility / script structure**: mirror <reference script>; specify new script(s) and what
   they reuse vs rewrite.
F) **Future enhancement** (explicitly requested): propose ONE scoped improvement building on this
   design WITHOUT breaking the base contract (e.g. additive schema_version). Specify compat rule.

## 10. OPEN DECISIONS (state recommended default + rationale; user may override)
- D1 ...
- D2 ...
Mark defaults that are currently configured one way but could be extended (e.g. "shorting: currently
off, OD whether to enable via direct short / inverse ETF / options"). Don't lock config defaults.

## 11. OUTPUT FORMAT (so the plan is directly actionable)
1. Decisions (D1–Dn with chosen values + 1-line rationale each)
2. Model Architecture
3. Ensemble Method
4. Accuracy Improvements (table: improvement | changes | gain | design-safety mechanism | accept/reject)
5. Alignment & Contract Locks (incl. current mismatches → resolution)
6. Training Script Structure (file list + reuse/rewrite map)
7. Future Enhancement (one, scoped, with compat rule)
8. API / Third-Party Recommendations (table: category | service | cost | free tier | Rust integration | priority)
9. Risks / Open Questions for the user
Keep it precise and implementation-ready; cite the specific file/contract you are preserving.
