# Multi-Model Orchestration for Quant Code Generation

When building a trading model overhaul, you can use external LLMs (reasoning models
for quant cores, structural models for scaffolding) to produce code that the agent
critiques, corrects, and commits. The user runs the training scripts on Colab and
reports results back.

## OmniRoute proxy model IDs (discovered 2026-07-20)

The OmniRoute proxy at `http://localhost:20128/v1` brokers OpenRouter models. Key
findings:

- `openrouter/deepseek/deepseek-r1-0528` → WORKS (provider: DeepInfra, streams)
- `openrouter/google/gemini-3.1-pro-preview` → WORKS (provider: Google, streams)
- `aug/gemini-3.1-pro` → WORKS (internal OmniRoute id)
- `tllm/openrouter_deepseek_r1` → FORBIDDEN (403 insufficient_quota)
- `auto/gemini` → alias, works

API key for the proxy: `100$$OmniRoute` (from `~/.hermes/config.yaml` under
`custom_providers: omniroute`).

To call from this environment:
```python
curl -sN http://localhost:20128/v1/chat/completions \
  -H "Authorization: Bearer 100$$OmniRoute" \
  -H "Content-Type: application/json" \
  -d '{"model":"openrouter/deepseek/deepseek-r1-05228","messages":[...],"stream":true}'
```

## Sector allocation (how to split work across models)

- **DeepSeek-R1** (reasoning model, temp 0.6, top_p 0.95): quant cores — GARCH vol,
  penetration labels, TCN training loop, walk-forward harness, deploy gate. These
  need step-by-step math reasoning + self-critique.
- **Gemini 3.1 Pro** (large context, temp 0.4): scaffolding — file trees, trait
  hierarchies, Rust interface signatures, build-order DAGs. Grounded with the
  real engine source code.
- **Claude Sonnet 4** (code review, temp 0.3): one-shot review of existing training
  Python files. Catches numerical bugs (GARCH constant, NaN propagation, magnitude
  outlier domination), architecture gaps (no residual connections, no scheduler),
  and methodology issues (no early stopping, no robust normalization). 90% as good
  as Opus 4 at 10% the price. See `references/claude-sonnet4-code-review.md` for
  the 8-issue pattern found in a real session.
- **Cheap/fast LLM** (e.g. Gemini Flash): plumbing — LLM regime adapter, caching
  logic, HTTP call shape. Low reasoning requirement.
- **The agent (you)**: critique + correct + commit. Both R1 and Gemini produce bugs
  that need validation before shipping.

**Model-router rule**: R1 for QUANT DESIGN (what to build), Claude Sonnet 4 for
CODE REVIEW (is what we built correct), Gemini for SCAFFOLDING (interfaces and
file trees). Using the wrong model for the task wastes tokens and produces
weaker output.

## R1 output bug patterns (seen in real sessions)

R1 produces strong architecture/code but consistently makes these Rust/Python errors:

1. **`..Default::default()` on structs that don't derive Default** — Rust structs
   like `Candle` with explicit fields don't derive Default. R1 test code uses
   `Candle { close: p, ..Default::default() }` which won't compile. Fix: spell out
   all fields or add `#[derive(Default)]` to the struct (not always appropriate).
2. **`rand::random()` in test code** — rand isn't always in Cargo.toml. Fix: use
   a deterministic test pattern (e.g. `if i % 2 == 0 { 1.02 } else { 0.98 }`).
3. **Hardcoded dict keys that don't match function parameters** — e.g. return dict
   hardcodes `directions[60]` when `horizons_bars=(1,4,24)` passed as argument →
   KeyError or silent mismatch. Fix: use the actual `horizons_bars` values as keys.
4. **TCN head architecture with mismatched Sequential dims** — `nn.Sequential(
   nn.Linear(hidden, 3), nn.Linear(hidden, 1))` — the second Linear expects `hidden`
   input but receives 3. Fix: separate `nn.ModuleList` heads, not a Sequential chain.
5. **GARCH test assertions that don't match the math** — constant-price test expects
   vol=0.0 but GARCH(1,1) with ω>0 floors at `sqrt(ω/(1-β))`, not 0. Fix: compute
   the expected floor value and assert against it.

## Validation protocol for LLM-produced code

1. **Rust files**: `cargo build --release` + `cargo test --release --lib <module>`
2. **Python files**: `python3 -m py_compile <file>` (syntax only; can't run torch/
   pandas without the Colab env)
3. **Self-tests**: note that pandas/torch/sklearn aren't in the agent env — these
   run only on Colab. Syntax-check is the max verification here.
4. **Function signature parity**: grep that the LLM-produced function signatures match
   the existing stubs exactly (e.g. `pub fn vol_regime(candles: &[Candle]) -> f64`)
5. After validation, commit with a message noting which bugs were fixed vs the LLM
   output.

## The "no LLM trains the model" clarifier

Users sometimes ask "which model does the training task?" — meaning which LLM runs
gradient descent. The answer: NO LLM trains. Gemini/R1 WRITE CODE. The user executes
the training script (train_tcn.py) on Colab (GPU/torch). Artifacts (.pt + .json)
drop into the repo; the Rust engine loads them. The engine never trains. Make this
explicit when the user asks about the training pipeline.