# Local engine boot & smoke-testing API changes

Proven procedure after backend API changes — verifies endpoints for real, not just `cargo build`.

## Boot quirks (learned live 2026-08-19)

1. **Port 8080 collision is silent.** A stale engine (from a previous deploy/test) keeps 8080; your new binary fails to bind and exits. Check `ss -tlnp | grep 8080` FIRST and kill the old pid before booting. Testing new endpoints against the old binary returns "not found" for new routes — looks like a code bug but isn't.
2. **`NORM_STATS_PATH` env is required for local runs.** Default `models/norm_stats_qqq_v1.json` doesn't exist; actual files are `models/{QQQ,SMH,XLF}/norm_stats_<sym>_v1.json`. Without it the engine hard-exits before the HTTP server binds. Docker deploy passes it via env; local runs need it too:
   `NORM_STATS_PATH=models/QQQ/norm_stats_qqq_v1.json ./target/debug/engine`
3. **Startup takes ~8–10s** (CBOE VIX backfill ~1s, FRED macro backfill ~2s, DB migrations). Wait for `http server listening` in the log before curling; an immediate curl hits connection-refused or nothing.
4. **"engine configured" log lines are NOT proof of liveness.** Config parse succeeds even with a bad norm-stats path; the process dies later at stats load. Only `http server listening` means the API is up.

## Smoke-test recipe

```bash
cd /home/ubuntu/projects/MarketMoves
ss -tlnp | grep 8080            # kill stale pid if present
cargo build -q --bin engine
(NORM_STATS_PATH=models/QQQ/norm_stats_qqq_v1.json ./target/debug/engine > /tmp/engine_smoke.log 2>&1 &)
# wait for: grep "http server listening" /tmp/engine_smoke.log
for ep in "options/positions?limit=5" "options/config" "events?limit=5"; do
  curl -s "http://127.0.0.1:8080/api/$ep" | head -c 280; echo
done
# PUT round-trip: write a valid + an invalid key, expect partial apply
curl -s -X PUT http://127.0.0.1:8080/api/options/config \
  -H 'Content-Type: application/json' -d '{"risk_pct": 0.012, "fake_key": 1.0}'
# → {"applied":1,"rejected":["fake_key: unknown key"]}
# then RESTORE the original value — never leave test writes in the live config
kill <pid>
```

## Notes

- The 4 failing `config::tests` (env-leak: defaults_load_when_env_unset, live_mode_falls_back_to_paper, shorting_default_is_disabled, accuracy_returns_503_when_no_resolved) are pre-existing since before Phase 7 — don't chase them as regressions.
- `cargo test --lib` ≈ 351 tests, ~6s. After adding modules to lib.rs, also `cargo build --bin engine` — the bin crate tree is separate (see marketmoves-dev backend patterns).
