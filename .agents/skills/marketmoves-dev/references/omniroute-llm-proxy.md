# OmniRoute Proxy for LLM features

The VPS runs an OmniRoute proxy at `localhost:20128` that provides free access
to many LLM models via the OpenAI-compatible API. It's the preferred backend for
any LLM feature in the MarketMoves engine (advisor, regime cache, etc.).

## Endpoint

```
http://localhost:20128/v1/chat/completions
```

No API key required. The proxy listens on `0.0.0.0:20128` so it's reachable from
any network interface.

## Available models

Run `curl -s http://localhost:20128/v1/models | python3 -m json.tool` to see the
current model list. Common useful ones:

- `auto/best-coding` — best coding model, auto-routed
- `auto/best-chat` — best conversational model
- `auto/cheap` — cheapest model, good for simple classification
- `auto/gemini` — Google Gemini models
- `auto/claude-sonnet` — Anthropic Claude Sonnet

## Docker networking — reaching the proxy from inside a container

The engine runs inside a Docker container on the `deploy_mmn` bridge network
(172.18.0.0/16). The OmniRoute proxy runs on the host. `localhost` inside the
container refers to the container itself, not the host.

**The fix**: add `extra_hosts` to the engine service in `docker-compose.yml`:

```yaml
engine:
  extra_hosts:
    - "host.docker.internal:host-gateway"
```

Then use `http://host.docker.internal:20128/v1/chat/completions` as the API base.

**Pitfall**: `reqwest` (the Rust HTTP client) may fail to resolve
`host.docker.internal` even when `curl` from the same container works. This
appears to be a DNS resolution issue specific to reqwest on Debian Bookworm
inside Docker. The root cause is unresolved as of 2026-08-02.

**Workaround**: use the Docker bridge gateway IP directly. The gateway for the
`mmn` network is `172.18.0.1`. This IP is reachable from the container and
routes to the host. However, reqwest may also fail to connect to this IP.

**Current status**: the engine uses OpenRouter directly (`https://openrouter.ai`)
for LLM calls, which works from inside Docker. The OmniRoute proxy is available
for local testing (run the engine binary directly on the host) but not yet
reliably reachable from reqwest inside Docker.

## Streaming vs non-streaming

The OmniRoute proxy returns SSE (Server-Sent Events) streaming by default. To
get a plain JSON response, add `"stream": false` to the request body:

```json
{
  "model": "auto/best-coding",
  "messages": [{"role": "user", "content": "Hello"}],
  "stream": false
}
```

Without `stream: false`, the response body is SSE-formatted (`data: ...` lines)
and won't parse as JSON.

## OpenRouter fallback

When the OmniRoute proxy is unreachable (e.g. from inside Docker), the engine
falls back to OpenRouter directly. The `OPENROUTER_API_KEY` env var is required.
OpenRouter model IDs use the format `provider/model-name` (e.g.
`google/gemini-2.5-flash-lite`), NOT `openrouter/provider/model-name`.

OpenRouter requires credits — free tier models may work without credits, but
most models return 402 "Insufficient credits" if the account balance is zero.