# Hyperopt runbook + self-waking scheduler loop

## Deployment runbook source of truth

`DEPLOYMENT_SUMMARY.md` (repo root) is the per-service redeploy guide. 6 services on VPS `vps-ab0567fd`:

| Service | Mechanism |
|---|---|
| mmn-engine | docker container, `--network deploy_mmn`, `-v deploy_data:/app/data`, `-v deploy_models:/models:ro`, `--env-file .env`, `--add-host host.docker.internal:host-gateway`. DB = `deploy_data` volume (`/app/data/candles.db`) — recreate containers, never the volume. |
| mmn-inference | docker, internal-only (no host port), mount `deploy_models` ro. |
| mmn-proxy | Caddy `caddy:2-alpine`, host 9080→80 / 9443→443, bind `deploy/Caddyfile`, volumes `deploy_caddy_data`/`deploy_caddy_config`. **Healthcheck 401s on the auth-gated route → always reports `unhealthy` = FALSE NEGATIVE; check logs for real 5xx.** |
| options-recorder | `systemctl --user` unit, host binary `target/release/options_recorder`, `After=opend.service`. Underlyings `US.QQQ,US.SMH,US.XLF`, DTE 30–45, delta 0.45, **quota tier 20**, engine via `http://127.0.0.1:9080`. |
| opend | `systemctl --user` unit, `-login_account=104483875 -login_by_remember=1` (auto-login from cached `Broker.dat`, NO password in unit). |
| hermes-gateway | `systemctl --user` unit (messaging). |

Gotchas encoded in the runbook: **docker-compose v1.29.2 is broken** on this VPS (`KeyError: 'ContainerConfig'`) → use raw `docker run`; Caddy auth password is not in the repo (verify internally via `docker run --rm --network deploy_mmn ...`); inference z-score cold-starts at `0.0` for ~2 bars post-restart (not a bug).

## The self-waking scheduler loop bug (worst pitfall in this codebase)

Symptom: the nightly hyperopt loop **never fired** — booting the engine produced no `Starting nightly hyperopt run` line, ever.

Wrong pattern (what was shipped first):
```rust
loop {
    let next = sched.next_run_time(now);            // returns the window *START* (20:30 UTC)
    let wait = (next - now).min(Duration::from_secs(1800));
    sleep(wait).await;                              // capped poll
    runner.run().await;                             // never reached!
}
```
Why it never fires: `next_run_time` returns the next window *start instant*. When `now` is before the start it returns today at 20:30; at/past 20:30 it returns **tomorrow** at 20:30. A capped 30-min poll never lands *exactly* on `:30:00`, and every wake recomputes `next` still in the future → `runner.run()` is unreachable forever.

Correct pattern: **poll a fixed interval, gate on state, run once per window identity**:
```rust
let mut last_window: Option<chrono::NaiveDate> = None;
loop {
    let now = chrono::Utc::now();
    let window = sched.window_start_date(now);      // date of the window's opening instant
    if last_window != Some(window) {
        match runner.run().await {                  // run() SELF-GATES on CanRun (safe to call often)
            Ok(r) if r.success => last_window = Some(window),   // once per window
            _ => {}                                  // not CanRun → retry next poll
        }
    }
    sleep(Duration::from_secs(1800)).await;
}
```
Design contracts that make this safe:
- `runner.run()` self-checks `check_state` and no-ops outside the window — you can call it on every poll.
- Exactly-once-per-window guard is **mandatory**, not cosmetic: `CandidateStore.store()` mints a **fresh UUID every call** (no dedupe) and `run()` returns `success` even when it ran, so a run-every-poll loop would duplicate candidate rows all night.
- `window_start_date(now)` must be **midnight-wrap aware**: the window spans [20:30, 04:30) UTC across midnight. Naive `now.date_naive()` is wrong for post-midnight times (02:00 later that night belongs to the previous day's window). If `now`'s time-of-day < earliest_start (20:30) → window opened **yesterday**. Unit test covers: 21:00 → that date; 02:00 next day → still that date; 12:00 → previous day.

## Verify a run fired

```bash
docker logs mmn-engine | grep -E 'hyperopt run|Starting nightly|Nightly run complete'
```
In-window deploy fires immediately at boot (loop's first iteration). Otherwise it waits for the next 20:30 UTC open. Also confirmed via API:
```bash
docker run --rm --network deploy_mmn caddy:2-alpine wget -qO- http://mmn-engine:8080/api/hyperopt/QQQ/status
# {"equity":"QQQ","pipeline_state":"idle","total_candidates":9,"by_status":{"NEW":9}}
```

## Night run facts

- Window 20:30→04:30 UTC (SchedulerConfig: timezone_offset +8; market_open 21:30 local / close 04:00 local; buffers 30 min; max_run_hours 8).
- 3 equities × `sma_regime` grid over ~1500 candles ≈ 3× the single-symbol cost — still fits the window.
- RunnerConfig::default().equities = QQQ/SMH/XLF drives **both** the candidate producer and the promotion applier in main.rs (same default). One source of truth.

## OPEN — negative rank-IC across all equities (unresolved)

First live run stored 27 candidates (9 per equity: QQQ/SMH/XLF) all with `mean_ic ≈ -0.07…-0.29` — **systematically negative**, not noise. Either SMA-momentum genuinely mean-reverts over the 5-day forward horizon for these ETFs, or there is a **sign inversion** in the eval objective (`engine/src/hyperopt/eval.rs`: walk-forward rank IC, Spearman avg-rank ties, 136-day embargo, min_bars=400, min_trades=100, IC_GATE=0.03). All candidates fail the IC gate (≥0.03) and will not promote. Do NOT trust stored candidates until this sign is resolved — before exporting a promotion, sanity-check whether a high SMA-momentum value should predict up or down next-5d for the target.