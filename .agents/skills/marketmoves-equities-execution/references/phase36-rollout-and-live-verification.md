# Phase 3.6 Rollout & Live Verification

The trap: **code complete + tests green ≠ deployed ≠ live-verified.**
The Phase 3 skill body covers how to BUILD Phase 3. This file covers how to be sure it actually SHIPPED and is safe to flip live.

## 1. Distinguish three states — never assume the next one

| State | What it means | How to verify |
|-------|----------------|---------------|
| **Coded** | Files exist in working tree (tracked or untracked) | `git status --short` |
| **Deployed** | Code is running in the container binary, new image hash matches the just-built one | `docker inspect mmn-engine --format '{{.Image}}'` matches `docker inspect marketmarkovnet/engine:latest --format '{{.Id}}'` |
| **Live-verified** | Mode actually flipped paper→live, audit row in `mode_switches`, first live fill (or at least first live order) succeeded | see Section 4 |

Always state which state the user is asking about. When the user says "have we done X?", check all three.

## 2. Phase 3.6 rollout checklist (from PHASE_3_EXECUTION_SHORTING.md §3.6)

The deployment runs as **Docker Compose**, NOT systemd. Container is `mmn-engine`. Network is `deploy_mmn`. Volumes are `deploy_data` (SQLite) and `deploy_models` (read-only).

