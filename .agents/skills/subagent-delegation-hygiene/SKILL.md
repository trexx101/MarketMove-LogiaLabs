---
name: subagent-delegation-hygiene
description: "Baseline + scope + verify for delegated coding work."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [subagent, delegation, verification, baseline, scope-creep, codex, claude-code, opencode]
    related_skills: [claude-code, codex, opencode, requesting-code-review, plan, test-driven-development]
---

# Subagent Delegation Hygiene

Subagents (whether `delegate_task` leaf workers or CLI agents like Claude Code /
Codex / OpenCode) save orchestration time but introduce a class of failure the
requester must catch. This skill covers the three things the requester must do
before/during/after dispatch that the subagent itself cannot be trusted to do.

**Core principle:** A subagent's "verified" claim is a report, not a proof.
The requester owns the verification loop.

## When to Use

- Delegating any code change that touches 3+ files or 2+ modules.
- Any task where the working tree already has uncommitted changes (in-flight
  multi-stage project).
- Any task where the subagent must add tests AND touch production code.
- Whenever the requester says "verify the subagent's work" or "the subagent
  said it passed but I want to confirm."

**Skip for:** single-file edits, pure docs, one-line config tweaks, or when
the subagent is asking a question (not making changes).

## Step 1 — Pre-dispatch Baseline (REQUIRED)

Establish `baseline_passed` / `baseline_failed` counts on the exact target the
subagent will touch. Pre-existing failures must be subtracted from any
post-dispatch result.

```bash
# Example: Rust crate
git stash && cargo test -p <crate> --lib 2>&1 | grep "test result"
git stash pop
```

If the working tree already has uncommitted changes (common in multi-stage
projects), the stash above contradicts the baseline. Two options:

1. **Commit the in-flight work first**, then baseline.
2. **Document the current `cargo test` counts in the delegation prompt** as
   the explicit baseline. The subagent must end with
   `post_passed - baseline_passed == expected_new_passes` and
   `post_failed <= baseline_failed`.

If the subagent's diff introduces *new* failures or breaks a previously
passing test, that is a regression — even if the absolute failure count is
the same.

## Step 2 — Bounded Scope in the Prompt

Subagents (especially leaf roles) will rewrite adjacent code if the prompt is
ambiguous. Always include a negative scope:

```
DO NOT touch:
  - strategy.rs
  - executor.rs
  - db.rs

You may only modify:
  - api.rs (add fields, add 2 tests)
  - index.html (rename ids)
  - views/predictions.js (render new ids)
  - style.css (add .row--scaled rule)
```

Also include in the prompt:

- **Exact test names** the subagent must add (the requester will assert on
  these exact names).
- **Exact assertion values** for the new tests, so the subagent doesn't
  invent numerical targets.
- **Helper functions to reuse** by name. Subagents invent helpers when they
  don't know what's already exported.
- **Absolute paths** for any file mentioned. Relative paths from the
  subagent's CWD may not match the requester's.

## Step 3 — Post-dispatch Verification

### 3a. Build / compile

```bash
cargo build -p <crate>      # or equivalent
```

Must finish cleanly. Any compile error is a stop.

### 3b. Targeted test run

```bash
cargo test -p <crate> --lib <new_test_name>
```

Run the specific tests the subagent claimed to add. Confirm them by name.

### 3c. Full-suite delta

```bash
cargo test -p <crate> --lib 2>&1 | grep "test result"
```

Compare `post_passed - baseline_passed` to the number of new tests requested.
If the new pass count matches, the subagent's tests are real. If the failure
count went up, investigate.

### 3d. Diff scope check

```bash
git diff --stat
git diff -- <expected_files>
```

The diff should match the bounded scope. If the subagent rewrote additional
files, classify the additions:

- **Pre-existing fix** (the rewrite was already in-flight migration): acceptable.
- **Creative license** (the subagent decided to "improve" something): revert
  and re-dispatch with a tighter scope.

### 3e. Spot-read the diff

`git diff -- <expected_files>` and read it. The subagent's logic for the new
feature should be minimal — if the diff is 4× larger than expected, it
probably contains side-quests.

### 3f. Orphaned-Module False Positive (build "passes" but new code never compiled)

A `cargo check` / `cargo build` that returns "Finished" is NOT proof the
subagent's new code was compiled. If the subagent created new module files
but **forgot to wire them in** at the integration points, the build succeeds
silently and the new code — and its tests — are simply excluded from the
compile tree.

This is the single most common "completed but broken" trap with Rust
multi-module work. The subagent reports "build passes" truthfully; it just
never compiled the files it wrote.

**Mandatory extra checks beyond `cargo check`:**

