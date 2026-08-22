# PROJECT RULES — MarketMoves

## TECH STACK & DATABASE STRICT RULES
- **Schema Changes:** Whenever database schema changes occur, ALWAYS run `drizzle generate` and `drizzle migrate`. NEVER run `drizzle push`.
- **Primary Keys:** For all ID columns NOT related to BetterAuth, use randomly generated UUIDs.
- **UI Design:** Always follow the UI design system in `DESIGN.md`. Use it when creating or reviewing components.

## TESTING PROTOCOL
1. Never assume code works; test all changes.
2. Use available testing tools, libraries, scripts, or MCP tools.
3. If no testing infrastructure exists, ask the user whether testing should be skipped.

## GRAPHIFY WORKFLOW
This project uses a static knowledge graph at `graphify-out/` (`graph.json` present; `graphify` CLI is installed at `/home/ubuntu/.local/bin/graphify`). Use it as the primary lookup when you need to locate or understand functions, classes, and modules — before scanning source files directly.

- **Targeted Queries:** Before browsing source code, run `graphify query "<question>"` (if `graphify-out/graph.json` exists). Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts.
- **Navigation:** Use `graphify-out/wiki/index.md` for broad navigation over raw source browsing.
- **Cost Efficiency:** Prefer `query` / `path` / `explain` over raw scans — they return a highly scoped subgraph and save significant token overhead. Read `graphify-out/GRAPH_REPORT.md` only for broad architecture review.
- **Maintenance:** Run `graphify update .` after code modifications to keep the AST graph current (no API cost).
- **Graph State:** Dirty `graphify-out/` files are expected after updates. Only skip graphify if the graph is fundamentally broken or the user explicitly commands it.