| Step | Done when | Where to check |
|------|-----------|----------------|
| 1. Deploy shorting logic (paper mode) | Image built from HEAD includes the Phase 3.1 commit; running container's image hash matches the newly built one; `equity_trades` rows include `symbol='PSQ'` after a backtest | `git log -1 --oneline`; `docker inspect mmn-engine --format '{{.Image}}'`; `GET /api/equity/data?symbol=PSQ&limit=10` (after a strategy tick) |
| 2. Deploy TOTP + mode toggle UI | `POST /api/mode` returns 200 against the running engine; UI's StatusPanel shows the `⇄` button next to the mode badge | `GET /api/mode` (no auth, returns JSON); `grep '<script src="/assets/index-' /` returns the Svelte bundle |
| 3. Test TOTP flow | Operator scanned the otpauth URL into an authenticator app, the running code is accepted by `POST /api/mode` | hands-on; can't script the QR scan. Persist `TOTP_SECRET=...` in `.env` immediately to avoid secret rotation on restart |
| 4. Live mode switch with minimal position size | Container state shows `trading_mode=live`; next QQQ candle that day triggered a LiveExecutor.order call (or no call if signal was flat — that's also valid) | `mode_switches` audit row + log line `trading mode flipped` |
| 5. Monitor `mode_switches` | Audit table is populated; toggle events have `authorized_by='totp:XXXXXX'` and `parity_marker_age_secs < 604800` | `SELECT * FROM mode_switches ORDER BY id DESC LIMIT 10;` via engine API or `docker run --rm -v deploy_data:/data alpine apk add sqlite && sqlite3 /data/candles.db "..."` |

## 3. Pre-live gate (don't skip — the engine honors it but you must remember it)

Phase 0+ deploy gate = `IC > 0.03 AND equity > 0 AND parity marker fresh`:

| Gate | Source of truth | Failure mode |
|------|-----------------|---------------|
| Walk-forward OOS mean IC > 0.03 | Colab runs in `training/` or V2 TCN eval; not auto-checked at runtime | Manual sign-off |
| Backtested equity curve positive | `engine::strategy_lab::backtest` results JSON; not auto-checked at runtime | Manual sign-off |
| Parity marker fresh (<7d) | `parity_verified.json` `verified_at` field | Engine refuses to START in `TRADING_MODE=live` |
| Moomoo OpenD daemon up + GUI unlocked | Manual; OpenD password entered each session | `place_order.py` returns `"未解锁"` / `"trade unlock needed"` error; engine surfaces warning |
| `TOTP_SECRET` persisted in `.env` or container env | `grep TOTP_SECRET .env` | After first run, the engine prints `warn!` with otpauth URL; if missing on restart, secret rotates and operator is locked out |

If any gate fails, flipping to live returns 403 with an actionable message OR the engine refuses to start (`Config::from_env` errors out). NEVER bypass by editing source.

## 4. Step 4 (live flip) playbook — concrete commands

The engine listens on `:8080` inside the `deploy_mmn` bridge network; only the Caddy proxy (`mmn-proxy`, ports 9080/9443) is reachable from the host. For headless flip you can curl from inside the network:

```bash
# On the VPS — pre-flight
cd ~/projects/MarketMoves
cat parity_verified.json | jq '.verified_at'                  # within 7 days
docker inspect mmn-engine --format '{{.State.Health.Status}}' # healthy
docker logs --tail 100 mmn-engine 2>&1 \
  | grep -i 'trading mode flipped'                           # none yet

# Get current TOTP from your authenticator
echo "Code from Google Authenticator: "; read TOTP

# Flip live (do this from inside the engine container OR via the proxy)
docker exec mmn-engine curl -sS -X POST http://127.0.0.1:8080/api/mode \
  -H 'Content-Type: application/json' \
  -d "{\"mode\":\"live\",\"auth_token\":\"$TOTP\"}" | jq .

# Or via the proxy:
curl -sS -X POST http://localhost:9080/api/mode \
  -H 'Content-Type: application/json' \
  -d "{\"mode\":\"live\",\"auth_token\":\"$TOTP\"}" | jq .

# Verify
curl -sS http://localhost:9080/api/mode | jq .
docker logs --tail 30 mmn-engine 2>&1 \
  | grep -i 'trading mode flipped'
```

The container has no `sqlite3` binary. To inspect `mode_switches` after a flip:

```bash
docker run --rm -v deploy_data:/data -v /tmp alpine:3.19 \
  /bin/sh -c 'apk add --quiet sqlite && sqlite3 /data/candles.db \
    "SELECT * FROM mode_switches ORDER BY id DESC LIMIT 3;" -header -column'
```

Wait for at least one equity-candle close to verify a real order attempt, then check `equity_trades` via the engine API (no shell binary needed) to confirm `symbol='QQQ'` (long) or `symbol='PSQ'` (short) rows.

## 5. Rollback

If live flip goes wrong:
- Flip back: same `POST /api/mode` with `{"mode":"paper", ...}` — **live→paper does NOT require parity marker** (asymmetric gating, see skill pitfall section).
- If executor is wedged (OpenD disconnect, order stuck), `docker restart mmn-engine` (or `kill -SIGTERM`). Live state is per-process: when the container restarts, it comes up in whatever `TRADING_MODE` env is set to — set that in `.env` (`TRADING_MODE=paper`) to guarantee paper on next boot.

## 6. What "live verification" means independent of Phase 4

Phase 4 (AI Advisor) is a pure add-on: `advisor_log` table + LLM chat endpoint + briefing widget. None of those touch the execution path. The Phase 3.6 + pre-live gate is **sufficient for live trading without Phase 4**. Verify by tracing the data flow:

- Strategy decision → `next_equity_position` returns Position.
- Scheduler reads `Arc<RwLock<TradingMode>>` + `Arc<RwLock<ExecutorKind>>`.
- Calls `executor.set_target_position(target, close, ts)`.
- MoomooExecutor shells out to `place_order.py` (REAL or SIMULATE).
- None of `advisor.rs`, `advisor_log`, SSE chat, briefing are on this path.

Conclusion: **Phase 4 is not a blocker for the first live flip.** If gate is met, you can go live today; advisor is additive polish.

## 7. Documentation drift watch

`README.md` at repo root has historically described the **BTC/Kraken/vanilla-JS** version of this project. Each major pivot (wave A equities, wave B/C, Phase 0-1 Svelte rewrite, Phase 2 Strategy Lab, Phase 3 PSQ/Moomoo/TOTP) left README partially or fully stale. When pivoting, refresh README last; when reading README, mentally diff against `AGENTS.md` and the latest `git log --oneline`.

**As of commit `53e08ae` (Phase 3 ship):** README was rewritten end-to-end around QQQ daily equities + Svelte Control Room + Moomoo OpenD. Operating-mode framing: PAPER is the headline, Moomoo is the future option, mode toggle is "wired + tested, not exercised on the running VPS yet". If you re-read the README and the framing differs from this, suspect a pivot landed without a README update.

## 8. Ad-hoc verification recipe (after every Phase-3 deploy)

There's no canonical test target for the deployed engine (no `make smoke` or `integration/`). Use an ad-hoc shell script in `/tmp` to verify the deployed behavior end-to-end. The script the team uses checks:

1. Container healthcheck reports `healthy` (the cosmetic fix pattern — `"mode":"paper"` regex match).
2. `/api/status` returns the Phase 3 schema (`mode`, `pred_1d`, `symbol`).
3. `/api/mode` returns the new payload (`parity_valid`, `parity_marker_age_secs`, `last_switch_ts`).
4. WebSocket upgrade at `/api/v1/ws` returns `HTTP/1.1 101 Switching Protocols` (use `curl -v` to see the status line — Caddy buffers the upgrade so the response is delayed, hence `timeout 5` not `timeout 1`).
5. The Svelte bundle: `curl /` returns the `<script src="/assets/index-HASH.js">` markup; `curl -I /assets/index-HASH.js` returns 200; `curl /assets/index-HASH.js | grep` finds Phase-3 strings (`TOTP`, `parity_valid`) inside the bundle.
6. Equity backfill coverage: `curl /api/equity/data?symbol=QQQ&limit=2500` returns `count>1000`; `?symbol=%5EVIX` returns `count>100`.
7. FRED span across the last 3 timeout log entries is ≤ 15 seconds (confirms the 5s timeout fix beat the prior 30s×3).
8. `git status --porcelain` is empty (changes committed, no in-flight debug left behind).
9. `git log --oneline -3` shows the expected commits.

Pipe `curl | grep` SIGPIPEs early on a 74KB bundle — always `curl -o /tmp/tmpfile; grep` or wrap the pipeline in `|| true` so the script reports failures honestly rather than masking them as SIGPIPE. Track results with `[PASS]` / `[FAIL]` markers and a summary line at the end; return non-zero on any failure so CI / shells exit cleanly.

The probe bugs to watch for:
- WS upgrade without `-v`: `curl` prints the progress meter first, making `head -1` capture `% Total` instead of the status line.
- `curl | grep` against the 74KB JS bundle hits SIGPIPE on early match — write to a temp file first.
- Yahoo `count` is `>=1000` not `==1251` — use `>1000` as the threshold, not equality.

Adapt the recipe by changing the regex / expected strings and re-running. Delete the script after capturing the run output.

## 9. When to use this file

User asks any of:
- "have we done Phase 3.6 / rollout / live verification?"
- "can we go live?"
- "is Phase 4 blocking the live flip?"
- "what's deployed on the VPS?"
- "the README is wrong / outdated"
- "we flipped to live but nothing is happening"
- "show me proof the deploy worked" / "verify the new UI is live" / "did my changes actually ship?"

→ Walk the user through sections 1, 2, 3 (status), or section 8 (after-deploy proof). Don't rebuild the rollout — verify state.
