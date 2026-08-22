# Multi-Model Sector-Split Orchestration

Concrete walkthrough of routing different sectors of a large task to different
models through OmniRoute, parsing the SSE output, and committing the result.

Learned in practice: MarketMoves Wave 5 edge-overhaul (2026-07-20).

## When to use

A task spans multiple skill domains (e.g. structural architecture + quant
math). One model is strong at scaffolding/interfaces (Gemini 3.1 Pro), another
at step-by-step reasoning (DeepSeek R1). The agent (Hermes) coordinates: writes
prompts, runs each model via curl, parses output, critiques, commits.

## Prompt structure (per model)

Each prompt should include:
1. ROLE + what to produce (scaffold, not full impl)
2. GROUNDING: paste the REAL code interfaces (struct defs, fn signatures,
   file paths) so the model scaffolds around actual code
3. HARD CONSTRAINTS (reuse infra, train==serve, etc.)
4. SECTOR ALLOCATION: explicit "SCAFFOLD NOW" vs "DEFERRED" sections
5. DEFERRED SECTORS → assign to: <model type>
6. DELIVERABLE FORMAT (file tree, trait signatures, stubs, DAG)
7. OUTPUT RULES (word count, no follow-ups, state assumptions)

## Runner script (reusable)

Write `/tmp/hermes-run-<model>-<task>.py`:

```python
import json, subprocess, sys
PROXY = "http://localhost:20128/v1/chat/completions"
KEY = "<proxy_password>"  # from config.yaml custom_providers.api_key
MODEL = "openrouter/google/gemini-3.1-pro-preview"
PROMPT_PATH = "/tmp/hermes-<model>-prompt.txt"
OUT_PATH = "/tmp/hermes-<model>-<task>.md"
PROMPT = open(PROMPT_PATH, encoding="utf-8").read()
payload = {
    "model": MODEL,
    "messages": [{"role": "user", "content": PROMPT}],
    "temperature": 0.4,  # 0.6 for R1
    "top_p": 0.95,
    "max_tokens": 12000,  # 8000 for R1
    "stream": True,
}
cmd = ["curl", "-sN", PROXY, "-H", f"Authorization: Bearer {KEY}",
       "-H", "Content-Type: application/json", "--max-time", "600",
       "-d", json.dumps(payload)]
proc = subprocess.run(cmd, capture_output=True, text=True)
raw = proc.stdout

content = []
reasoning = []  # R1 only
for line in raw.splitlines():
    line = line.strip()
    if not line.startswith("data:"):
        continue
    data = line[len("data:"):].strip()
    if data in ("[DONE]", ""):
        continue
    try:
        obj = json.loads(data)
    except Exception:
        continue
    ch = (obj.get("choices") or [{}])[0]
    d = ch.get("delta", {})
    if "content" in d and d["content"]:
        content.append(d["content"])
    if "reasoning" in d and d["reasoning"]:
        reasoning.append(d["reasoning"])

plan = "".join(content) if content else "".join(reasoning)
open(OUT_PATH, "w", encoding="utf-8").write(plan)
print(f"Plan: {OUT_PATH} ({len(plan)} chars)")
```

## Key differences: R1 vs Gemini 3.1 Pro

| Aspect | DeepSeek R1 | Gemini 3.1 Pro |
|--------|-------------|-----------------|
| SSE channels | `delta.reasoning` + `delta.content` | `delta.content` only |
| Temperature | 0.6 | 0.4 |
| max_tokens | 8000 | 12000 |
| Typical time | ~3-4 min | ~55s for 16K chars |
| Model id | `openrouter/deepseek/deepseek-r1-0528` | `openrouter/google/gemini-3.1-pro-preview` |

## Model ID discovery

```bash
curl -s http://localhost:20128/v1/models \
  -H "Authorization: Bearer <pw>" | tr ',' '\n' | grep -i gemini
```

Use the FULL provider-qualified id (e.g. `openrouter/google/gemini-3.1-pro-preview`),
NOT the proxy's internal alias (e.g. `aug/gemini-3.1-pro` — may 403).

## Cleanup

After committing the plan to `.omo/plans/`, delete all temp files:
```bash
rm -f /tmp/hermes-*-prompt.txt /tmp/hermes-run-*.py /tmp/hermes-*-raw.jsonl /tmp/hermes-*.md
```

## Post-run review checklist (LLM-generated code bug taxonomy)

LLM-generated code (R1, Gemini, etc.) almost always arrives with bugs.
The agent MUST review before writing to disk. Common failure modes observed
in practice (MarketMoves Wave 5 R1 D1-D3 pass):

1. **Missing dependencies / derives**: code references crates or derives not
   in the project manifest (e.g. `rand::random()` without `rand` in Cargo.toml;
   `..Default::default()` on a struct without `#[derive(Default)]`).
   Fix: rewrite the code to use only available dependencies, or add the dep
   to Cargo.toml if it's genuinely needed.

2. **Architecture / shape mismatches**: neural-net layers wired with
   incompatible dimensions (e.g. `nn.Sequential(nn.Linear(64, 3), nn.Linear(64, 1))`
   — the second layer expects 64 inputs but the first outputs 3).
   Fix: split into separate heads (`nn.ModuleList` of independent `nn.Linear`
   layers, not a `Sequential` chain).

3. **Logic errors in validation gates**: per-fold checks where mean-across-folds
   is needed; test assertions with wrong expected values (e.g. GARCH floor
   math — constant price converges to σ²=ω/(1-β), not 0).
   Fix: trace the math by hand, correct the assertion, verify against the
   actual computed value.

4. **Stubs left as `pass` / `todo!()`**: the model describes a function in the
   prompt but ships it as `pass` or `todo!()` in the code, expecting the agent
   to fill it.
   Fix: port the logic from the sibling implementation (e.g. Rust `vol_regime`
   → Python `vol_regime` for train==serve parity).

5. **Test data construction bugs**: pandas DataFrame built with lambda columns
   that don't evaluate (e.g. `'high': lambda x: x['open'] * 1.002` — lambdas
   aren't called by DataFrame constructor).
   Fix: use numpy arrays directly, not lambdas in dict construction.

After fixing all bugs: write the corrected code, `cargo build` / `py_compile`,
run unit tests, THEN commit. Note what was fixed in the commit message.

