---
name: resume-half-finished-project
description: Reconstruct the true status of a half-finished, multi-stage project or workstream and report what's done vs. what's next. Use when the user says "we stopped halfway", "where did we leave off", "what's the status", or returns to a project after a gap and asks for a progress accounting.
---

When a user returns to a long-running, multi-stage project (waves / phases / milestones) and asks what's done or what's next, do NOT guess from memory or from a single file. Reconstruct ground truth from artifacts, then report. The goal is an honest done-vs-gaps accounting, not a cheerleading summary.

## Triggers

- "we stopped halfway", "where did we leave off", "what was done and what's next"
- Returning to a project after days/weeks with no prior summary in context
- Any request to audit progress against a staged plan

## Workflow (reconstruct, don't assume)

1. **Git is the spine.** `git status` (modified / untracked files = work started but not committed) and `git log --oneline -15` (direction of recent work). Untracked files are unfinished, not finished.
2. **Find the plan / intent doc.** The latest relevant commit often references a plan file or carries the milestone list in its body. Read it: `git show <sha>` or `git log -1 --format='%B' <sha>`, then read the referenced plan doc in full. The plan defines the per-stage deliverable — your audit yardstick.
3. **Map built vs. wired.** Reading an implementation file proves it exists, not that it runs. For each planned stage, check:
   - Are the new modules/files present and do they compile?
   - Are they actually *called* from the entry point (main.rs / scheduler / config / route registration)? A feature that exists but is never invoked is a GAP, not a completion.
   - Is there a second, orphaned source of truth (e.g. a standalone migration `.sql` file that the code never applies, while tables are actually created inline in a DDL string)? Flag which one is live.
4. **Verify, don't trust.** Run the project's own build + tests (`cargo check -p <crate>`, `cargo test --lib <filter>`). Code that compiles and has passing unit tests is still only "built", not "runtime-integrated".
5. **Probe live dependencies.** If the work depends on external endpoints (data APIs, auth), do a quick reachability check (e.g. `curl -s -m 20 <endpoint> | head -c 400`). Reachability changes — a prior session may have hit a geo-block/rate-limit that is now resolved, or vice versa.
6. **Report structure.** Lead with DONE, then NOT DONE (the specific gaps that block the deliverable), then the deliverable definition (quote the plan), then NEXT. Be blunt: "built and correct but not runtime-integrated" is a precise and useful verdict — use it.

## Pitfalls (the traps this workflow exists to catch)

- **Implemented ≠ integrated.** A client module with passing unit tests that is never called from startup is a gap. Always trace call sites from the entry point, not just the file's existence.
- **Dual sources of truth.** Schema/migration defined in both a `.sql` file and inline DDL — only the code path that actually executes is real. Identify and name which one runs.
- **Retired paths still launching.** The entry point may still invoke a deprecated data source (e.g. a retired exchange client) that masks or breaks the new path. Check what `main()` actually does, not what it should do.
- **Test-pass ≠ deliverable-met.** Unit tests green only proves the module's internal logic; it does not prove the endpoint returns data without a manual trigger. Tie verification back to the plan's stated deliverable.
- **sqlx SQLite file not auto-created.** sqlx 0.8 does not auto-create the `.db` file on connect — only the directory. A fresh DB path fails with "unable to open database file (code 14)" even when the directory exists. If the engine exits at startup with this error, pre-create the file (`touch path.db` or `OpenOptions::new().create(true)`) before connecting. This is a non-obvious failure that looks like a permissions or path issue but isn't.
- **cargo run CWD mismatch.** `cargo run -p <crate>` executes from the crate dir, not the workspace root. Relative `.env` paths (e.g. `NORM_STATS_PATH=models/...`) resolve against the wrong CWD, causing the binary to fail at startup with a misleading "file not found" error. When verifying locally, pass absolute paths via env vars, or run the compiled binary directly from the workspace root.
- **Don't save task progress to memory or as project-state in a skill** — it goes stale within a week. Capture the durable *technique* (this skill) and only the durable *codebase facts* that persist regardless of ongoing work (e.g. "schema is created inline, not from migrations/").

## Verification (when the user wants to close the gap)

Wire the missing call sites, add the missing config entries, then re-run build + the targeted tests + (if applicable) a live endpoint reachability check. Only then is the stage's deliverable actually met.

## References

- `references/audit-checklist.md` — copy-paste runbook for the reconstruct-and-report pass.
