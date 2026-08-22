# Rust: dotenvy re-injection + cargo test filtering (from a MarketMoves session)

## The pitfall
`Config::from_env()` calls `dotenvy::dotenv()` on EVERY invocation. dotenvy
reads the root `.env` and only sets vars that are currently UNSET in the process
env. In a batch `cargo test` run this causes order-dependent contamination:

- Test A clears `SYMBOL` (via `env::remove_var`), asserts default == `"BTC/USD"`.
- `from_env()` runs, calls `dotenvy::dotenv()` → re-sets `SYMBOL=QQQ` from `.env`
  (because A had removed it, it's now unset → dotenv re-injects it).
- Test A now sees `SYMBOL=QQQ` and fails.

Result: the ENTIRE `config::tests` module fails in batch, even though each test
passes in isolation (`cargo test -p engine --lib config::tests::shorting_enabled_via_env`).
This is a pre-existing harness bug, NOT a logic bug in the change under review.

## How to verify a config change cleanly
1. Run the SPECIFIC test alone to prove logic:
   `cargo test -p engine --lib config::tests::<name>`
2. Run a substring filter to batch related tests without the contaminated module:
   `cargo test -p engine --lib -- short`  (matches `equity_short_*`, `short_*`, `next_position_*_short_*`)
3. For the full suite, neutralize `.env` for the run only:
   `mv .env .env.bak && cargo test -p engine --lib config::tests; mv .env.bak .env`
   → reveals the remaining failures are the genuine pre-existing assertion drift
   (e.g. `norm_stats_path` default `"models/..."` vs test-expected `"/models/..."`).
4. Prove your change added NO new failures by diffing clean vs branch:
   ```
   git stash push -- <your files>
   cargo test -p engine --lib 2>&1 | grep FAILED | sort > /tmp/clean.txt
   git stash pop
   cargo test -p engine --lib 2>&1 | grep FAILED | sort > /tmp/mine.txt
   diff /tmp/clean.txt /tmp/mine.txt   # empty diff = you broke nothing
   ```

## cargo test filter semantics (frequent confusion)
- `cargo test --lib -- a b c` → runs tests whose path matches ANY of a/b/c (OR).
- After `--`, args are test-name substrings, not file paths.
- `cargo test <ONE>` accepts only ONE positional filter; to run multiple named
  tests, use the substring form or repeat with separate invocations.

## paper.rs note (MarketMoves) — PSQ remap is DONE (Phase 3.2)
`PaperExecutor` now expresses a short by **buying PSQ** (inverse ETF), not a
traditional short-sell. For BOTH long and short: entry leg = `TradeSide::Buy`,
exit leg = `TradeSide::Sell`; only the `symbol` field differs (`QQQ` vs `PSQ`).
The strategy's `next_equity_position` enforces the two-step guard (never returns
`Short` directly from `Long`), so the executor produces the right two fills
(`Long→Short`: Sell QQQ, Buy PSQ; `Short→Long`: Sell PSQ, Buy QQQ).
Fills persist to `equity_trades` (symbol-aware) via `db::insert_equity_trade`,
NOT the legacy `trades` table. Full fill matrix + PnL formula:
`marketmoves-equities-execution` skill, `references/psq-inverse-etf-remap.md`.
