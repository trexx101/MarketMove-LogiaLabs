# One-off direct call to a named upstream model via OmniRoute

Use when you want to route a single prompt to a specific stronger/cheaper model than
the current session (e.g. hand an architecture/research prompt to DeepSeek-R1 while the
session runs on a free model), through the OmniRoute proxy already configured in
`~/.hermes/config.yaml`.

## Gotchas (confirmed 2026-07)
- Model id MUST be the provider-qualified OpenRouter id, e.g.
  `openrouter/deepseek/deepseek-r1-0528`. The proxy's internal alias id from
  `/v1/models` (e.g. `tllm/openrouter_deepseek_r1`) returns `403 Forbidden`.
- Bearer token = the proxy password (the `api_key` value under `custom_providers`).
- Reasoning models stream `delta.reasoning` (CoT) separately from `delta.content`
  (final answer). Keep `content`; only fall back to `reasoning` if content is empty.
- Skip non-`data:` SSE lines (the stream opens with `: OPENROUTER PROCESSING`).
- `stream:true` + `--max-time 600` avoids timeouts on long generations.

## Reusable harness (write to /tmp, run, then clean up)

```python
#!/usr/bin/env python3
# Streams an OmniRoute -> OpenRouter model call, splits reasoning vs final content.
import json, subprocess

PROXY = "http://localhost:20128/v1/chat/completions"
KEY   = "<proxy_password>"                       # from custom_providers.api_key
MODEL = "openrouter/deepseek/deepseek-r1-0528"
PROMPT = open("/tmp/hermes-prompt.txt", encoding="utf-8").read()

payload = {"model": MODEL,
           "messages": [{"role": "user", "content": PROMPT}],
           "temperature": 0.6, "top_p": 0.95, "max_tokens": 8000, "stream": True}
cmd = ["curl", "-sN", PROXY,
       "-H", f"Authorization: Bearer {KEY}",
       "-H", "Content-Type: application/json",
       "--max-time", "600", "-d", json.dumps(payload)]

raw = subprocess.run(cmd, capture_output=True, text=True).stdout
reasoning, content = [], []
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
    ch = obj.get("choices") or []
    if not ch:
        continue
    d = ch[0].get("delta", {})
    if d.get("reasoning"):
        reasoning.append(d["reasoning"])
    if d.get("content"):
        content.append(d["content"])

final = "".join(content) or "".join(reasoning)
open("/tmp/hermes-model-output.md", "w", encoding="utf-8").write(final)
print(f"reasoning={len(''.join(reasoning))} chars  final={len(final)} chars")
print(final[:1500])
```

## Discover model ids
```bash
curl -s http://localhost:20128/v1/models -H "Authorization: Bearer <pw>" \
  | tr ',' '\n' | grep -i deepseek
```
