# Plan Gap Analysis — Resuming a Partially-Implemented Plan

When the user asks to "review changes against the plan" or "complete the
plan", the plan file and the commit history are the two sources of truth.
Reconcile them before doing anything else.

## Procedure

### 1. Locate the plan

Plan files live in:
- `.omo/plans/<name>.md` (OpenCode/Boulder workflow)
- `.hermes/plans/<name>.md` (Hermes plan mode)
- `.hermes/plans/YYYY-MM-DD_HHMMSS-<slug>.md` (timestamped)

### 2. Map commits to todos

For each todo in the plan, search the commit history and the current
codebase for evidence of completion:

```bash
git log --oneline | grep -iE "<keyword from todo description>"
grep -n "<symbol or function name>" <expected file path>
```

### 3. Build a status table

Track each todo's status:

| Todo | Wave | Commit | Status |
|------|------|--------|--------|
| 1. Scheduler fix | W1 | a4829ca | done |
| 7. Actuals background task | W3 | — | MISSING |
| 6. DB actuals functions | W2 | 3875b48 | PARTIAL (dead code) |

- **done** — committed, tests pass, invoked from runtime path
- **PARTIAL** — function exists with tests but is never called (dead code)
- **MISSING** — no commit, no code

### 4. Identify the completion boundary

Waves 1-2 may be done while Waves 3-4 are entirely missing. The boundary
between done and missing is where work resumes.

### 5. Check dependencies for the missing todos

Before dispatching, verify the missing todos' dependencies are satisfied.
E.g. if Wave 3 needs to wire a DB function from Wave 2, confirm that
function exists and is tested — the subagent only needs to wire it, not
reimplement it.

## Pitfall: Dead Code vs Done

A common false positive: a function exists in the codebase with tests
passing, so it looks "done" in the gap analysis. But it's never called
from any runtime path.

Example from MarketMoves wave2:
- `compute_actuals()` — defined in `db.rs`, has passing tests, but never
  spawned as a background task from `main.rs`. Dead code.
- `fetch_accuracy()` — defined in `db.rs`, has passing tests, but no
  `/api/accuracy` endpoint calls it. Dead code.
- `AccuracyStats` struct — defined, tested via `fetch_accuracy`, but
  never serialized by any API handler. Dead code.

Detection:
```bash
# If the only hits are in the definition file and tests, it's dead code.
grep -n "compute_actuals" engine/src/main.rs engine/src/scheduler.rs engine/src/api.rs
# No output → never invoked → not wired → PARTIAL, not done.
```

The cargo compiler confirms this: `warning: function 'compute_actuals'
is never used` — these warnings are the compiler telling you the gap
analysis should mark the todo as PARTIAL.

## Dispatching the Remaining Work

Once you know what's missing, dispatch subagents **per-wave** (not
per-todo) — a wave is the natural parallelization unit defined in the
plan. Give each subagent:

- The exact todo text from the plan (they have no interview context)
- The file paths and line numbers from the plan's References field
- The acceptance criteria and QA scenarios from the plan
- The commands to verify (cargo build, cargo test, node --check, etc.)
- The API contracts that parallel waves depend on (e.g. frontend wave
  needs to know the JSON shape the backend wave will produce)

Run waves in parallel when their dependency matrix allows. Serialize when
a later wave depends on an earlier wave's output (e.g. frontend wiring
that calls backend endpoints should wait for the backend wave to finish
building, though the subagent can work against the documented API
contract).

## Integration Step

After all subagents return:

1. `cargo build --release` — confirm the full project compiles with
   all waves' changes merged.
2. `cargo test --release --lib` — run the full test suite (not just
   per-module) to catch integration issues.
3. Rebuild the Docker image(s) — the subagents changed source, so the
   running image is now stale.
4. Redeploy and verify (follow the main rust-docker-review-redeploy
   procedure, steps 3-5).
