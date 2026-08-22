# External LLM Code Review via OmniRoute Proxy

Used for dispatching structured code reviews to frontier models (Opus 5, Kimi K3)
through the OmniRoute proxy at `http://localhost:20128/v1/chat/completions`.

## When to use

When a focused architectural/correctness review of a codebase module would
catch bugs that local tooling (compiler, tests, lints) won't. Especially
valuable after multi-file feature merges when integration-level bugs are
likely.

## Proxy capabilities

The proxy lists models at `GET /v1/models`. The prefix `openrouter/anthropic/claude-opus-5`
routes to OpenRouter. Always pass `stream:false` — the proxy's streaming
format differs from OpenRouter's native SSE.

Working prefixes (tested 2026-08-13):
- `openrouter/anthropic/claude-opus-5` — $5/$25 per 1M tokens
- `openrouter/moonshotai/kimi-k3` — $3/$15 per 1M tokens (reasoning tokens bill as output)

The `openadapter/` prefix also works for some 0G models but routes through
a different provider chain.

## Prompt size limit

Opus 5 on this proxy takes ~4-5 minutes for a 26K-token prompt and times
out at 300s (5 min). Split prompts into **sub-passes of ~13K tokens each**
for reliable delivery. Each sub-pass costs ~$0.50 (input + output).

The split point should be natural boundaries: strategy.rs + config.rs in
one sub-pass, scheduler.rs + bridge.rs + paper.rs in another.

## Pass structure for a 4-module review

| Pass | Modules | Input tokens | Cost |
|------|---------|-------------|------|
| 1a | strategy.rs + config.rs | ~13K | ~$0.50 |
| 1b | scheduler.rs + bridge.rs + paper.rs | ~13K | ~$0.50 |
| 2 | equities_v2.rs + equity_model.py + parity.rs | ~15K | ~$0.55 |
| 3 | db.rs + api/*.rs + backtest.rs | ~20K | ~$0.70 |
| 4 | stores.js + websocket.js + docker-compose.yml | ~10K | ~$0.40 |
| 5-6 | Synthesis + follow-up Q&A | ~15K each | ~$0.60 each |

Total ~$3.85 for 6 passes. Well under a $10 budget.

## Construction pattern

Use `execute_code` to read files and construct the prompt, save to disk,
then dispatch via `terminal` with a 600s timeout:

```python
import json, os

# 1. Read files
files = {}
base = '/home/ubuntu/projects/MarketMoves/engine/src'
for fname in ['strategy.rs', 'config.rs']:
    with open(os.path.join(base, fname), 'r') as f:
        files[fname] = f.read()

# 2. Construct prompt with specific questions
prompt = f"""Code review focusing on [specific concerns].

QUESTIONS:
1. [Question 1]
2. [Question 2]

{'='*60}
{filename}.rs ({len(files[filename])} chars):
{'='*60}
{files[filename]}
"""

# 3. Save
with open('/tmp/review_pass1_prompt.txt', 'w') as f:
    f.write(prompt)
```

Then dispatch:

```bash
PROMPT=$(python3 -c "import json,sys; sys.stdout.write(json.dumps(open('/tmp/review_pass1_prompt.txt').read()))")
curl -s -X POST http://localhost:20128/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"openrouter/anthropic/claude-opus-5\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":$PROMPT}],\"max_tokens\":8000,\"temperature\":0.3}" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['choices'][0]['message']['content'])"
```

## Timeout handling

The terminal foreground timeout caps at 600s. For large prompts (>15K tokens),
use `terminal(background=true, notify_on_complete=true)` so the review runs
async and notifies when done. This is essential for Opus 5 which can take
5+ minutes on 20K+ token prompts.

## Review prompt design

The prompt should be **specific and adversarial**, not open-ended. Each
pass should ask 5-6 concrete questions about correctness, edge cases,
and scalability. The questions should reference known issues in the
codebase so the model can cross-check.

Effective question types:
- "Does function X have a logic bug when condition Y?"
- "What happens if field Z is NaN or zero?"
- "Is the concurrency pattern safe under scenario W?"
- "Would this scale to N concurrent instances?"
- "Is this consistent with the notebook's approach (cite specific cell)?"

Avoid: "Review this code." or "What do you think?"