# Prompt: Convert a Plan into Per-Feature Implementation Specs

Use this after you have an approved plan (e.g. from plan mode) and want it broken
down into separately-tracked feature files, namespaced under its own subfolder so
multiple initiatives never collide.

---

Convert the plan into a detailed implementation plan, split up into "features". Create a **dedicated subfolder under `plans/` named after this initiative** (kebab-case, e.g. `plans/todo-kanban-app/`) and put everything inside it. Each feature should have detailed requirements, technical implementation steps, and acceptance criteria, and be stored as a separate file.

The structure should be:

- plans
-- /[initiative-name]
--- REQUIREMENTS.md
--- /features
---- [feature number] - [feature name].md

---

## Why this works

- **Dedicated subfolder** (`plans/[initiative-name]/`) keeps each project's plan
  self-contained so future plans don't collide or overwrite each other.
- **Naming guidance** (kebab-case, derived from the initiative) removes guesswork.
- **REQUIREMENTS.md as an index** plus one file per feature keeps each unit of work
  separately trackable, with requirements, technical steps, and acceptance criteria.
