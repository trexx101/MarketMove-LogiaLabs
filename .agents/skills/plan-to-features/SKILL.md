---
name: plan-to-features
description: Convert an approved plan into per-feature implementation specs under a dedicated subfolder in plans/. Use when the user has a plan (from plan mode, a prior conversation, or a pasted document) and asks to "turn the plan into features", "break this into features", "create an implementation plan", or "split the plan up". Produces a REQUIREMENTS.md index plus one file per feature, each with requirements, technical steps, and acceptance criteria.
---

# Plan to Features

Turn an approved plan into a set of separately-trackable feature specs.

## Output structure

Create a **dedicated subfolder under `plans/` named after the initiative** (kebab-case),
so multiple plans never collide. Put everything inside it:

```
plans/
└── [initiative-name]/          # kebab-case, derived from the plan (e.g. todo-kanban-app)
    ├── REQUIREMENTS.md
    └── features/
        ├── 01 - [feature name].md
        ├── 02 - [feature name].md
        └── ...
```

## Steps

1. **Pick the initiative name.** Derive a short kebab-case slug from the plan
   (e.g. `todo-kanban-app`). If it's ambiguous, ask the user once; otherwise pick the
   obvious name and state it.
2. **Write `REQUIREMENTS.md`** as the index. Include: a one-paragraph overview, confirmed
   product decisions, tech stack, global constraints/rules, the data model (if any),
   environment variables (if any), and a **feature table** listing each feature with its
   number, name, and dependencies.
3. **Split the work into features.** Each feature is one coherent, independently-verifiable
   unit. Order them by dependency (foundation → data → core features → polish). Note which
   features can be built in parallel.
4. **Write one file per feature** at `features/[NN] - [name].md` using the template below.
   Use zero-padded two-digit numbers (`01`, `02`, ...).
5. **Keep it scannable** — detailed enough to execute, concise enough to skim. Reference
   concrete file paths and reuse existing utilities where known.

## Feature file template

```markdown
# Feature [NN] — [Name]

**Depends on:** [feature numbers, or "none"]
**Goal:** [one sentence]

## Requirements

- [what this feature must do — bullet points]

## Technical Implementation Steps

1. [concrete steps: files to create/edit, approach, libraries]

## Acceptance Criteria

- [ ] [verifiable, checkable outcomes — include quality gates like lint/typecheck/build]
```

## Notes

- Do **not** implement anything — this skill only produces the planning documents.
- One subfolder per initiative keeps each plan self-contained; never write `REQUIREMENTS.md`
  or `features/` directly at the `plans/` root.
