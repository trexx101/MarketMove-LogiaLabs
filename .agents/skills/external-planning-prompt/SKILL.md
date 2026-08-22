---
name: external-planning-prompt
description: Build a self-contained, fact-grounded planning/implementation prompt to hand off to a stronger or external model (Gemini, Opus, a Colab notebook) when the user will run it themselves. Covers grounding the prompt in the real repo, locking hard constraints, flagging open decisions with recommended defaults, and guaranteeing training↔inference↔serving alignment so the external model's output is directly implementable.
---

# External Planning Prompt (handoff to another model)

## When to use
The user wants a *document* to feed into another model/host they control — e.g. "create a prompt
for Wave C to run on Gemini 3.1 Pro", "flesh out the plan, I'll run it externally", "write a prompt
and I'll paste it into Colab". You are the RESEARCH + GROUNDING agent; the external model is the
PLANNER/IMPLEMENTER. Your deliverable is the prompt file, not the code.

This is a recurring pattern for this user: agent grounds the task in the repo → user runs the prompt
on an external/stronger model (Gemini via OmniRoute, Colab, etc.) → reports the output back → agent
implements and tests. Build the prompt so the loop closes cleanly.

## Workflow (do all of these)
1. **Ground in the repo first.** Read the actual files that define the contract you're planning
   around: feature/transform code, the model backbone you're porting, the label/objective function,
   the DB schema, the serving/DTO, and config. Do NOT trust memory for technical values (dims, gate
   thresholds, horizons, normalization method). Quote `file:line` for every locked fact.
2. **Lock the hard constraints explicitly** in the prompt: exact feature order + dimension,
   normalization method (and that it must be identical at train AND inference), DB column names,
   serving DTO field names, deploy-gate formula + threshold. Mark these HARD / DO-NOT-BREAK.
3. **Surface design tensions/mismatches.** If the existing code has a contract the new work violates
   (e.g. crypto 1h/4h/24h horizons vs new 1d/5d/21d), name them and state how the plan resolves them
   *without a mid-flight break* (retire old path dormant, bump schema_version, additive dims).
4. **Bake recommended defaults for open decisions, but flag them.** Where you lack a definitive
   answer, pick a sensible default, mark it `OPEN DECISION Dn` with rationale, and let the external
   model confirm/refine. Never leave a blank the external model must guess on silently.
5. **Make it self-contained.** Inline all the facts from steps 1–3 so the external model stays
   aligned without the user re-explaining. No "see the repo" — paste the contract.
6. **Specify an OUTPUT FORMAT** (numbered sections) so the returned plan is directly actionable and
   you can implement it.
7. **Close the loop:** after the user runs it, take the external model's output and implement/test it
   in the repo.

## Pitfalls (this user has corrected these)
- **Honor committed code over your "recommended" value.** If memory/your-recommendation disagrees
  with a value in the repo, the repo is right. Example: the training gate was `ic_gate = 0.03` in
  `train_tcn.py`; a stale note said 0.05 and the agent "recommended 0.05" — the user overrode to
  0.03 (the code value). When you spot a memory-vs-code discrepancy, trust the code and flag it;
  don't silently pick a different "plan target".
- **Fold in scope the user asks to fold in.** If the user says "yes, fold it in", own the dependent
  rewrite in the *same* wave rather than deferring it to a later wave. Example: the daily
  strategy/bridge rewrite (`next_position`, `predict_v3`) was being pushed to Wave D; the user wanted
  it inside Wave C. Don't artificially defer dependent contract work.
- **Every proposed improvement must carry a design-safety mechanism.** When asked for "improvements
  that increase accuracy but must not break the design", require each candidate to state *how* it
  preserves the locked contract (feature order, norm-stats, serving DTO). Reject any that mutate base
  indices or the normalization spec.
- **Don't invent APIs/data.** Ground every fact in a file you read. A self-contained prompt is built
  from real reads, not assumptions.
- **Distinguish hard contracts from soft design choices — don't over-constrain.** When planning a
  redesign/extension, only lock what is truly contracted: DB schema, feature pipeline + ordering,
  normalization method, inference protocol, deploy gate, serving DTO field names. Do NOT lock
  current implementation details that are merely "how it's configured today": strategy params
  (e.g. long/flat-only is a config default, not a contract — the Position enum may already support
  Short), UI layout ("preserve all 4 panels" when the user wants a full redesign), or feature scope
  (the base-8 is locked, but additive extensions / schema_version bumps are an open decision).
  Over-locking forces the external model to propose workarounds for constraints that don't exist
  and frustrates the user who explicitly wanted those options open. When in doubt, mark it
  `OPEN DECISION Dn` with a recommended default rather than `HARD / DO-NOT-BREAK`.
- **When the user asks for API/third-party recommendations, include them as a dedicated section.**
  A platform-redesign prompt is incomplete without surveying the external services the platform
  will need (real-time data, brokerage/exec, news/sentiment, alt data). Recommend specific
  services with cost, free-tier limits, Rust integration approach, and priority (MVP vs later).
  Don't make the user ask twice.
- **Separate "locked facts" from "replaceable/reference" sections.** Mark sections that are
  reference-only (e.g. current frontend structure, current strategy defaults) as `replaceable` or
  `for reference — rewrite freely` so the external model knows it can redesign there. Reserve
  `locked` for true contracts. This prevents the external model from treating a description of
  the current state as a constraint to preserve.

## Also covers: multi-phase plan from a design document
When the user says "digest this design document and create a multi-step plan" or "produce
detailed plan files for each stage," the same grounding workflow applies — but instead of a
prompt for an external model, the deliverable is a **plan directory** with a MASTER.md and
per-phase files. Use `templates/multi_phase_plan_directory.md` for the structure. Each phase
file gets: goal, dependencies, exact files to create/modify, inline contracts/DDL/types, test
requirements, rollout steps, and risk notes. Read-only codebase exploration IS expected
during planning.

## Support files
- `templates/handoff_prompt_skeleton.md` — copy this skeleton and fill the locked sections with repo
  facts; this is the fastest way to produce a consistent, self-contained handoff prompt.
- `templates/multi_phase_plan_directory.md` — when the user asks you to turn a design document into
  a multi-phase implementation plan (not an external-model prompt), use this directory structure
  (MASTER.md + per-phase files) instead of a single-file plan. Covers phase sequencing, locked
  contracts, per-phase test/rollout/risk sections.
- `references/marketmoves_contracts.md` — condensed locked contracts for the MarketMoves QQQ-equities
  project (wave structure, 8-dim feature contract, deploy gate, serving DTO, crypto↔equities mismatch)
  for quick re-grounding in that specific repo.