1. **Confirm the module is declared.** For every new `src/foo/mod.rs` (or
   `src/foo.rs`) file the subagent created, assert it is reachable from the
   crate root:
   - Library crate: `pub mod foo;` in `lib.rs`.
   - Binary crate (a `main.rs` that re-declares its own `mod` list — common in
     Rust projects that ship both `lib` and `bin` from one tree): **also**
     `mod foo;` in `main.rs`. A module declared only in `lib.rs` is invisible
     to the binary and produces `cannot find crate::foo` when the bin is built.
   - Axum (or other router) handlers: every route registered in the router
     (`/api/x` → `handler::run_x`) must have a corresponding `mod handler;`
     declaration and a function of that exact name. Missing `mod` →
     "file not found for module" or "cannot find function in this scope".

2. **List the compiled tests and grep for the new module.** If the subagent
   claimed to add tests in `foo::tests`, prove they are in the binary:
   ```bash
   cargo test --lib -- --list 2>&1 | grep -i "foo::"
   ```
   If the grep returns **nothing**, the module was never wired in — the
   "tests pass" claim was a false positive (the tests don't exist in the
   compiled artifact). This is how you catch an orphaned module when the
   build itself looks clean.

3. **Run the subagent's named tests by full path, not by a filter that may
   match zero.** `cargo test --lib foo::` returning "0 tests" is the tell. Run
   `cargo test --lib foo::tests::the_exact_name` and confirm it actually
   executes (you should see `running 1 test ... ok`). A filter that prints
   "0 passed; 0 failed" without listing any test name means it matched
   nothing — not that it passed.

4. **Fix wiring yourself when found.** Typical fixes: add `pub mod foo;` to
   `lib.rs` (+ `main.rs` for bin crates), add `mod foo;` near sibling module
   declarations in the router file, and point route handlers at the real
   module path (`crate::foo::api::run_x`, not `foo::run_x` when `foo` lives at
   crate root). Then re-run `cargo test --lib foo::` to confirm the tests now
   appear and pass.

**Why this matters:** a subagent that hits a model quota / rate limit mid-run
may report `status=completed` while having skipped its own build+test step
(e.g. the upstream returned 429 right before the verification command). The
"completed" flag is then a false positive too — verify with the filesystem +
compiled-test-list, never trust the summary text. (This is the same class of
"completed but partial" failure as Step 5's upstream errors — always inspect
the filesystem and run the build yourself.)

## Step 4 — Stale Test Fixtures

A common failure mode: the subagent's changes expose a test that was already
broken on main. The failing test was passing on `git stash` with the
subagent's edits removed, so it's a pre-existing fixture bug.

The subagent's job is to make the new code work, not to fix every stale test
in the repo. The requester must:

1. Identify which file the fixture is in.
2. Confirm the fixture was already stale on clean main.
3. Either fix it explicitly (one-line change is usually enough) or add it
   to the project backlog.

If the failing test was NOT broken on clean main, it's a regression caused
by the subagent's dispatch — that's a real bug, not a stale fixture.

## Step 5 — Subagent Returns with Upstream Error (Partial Work)

A subagent may report "upstream error — request could not be completed" or
similar but still have landed some files on disk. **Do not re-dispatch
blindly.** Before treating the error as a hard failure:

1. **Check the filesystem.** `ls -la` the files the subagent was supposed to
   create or modify. `grep` for the markers the subagent was supposed to
   add. The error may be a final-summary upload problem, not a work-completion
   problem.
2. **Run the build.** If the files compile, the subagent's work is probably
   real. Compile errors are a real failure; runtime errors during the agent
   itself are not.
3. **Run the targeted tests.** If the subagent claimed to add tests, run
   them in isolation. They may be a mix of pass and fail.
4. **Classify the failures.** Anything that fails is now a real bug to
   triage — even if the subagent blamed the error on infrastructure.
   Group failures by root cause:
   - **Hand-rolled primitive wrong** (e.g., Zeller congruence, custom
     Easter algorithm) — replace with the library's built-in:
     ```rust
     chrono::NaiveDate::from_ymd_opt(y, m, d)
         .map(|d| d.weekday().num_days_from_sunday())
         .unwrap_or(0)
     ```
   - **Test expected wrong value** — the code is correct, the test
     assertion is wrong. Fix the test (verify the expected value against
     an authoritative source: Python `datetime`, `dateutil`, etc.).
   - **Stale fixture surfaced** — see Step 4.
5. **Apply minimal patches**, re-run focused tests, then re-run the full
   suite delta before declaring done.

If the subagent landed zero files, then the upstream error is a hard
failure — re-dispatch with a clearer scope or escalate the requester.

