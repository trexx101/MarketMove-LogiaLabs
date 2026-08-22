# Hermes → DSH Skill Migration

Date: 2026-08
Source: `/home/ubuntu/.hermes/skills` (Hermes profile skills)
Destination: `.agents/skills/<name>/` (auto-discovered by DeepSeek Harness)

## Why this works without ports

DSH scans `.<project>/.agents/skills/<name>/SKILL.md` (one level deep) natively,
so transplanted skills become available live. No conversion needed — Hermes
`SKILL.md` frontmatter beyond `name`/`description` is simply ignored.

## Migrated skills (33)

### MarketMoves-custom (the gold)
- `marketmoves-project` — project context, deploy gate, phases A–E
- `marketmoves-dev` — engine backend traps, Phase-7 options architecture, DB patterns
- `marketmoves-ops` — operating the live trading stack
- `marketmoves-strategy-findings` — backtest findings (dominant threshold config)
- `marketmoves-equities-execution` — shorting / PSQ remap / TOTP live toggle
- `quant-trading-model-validation` — predictive-edge gate (IC, labels, traps)
- `rust-quantitative-dev` — Rust quant dev (options, backtesting, stats)
- `ml-train-serve-parity` — 5-axis train/serve skew detection + parity harness

### Stack / execution / quality
- `rust-axum-backend`, `svelte-frontend-development`,
  `svelte-vite-serve-from-built-dist`, `docker-compose-deployment`,
  `rust-docker-review-redeploy`, `github-pr-workflow`,
  `code-change-verification`, `requesting-code-review`,
  `systematic-debugging`, `test-driven-development`, `python-debugpy`,
  `safe-structured-edits`, `subagent-delegation-hygiene`,
  `model-routing-proxy`, `moomoo-opend-ops`

### Planning / research / problem-solving
- `plan`, `spike`, `external-planning-prompt`, `resume-half-finished-project`,
  `polymarket`, `jupyter-live-kernel`,
  `planning-and-task-breakdown`, `incremental-implementation`,
  `debugging-and-error-recovery`, `code-review-and-quality`

## Intentionally NOT migrated

- **`omh/*` orchestration suite** (ralph / ralph-driver / triage / ralplan) —
  DSH has native `goal`, `subagent`, `workflow`, and `ralph` tools that replace
  this; kept only as reference, not re-litigable.
- Irrelevant categories: `apple/*`, `creative/*` (video/design), `smart-home/*`,
  `social-media/*`, `email/*`, `note-taking/*`, `media/*`, `caveman`, etc.
- `software-development/spec-driven-development` — source file does not exist
  in the Hermes profile.
- `autonomous-ai-agents/hermes-agent` — meta skill about running Hermes itself.

## Pre-existing skills left untouched

`find-skills`, `install-moomoo-opend`, `moomooapi` (project-owned), plus the
DSH-installed `plan-to-features`, `rust-best-practices`, `technical-analysis`
(and a non-skill `prompt/` folder that DSH ignores).

## Resolved during migration

**`moomooapi` discovery fix.** It was being dropped because its frontmatter
`description:` was a single 1.6 KB *unquoted* plain scalar containing
`mentions: quote` / `search: …` — the `: ` (colon-space) inside a plain scalar
is invalid YAML, so DSH's parser threw and silently ignored the file. Fixed by
wrapping the description in double quotes (byte-preserving; CRLF kept; validated
with the same `yaml` parser DSH uses). It now loads.

## Outstanding items

1. Re-run a fresh DSH session to confirm all 33 migrated skills + `moomooapi`
   resolve and load in the catalog.