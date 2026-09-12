Execute skills for specialized capabilities and domain knowledge. When users reference a slash command (e.g., '/commit', '/review-pr'), use this tool to invoke it.

When a skill matches the user's request, invoke the Skill tool BEFORE generating any other response about the task. Only use for skills listed in user-invocable skills — do not guess or use built-in CLI commands.

## hexa skills available

These skills are defined in `hexa-cli/assets/skills/` and deployed to `.claude/skills/` in target projects:

| Skill | Trigger | Purpose |
|---|---|---|
| `/hexa-feature-dev` | Starting a new feature | Full lifecycle: specs → workplan → worktrees → code → validate |
| `/hexa-scaffold` | Creating a new hexa project | Scaffold hexagonal project structure |
| `/hexa-generate` | Generating code | Code generation within an adapter boundary |
| `/hexa-summarize` | Token-efficient summaries | AST summaries at L0-L3 depth |
| `/hexa-analyze-deps` | Dependency analysis | Tech stack recommendations |
| `/hexa-analyze-arch` | Architecture health | Boundary violation check |
| `/hexa-validate` | Post-build validation | Semantic validation gate |
| `/hexa-workplan` | Workplan management | Create, validate, activate workplans |
| `/hexa-ADR-create` | Creating ADRs | New Architecture Decision Records |

## Rules for hexa agents

- **Always invoke via Skill tool** — do not describe what a skill does and then skip invoking it
- **Skills run BEFORE other responses** — if a user's message maps to a skill, invoke first, then elaborate
- Skills are defined in `.claude/skills/*.md` — do NOT guess at skill names not listed there
- Built-in CLI commands like `/help`, `/clear` are NOT skills — do not use the Skill tool for them
