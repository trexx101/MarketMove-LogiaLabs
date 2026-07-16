CRITICAL RULES - MUST FOLLOW
RESPONSES
Keep responses concise and to the point - unless the user asks otherwise
PLANNING MODE
Always ask clarifying questions
Never assume design, tech stack or features
Use deep-dive sub-agents to assist with research
Use deep-dive sub-agents to review the different aspects of your plan before presenting to the user
CHANGE / EDIT MODE
Never implement features yourself when possible - use sub-agents!
Identify changes from the plan that can be implemented in parallel, and use sub-agents to implement the features efficiently
When using sub-agents to implement features, act as a coordinator only
Use the best model for the task - premium models for complex tasks (like coding) and mid-tier models for simpler tasks, like documentation
After completing features (large or small), always run commands like lint, type check and next build to check code quality
DATABASE SCHEMA CHANGES
Whenever you make changes to the database schema, ALWAYS run the drizzle generate and migrate commands
NEVER run drizzle push!
For all ID columns NOT related to BetterAuth, use UUID for the ID columns and be randomly generated
TESTING
Use any testing tools, libraries available to the project for testing your changes
Never assume your changes simply work, always test!
If the project does not have any testing tools, scripts, MCP tools, skills, etc. available for testing, ask the user whether testing should be skipped.
UI DESIGN
Always follow the UI design system when creating or reviewing components or pages.
Design System: @DESIGN.md

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
