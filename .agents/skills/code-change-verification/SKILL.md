---
name: code-change-verification
description: Verify a focused change when the test suite is partly red.
---

# Code Change Verification (under a partially-broken suite)

## When to use
- You changed a few files in a mature codebase and need to confirm the change works.
- The full `cargo test` / `pytest` / `npm test` is partly red for reasons unrelated to your change (pre-existing failures, env contamination, flaky tests).
- You were asked to "verify", "confirm it works", "show evidence", or produce FRESH verification — not a recap of an earlier run.

## Why this skill exists
A common failure mode: an agent claims "tests pass" by recollecting an earlier inline command, or runs the whole suite and hand-waves the red lines as "not mine". Both are weak. The user wants **fresh, self-contained, reproducible evidence** that the specific change is green, with a clear separation between what you broke and what was already broken.

## Steps
1. **Map the blast radius.** List every module/function you touched. Your verification targets those.
2. **Run targeted, not whole-suite.** Scope the test run to your change:
   - Rust: `cargo test -p <crate> --lib -- <name-substring>` (substring filter on the test path).
   - Python: `pytest path/to/test_file.py::test_your_thing`.
   - Avoid running the entire suite as your first move — it buries your signal in pre-existing noise.
3. **Write a self-contained verification script.** Put it in /tmp (e.g. `/tmp/verify-<topic>.sh`), have it build + run the targeted tests and tee output to a file, then `cat` the file. Run the script and present its **actual output**, not a paraphrase.
4. **Neutralize confounding shared state for the run only.** If batch runs leak env/state (see references/rust-cargo-dotenvy.md), move the offending file aside (`mv .env .env.bak`) for the run and restore after. Do NOT bake the workaround into your code change unless it is in scope.
5. **Prove pre-existing failures are not yours.** Diff the failure set on clean tree vs your branch:
   - `git stash push -- <your files>`, run the suite, capture failures to /tmp/clean.txt.
   - `git stash pop`, run again, capture to /tmp/mine.txt.
   - `diff /tmp/clean.txt /tmp/mine.txt` — if identical (plus only your intended new tests), your change introduced nothing new.
6. **Run a fresh ad-hoc verification harness AFTER every behavioral commit, not just the last one.** Inline `cargo test --lib -- <filter>` covers the test surface. The standalone harness (under `/tmp/hermes-verify-<feature>-<YYYY-MM-DD>/`) exercises the public API surface — things that may not be called by any test. Each new API surface, new DB table, or new behavior deserves its own harness run. Don't reuse the same crate across commits — the point is fresh, self-contained evidence per change. Delete the crate after the run.

   **Scratch-crate `Cargo.toml` must opt out of the parent workspace.** When the harness lives under `/tmp/...` and the engine is part of a Cargo workspace (the common case for repos like MarketMoves), the harness crate will try to JOIN that workspace because Cargo discovers the parent `Cargo.toml` by walking up the directory tree. Add an empty workspace declaration at the top of the harness `Cargo.toml` to prevent this:

   ```toml
   [package]
   name = "hermes-verify-<feature>-<date>"
   version = "0.1.0"
   edition = "2021"

   [dependencies]
   engine = { path = "/home/ubuntu/projects/MarketMoves/engine" }
   tokio = { version = "1", features = ["full"] }
   sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio", "macros"] }

   # CRITICAL — prevent this scratch crate from joining the parent workspace.
   [workspace]
   ```

   Without `[workspace]`, you get errors like `current package believes it's in a workspace when it's not` or path-resolution failures unrelated to your actual code. Same applies to `pytest` projects that share a parent `pyproject.toml` — use `[tool.uv]` / isolated venv setup or pass `--no-workspace`.

   **Pull in `sqlx` with the right feature set** when the harness touches DB code. The minimum is `features = ["sqlite", "runtime-tokio", "macros"]`. Without `macros`, `sqlx::query!()` fails; without `runtime-tokio`, the pool connect fails.
7. **Report honestly.** State: what changed, the exact verification command + its real pass/fail counts, and explicitly which failures are pre-existing/unrelated. Never claim "full suite green" unless it actually is.

## Pitfalls
- **Don't re-describe earlier output as evidence.** The user explicitly wants fresh, runnable artifacts. Write the script, run it, show it.
- **Don't silently "fix" unrelated test-infra debt.** Flag it and offer to fix separately. Absorbing it into your feature change obscures the diff and the review.
- **Test-order contamination is real.** A test that loads env files / sets process globals can poison later tests in the same binary. Confirm a "failing" test by running it ALONE, then in batch. Contamination ≠ logic bug.
- **Compile-check separately.** `cargo build -p <crate>` catches broken references (e.g., a struct gained fields and a direct constructor elsewhere now fails to compile) faster than a full test build.

## Verification of THIS skill's outcome
- Targeted filter run: 0 failures among tests touching your change.
- Optional clean-tree diff: no NEW failures beyond your intended additions.

References: see references/rust-cargo-dotenvy.md for the dotenvy re-injection pitfall and cargo filter recipes.
