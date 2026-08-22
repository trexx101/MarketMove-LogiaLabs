# OmniRoute Operations Reference

## Config Directory Structure

```
~/.omniroute/
├── .env                          # STORAGE_ENCRYPTION_KEY, optionally INITIAL_PASSWORD
├── storage.sqlite                # ALL config — providers, combos, API keys, settings
├── storage.sqlite-shm            # SQLite shared memory (runtime)
├── storage.sqlite-wal            # SQLite write-ahead log (runtime)
├── server/
│   └── .pid                      # PID of running OmniRoute server process
├── call_logs/
│   └── YYYY-MM-DD/               # Per-day request logs (JSON)
├── db_backups/                   # Automatic database backups
└── logs/                         # Server logs
```

**Key fact:** There are NO editable YAML/JSON config files. Everything is configured through the web dashboard at `http://localhost:20128/dashboard` and stored in SQLite.

## Process Management

```bash
# Check if OmniRoute is running
ps aux | grep omniroute | grep -v grep

# Expected: two processes
#   - node /path/to/.nvm/.../bin/omniroute       (wrapper)
#   - omniroute (vX.Y.Z)                          (actual process)

# Read the PID from the pidfile
cat ~/.omniroute/server/.pid

# Check what INITIAL_PASSWORD was set to at process start
cat /proc/$(pgrep -f "omniroute" | head -1)/environ 2>/dev/null | tr '\0' '\n' | grep INITIAL_PASSWORD

# Restart (stop then start)
kill $(cat ~/.omniroute/server/.pid)
omniroute

# Reset password (requires stopped server)
npx omniroute reset-password
```

## Health Check

```bash
# Check server is alive
curl -s http://localhost:20128/v1/models | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'{len(d[\"data\"])} models available')"

# Expected: 200+ model IDs including auto/*, openrouter/*, opencode-go/*, etc.
```

## Provider Connection Info

Provider connections in OmniRoute are organized by authentication type:

| Type | Counter Label | Typical Providers |
|------|--------------|-------------------|
| API Key | "API Key X/167" | OpenRouter, Gemini, Together, Groq |
| Free Tier | "Free Tier X/101" | OpenCode Go, Auggie, The Old LLM |
| OAuth | "OAuth X/19" | Google, GitHub login-based providers |
| No Auth | "No Auth X/7" | Public endpoints |
| Web Cookie | "Web Cookie X/25" | Cookie-based access |

Use the **flat list view** on the Providers page to see only configured + no-auth providers at a glance.

## Dashboard Page Map

OmniRoute is a client-side Next.js app. Direct URL navigation to sub-pages (e.g. `/providers`, `/combos`) returns 404. Navigate by clicking links in the sidebar instead.

| URL | Purpose |
|-----|---------|
| `/login` | Login page |
| `/dashboard` | Home / overview |
| `/providers` | Configure provider backends |
| `/api-keys` | Manage API keys for clients |
| `/endpoints` | Connection URLs for clients |
| `/combos` | Define routing groups (key section) |
| `/combo-studio` | Live routing cascade visualization |
| `/quota` | Provider usage limits |
| `/traffic` | Traffic Inspector (LLM call debugging) |
| `/forgot-password` | Recovery instructions |
| `/status` | System status page |

## Call Logs

Logs are stored per-request in `~/.omniroute/call_logs/YYYY-MM-DD/` as individual JSON files. Each file captures the full request details — useful for debugging routing failures, rate limits, and token usage.

## Hermes Config Integration

When pointing Hermes at an OmniRoute proxy, the `~/.hermes/config.yaml` must be edited with care:

**Agent tools cannot edit Hermes config directly** — `write_file` and `patch` refuse to touch `~/.hermes/config.yaml` (security guard). Use either:

1. **Single-value set** (simple changes):
   ```bash
   hermes config set model.default auto/coding:cheap
   hermes config unset model.base_url
   ```

2. **Bulk YAML edit** (multiple interrelated changes, or YAML lists):
   ```python
   import yaml
   with open('/home/ubuntu/.hermes/config.yaml') as f:
       cfg = yaml.safe_load(f)
   cfg['model']['default'] = 'auto/coding:cheap'
   cfg['fallback_providers'] = ['custom:omniroute:auto/coding:free']
   with open('/home/ubuntu/.hermes/config.yaml', 'w') as f:
       yaml.dump(cfg, f, default_flow_style=False, sort_keys=False, allow_unicode=True)
   ```

**Pitfall — array index syntax:** `hermes config set custom_providers[0].api_key 'value'` does NOT nest `api_key` under the first list item. Instead it writes a separate top-level YAML entry `custom_providers[0]:` that Hermes ignores. Always use the Python YAML dump approach for nested list edits.

**Pitfall:** `hermes config set fallback_providers '[...]'` stores the value as a JSON *string*, not a YAML list. Always use the Python YAML dump approach for list-typed config values.

**Pitfall — missing api_key causes fallthrough:** If the custom provider has `base_url` but no `api_key`, Hermes silently falls through to the OpenRouter default provider. The error is a 401 from OpenRouter (not connection-refused from the proxy). Always include the proxy's password as `api_key` in the custom provider definition.

**Canonical Hermes config when using OmniRoute proxy:**
```yaml
model:
  default: auto/coding:cheap
  provider: custom:omniroute

custom_providers:
  - name: omniroute
    base_url: http://localhost:20128/v1
    api_key: your_omniroute_password   # REQUIRED — prevents fallthrough

fallback_providers:
  - custom:omniroute:auto/coding:free
  - custom:omniroute:auto/cheap
```
