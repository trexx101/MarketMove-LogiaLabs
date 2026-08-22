# Resume / audit a half-finished project — copy-paste runbook

Use for any "we stopped halfway / what's the status" request on a multi-stage
(waves / phases / milestones) project. Goal: honest done-vs-gaps accounting.

## Step 1 — Git spine
```bash
git status                      # modified = in progress, untracked = unfinished
git log --oneline -15           # direction of recent work
git show <sha> --stat           # what a milestone commit touched
git log -1 --format='%B' <sha>  # milestone/deliverable body
```

## Step 2 — Find & read the plan doc
- The latest commit often references a plan file (tracked) or embeds the milestone list in its body.
- Read the referenced plan doc IN FULL — it defines the per-stage deliverable, your yardstick.

## Step 3 — Map built vs. wired (the core)
For each planned stage file:
```bash
# untracked / modified source files
git status --short
# every module present
search_files(target='files', pattern='*.rs')   # or *.py / *.ts
# is the new code actually CALLED?
search_files(pattern='mod <newname>|newname::|handle_new|Spawning new')
```
Check the ENTRY POINT (main.rs / scheduler / config / api router) for real call sites.
A module with passing tests that is never invoked from startup = GAP.

## Step 4 — Verify, don't trust
```bash
cargo check -p <crate>          # or: pytest / npm run build / tsc
cargo test -p engine --lib <filter>   # targeted unit tests
```

## Step 5 — Probe live deps
```bash
curl -s -m 20 -A "<ua>" "<endpoint>" | head -c 400
```
Reachability changes between sessions — re-check, don't assume a prior block persists.

## Step 6 — Report shape
1. DONE (file:line evidence)
2. NOT DONE — gaps blocking the deliverable (quote the plan's deliverable)
3. Deliverable definition (from plan)
4. NEXT concrete step

Useful precise verdict: "built and correct but not runtime-integrated."

## Pitfall flags to raise explicitly
- Implemented ≠ integrated (module exists + tests pass, but never called).
- Dual sources of truth (`.sql` migration file vs inline DDL — only the executing path is real).
- Retired path still launched by entry point (entry point invokes deprecated source).
- Test-pass ≠ deliverable-met (endpoint still needs a manual trigger).
- sqlx SQLite file not auto-created (code 14 on fresh DB — pre-create the `.db` file).
- cargo run CWD mismatch (relative `.env` paths resolve from crate dir, not workspace root).
- Do NOT save task progress to memory/skill — it goes stale. Save the durable technique only.