**Stream-interrupted subagents (special case):** A subagent may report
`status=completed` with summary `[Stream interrupted. Please retry.]` —
this is NOT a completion, it's a network/transport failure during the
final summary upload. The subagent may have done zero work despite
running for minutes. Before re-dispatching:

1. **Check the filesystem immediately.** `ls` the target directory and
   `grep` for expected markers. The 440s runtime means nothing if no
   files were written — some providers bill for the full duration
   regardless.
2. **Check the live transcript.** `tail -10` the log file at
   `~/.hermes/cache/delegation/live/<delegation_id>/task-N.log`. If the
   last entry is `think` or `assistant` without corresponding `tool`
   results, the agent was mid-generation when interrupted.
3. **If zero files were written, do NOT re-dispatch the same prompt.**
   Re-dispatching burns another agent budget on a task that may
   fail the same way. Instead, assess: if you (the orchestrator) have
   all the context needed (you've read the source files, know the
   target structure, and the task is well-defined), do the work
   yourself. This is faster and more reliable than a third-party agent
   that might also get stream-interrupted. Reserve re-dispatch for
   cases where the task is too large or complex for the orchestrator
   to do directly.

**Key signal:** `final | status=completed ... summary: [Stream
interrupted. Please retry.]` in the transcript = transport failure,
not work completion. Always verify with filesystem inspection.

**Anti-pattern:** "The subagent had an error, I'll just re-dispatch the
whole prompt." This burns another agent budget on a task that may have
been 80% complete. Always inspect the filesystem first.

**Cross-check date math with an authoritative source.** Before trusting a
subagent's date arithmetic (Easter, nth-weekday, holidays, DST
transitions), verify against a known-good implementation:

```bash
# Python's datetime is the standard reference
python3 -c "import datetime; print(datetime.date(2026, 4, 5).strftime('%A'))"
# Should print: Sunday

# Python's easter algorithm
python3 -c "
def easter(y):
    a=y%19; b=y//100; c=y%100; d=b//4; e=b%4; f=(b+8)//25; g=(b-f+1)//3
    h=(19*a+b-d-g+15)%30; i=c//4; k=c%4; l=(32+2*e+2*i-h-k)%7; m=(a+11*h+22*l)//451
    t=h+l-7*m+114; return (y, t//31, t%31+1)
print(easter(2026))  # Should be (2026, 4, 5)
"
```

If the subagent's expected value disagrees with Python, the **test
assertion is wrong, not the code**.

## Step 6 — Ad-hoc Verification Script (Optional, High-Value)

For non-trivial changes, write a small `/tmp/<task>-verify.sh` script that
runs the per-step checks. The script makes the verification reproducible
and survives context-compaction in long sessions.

```bash
#!/bin/bash
# /tmp/option-a-intraday-approx.sh
set -uo pipefail
PASS=0; FAIL=0
ROOT=/home/ubuntu/projects/MarketMoves

report() {
  if [ "$1" = "ok" ]; then echo "  [OK]   $2"; PASS=$((PASS+1))
  else echo "  [FAIL] $2"; FAIL=$((FAIL+1)); fi
}

# 1. Source has the new fields
grep -n "pred_1h_approx: Option<f64>" "$ROOT/engine/src/api.rs" > /dev/null \
  && report ok "StatusResponse has pred_1h_approx" \
  || report fail "StatusResponse missing pred_1h_approx"

# 2. Math is correct
grep -n "pred_1h_approx: latest_pred.as_ref" "$ROOT/engine/src/api.rs" | grep -q "/ 6.5" \
  && report ok "handle_status: pred_1d/6.5" \
  || report fail "handle_status formula wrong"

# 3. Both new tests pass
TEST_OUT=$(cd "$ROOT" && cargo test -p engine --lib -- \
  prediction_dto_computes_approx_fields prediction_dto_handles_negative_pred 2>&1 | tail -10)
echo "$TEST_OUT" | grep -q "2 passed; 0 failed" \
  && report ok "Both approx tests pass" \
  || report fail "Approx tests failed"

# 4. Full suite shows expected delta
SUITE=$(cd "$ROOT" && cargo test -p engine --lib 2>&1 | grep "test result" | tail -1)
echo "$SUITE" | grep -q "96 passed; 12 failed" \
  && report ok "96 passed / 12 failed matches baseline" \
  || report fail "Suite counts changed"

# 5. Frontend DOM has the new ids
for id in p-1d p-5d p-21d p-1h-approx p-5h-approx; do
  grep -q "id=\"$id\"" "$ROOT/frontend/index.html" \
    && report ok "id=$id present" \
    || report fail "id=$id missing"
done

echo "========================================="
echo "Ad-hoc verification: $PASS passed, $FAIL failed"
echo "========================================="
exit $FAIL
```

Key properties of the script:

