# Multi-Phase Plan Directory Template

For large-scale efforts (platform redesigns, major refactors, system migrations) spanning
multiple weeks with 3+ independently-deployable phases. Use this instead of a single-file
plan when the work doesn't fit in one implementation session.

## Structure

```
.hermes/plans/<slug>/
  MASTER.md           — overview: phase sequence, current→target, locked contracts, decisions
  PHASE_0_<name>.md   — one file per phase
  PHASE_1_<name>.md
  PHASE_N_<name>.md
```

## MASTER.md skeleton

```markdown
# <Project Name> — Master Plan

**Source**: <link or path to design doc / requirements>
**Scope**: <one-line scope statement>

---

## Current State → Target State

| Layer | Current | Target |
|-------|---------|--------|
| ...   | ...     | ...    |

## Phase Sequence

**Phase 0 — <name>** (<effort>)
<what it does, why it's first>

**Phase 1 — <name>** (<effort>)
<what it does, dependency on Phase 0>

...

## Locked Contracts (must not break across all phases)

- <DB table schemas that are immutable>
- <API contracts that are additive-only>
- <feature pipelines / normalization specs>
- <deploy gates / validation rules>

## Key Design Decisions

| OD  | Topic  | Decision |
|-----|--------|----------|
| OD1 | ...    | ...      |

## Files in This Plan

- `MASTER.md` — this file
- `PHASE_0_<name>.md` — ...
- `PHASE_1_<name>.md` — ...
```

## Phase file skeleton

```markdown
# Phase N — <Phase Name>

**Goal**: <what this phase delivers>
**Estimated effort**: <time>
**Can deploy independently**: Yes/No
**Depends on**: <prior phases>

---

## N.1 <Component name>

### Current state
<what exists now>

### Target state
<what this phase produces>

### Steps
1. <step>
2. <step>

### Key constraint
- <what must not break>

### Files to create/modify
- **CREATE** `path/to/new_file.rs`
- **MODIFY** `path/to/existing.rs` — <what changes>

---

## N.X Test requirements
### Backend
- <unit test description>
- <integration test description>

### Frontend (if applicable)
- <manual test description>

---

## N.X Rollout steps
1. <deploy step>
2. <verify step>

---

## N.X Risk notes
- **<risk>**: <mitigation>
```

## When to use directory vs single file

- **Single file** (`.hermes/plans/YYYY-MM-DD_HHMMSS-<slug>.md`): feature-level work, 1-3 days,
  fits in one session. Bite-sized TDD tasks (2-5 min each). This is the DEFAULT.
- **Directory**: multi-week, 3+ phases, each independently deployable, phases implemented
  weeks apart. Each phase file owns its own contracts, tests, rollout, risks.

## Process for deriving phases from a design document

1. Read the design document end-to-end.
2. Map the current codebase (read the actual files — verify types, schemas, API shapes).
3. Build the current→target gap table.
4. Identify locked contracts (things that must not break).
5. Sequence phases by dependency: foundation → UI → features → integrations.
6. Write MASTER.md first, then each phase file.

Read-only codebase exploration IS expected during planning — the restriction is on *writing*
project files, not on *reading* them to ground the plan.