- Each `report` increments a counter so the final summary is one line.
- The script **exits with the failure count**, so it can be chained.
- Cleanup is implicit — the script lives in `/tmp` and can be deleted
  after use.
- grep checks source files for expected changes, then runs targeted tests
  once. Doesn't pay for a full suite per check.

## Anti-patterns

- **"Trust the subagent's verification"** — never. Re-run.
- **"I'll fix the test later"** — if the subagent's dispatch broke a
  pre-existing passing test, that test is now a regression. Fix it
  before declaring done.
- **"Let the subagent decide what to read"** — if the project has a
  knowledge graph or specific docs, cite them by path in the prompt.
- **"Delegate the whole task without checkpoints"** — for any task
  touching 3+ files or 2+ modules, set a mid-task checkpoint so the
  requester can catch drift before it compounds.
- **"Compare absolute pass counts without baseline"** — a 12 → 12 failure
  count is meaningless without knowing the 12 was already there.

## Model Tier Selection for Mechanical Coding Tasks

Mechanical code migrations (renaming tables, swapping one DB helper for another, adding DTO converters that mirror existing patterns) do not require a premium model. MiniMax M2.7 or equivalent mid-tier models complete these reliably when:

- The requester has already done the design work (identified all call sites, chosen the pattern, written the exact field mappings).
- The task is adding new functions with known signatures, or swapping one function call for another.
- There are no novel algorithms, no new abstractions to invent, and no architectural decisions to make.

When to use a mid-tier model for delegated coding:
- Adding a new DB query helper with a known sqlx pattern already visible in the file.
- Rewiring 3 API endpoints from `predictions` table → `equity_predictions` table (same DTO shape, no new logic).
- Adding a converter function that mirrors an existing one (e.g., `X_to_dto` → `equity_X_to_dto`).

When to escalate to a stronger model:
- The requester hasn't done the design work yet (the subagent needs to figure out the architecture).
- The change introduces a new abstraction, protocol, or data structure the codebase doesn't already have.
- The task touches error handling paths, concurrency, or trait bounds in a non-trivial way.
- The codebase has known footguns the subagent needs to navigate around (sqlx offline mode, specific Rust edition constraints, etc.).

Practical signal: if the requester's delegation prompt ends with "then run `cargo build` and fix any errors", the task is mechanical enough for a mid-tier model. If the prompt has to say "figure out the right approach", it's probably not.

## Step 7 — Parallel Delegation with Shared Contract

When two sub-agents need to build against each other's interfaces
(e.g. a backend WS module + a frontend dashboard that consumes WS
events), dispatch both simultaneously with each prompt containing the
**exact contract** the other is building.

```
Sub-agent A (backend) gets:
  "WebSocket endpoint /api/v1/ws pushes JSON with `type` field.
   Events: PnlTick{realized_pnl, unrealized_pnl, position, ...},
           PredictionUpdate{pred_1d, pred_5d, pred_21d, ...}, ..."

Sub-agent B (frontend) gets:
  "The backend will expose WS /api/v1/ws with events:
   PnlTick{realized_pnl, unrealized_pnl, position, ...},
   PredictionUpdate{pred_1d, pred_5d, pred_21d, ...}, ..."
```

Rules:
- **The contract is the coupling.** If both agents have the exact same
  field names, types, and JSON shapes in their prompts, they can build
  independently and integrate on first try.
- **The orchestrator writes the contract.** Don't let each agent invent
  its own event names. The orchestrator must decide on the exact enum
  variant names, field names, and JSON serialization shape, then paste
  that into both prompts verbatim.
- **Verify both sides match after dispatch.** After both sub-agents
  complete, grep the backend output for the event names and grep the
  frontend for the same names. If they disagree, the contract was
  violated — patch the side that drifted.
- **Works for any split.** Backend+frontend, DB schema+migration code,
  API+CLI client, protocol+parser — any case where two independent
  workstreams share a contract the orchestrator can define upfront.

This pattern eliminates the serialization cost of doing them
sequentially. The orchestrator's only overhead is writing a complete
contract before dispatch — which takes 1-2 minutes and saves 10+ minutes
of sequential waiting.

## Relationship to Other Skills

- **`requesting-code-review`** — covers the requester's own verification
  pipeline. This skill covers the case where the requester delegated the
  implementation to a subagent and must verify the subagent's output.
  Complementary.
- **`claude-code` / `codex` / `opencode`** — CLI-specific orchestration
  guides. This skill is CLI-agnostic and applies to `delegate_task`
  (Hermes internal) too.
- **`plan`** — write-only planning. This skill is for implementation +
  verify.
- **`test-driven-development`** — TDD discipline. This skill is the
  verification wrapper around it when implementation is delegated.