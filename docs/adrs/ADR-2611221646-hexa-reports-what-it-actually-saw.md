# ADR-2611221646: hexa reports what it actually saw

**Status:** Completed
> 2026-11-22 Completed: landed on main at 9e913fb; gate `cargo test -p hexa-cli --test adr_2611221646_spec_2 --test adr_2611221646_spec_3` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec, chose the fix under his rule (real bug fixes only; architectural correctness first), and its sealed tests, red before the code, pass on main
**Date:** 2026-11-22
**Drivers:** Five places where hexa printed something untrue or refused something harmless, found while using it on 2026-11-21 and 2026-11-22:
- `hexa spec tests` failed with "claude-text exited with 1: " and nothing after the colon; the reason (an expired AWS SSO token) was in the CLI's output and hexa dropped it.
- `hexa bench agentic` ran five fixtures for three models over two hours with `bun` not installed; every fixture's evidence is `bun test …`, so every edit failed with `bun: command not found`, and each model was reported as passing 0%.
- `hexa analyze` on a Python SDK printed "Use Cases (97 files)" when 70 files were analysed in all: the "Hex layers" tally counts files the grade excludes.
- In a Python repository the session context hexa injects said "Language: unknown", while `hexa analyze` said "language: python (pyproject.toml)".
- The pre-agent hook blocked a code-reviewer subagent ("it would edit on this branch") whose definition grants it only Read, Grep and Glob.
CLAUDE.md: "A gate that degrades silently is worse than no gate."
**Supersedes:** none. Defects in what hexa reports; no gate becomes looser.

## Context

- `claude-text` runs `claude -p --output-format json` (`hexa-infer/src/adapters/secondary/claude_code.rs`). On failure the CLI exits non-zero and writes its reason into the JSON on stdout (`result`, `terminal_reason`); stderr is empty. hexa reports stderr only.
- `hexa bench agentic` (`hexa-cli/src/commands/bench.rs`) runs each fixture's oracle command as its evidence, and never checks that the command's program exists.
- The "Hex layers" tally (`hexa-cli/src/commands/analyze.rs`) walks the source files under the scan directory and classifies each, without the exclusions the grade applies (`analyze.exclude` and the built-in patterns).
- The session context (`hexa-cli/src/commands/hook/mod.rs`, `minimal_fingerprint_block_inner`) detects the language from `Cargo.toml`, `go.mod` or `package.json` and knows no Python manifest.
- The pre-agent hook (`hexa-cli/src/commands/hook/mod.rs`, `pre_agent`) treats a subagent as read-only only when its type is in a fixed list; any other type without `isolation: "worktree"` is blocked, whatever tools its definition grants.

## Decision

1. **A failed `claude-text` call says why.** When the CLI exits non-zero, the error names the reason the CLI wrote to stdout (its `result`, else its `terminal_reason`) along with stderr, so "exited with 1:" is never followed by nothing when the CLI said something.
2. **`hexa bench agentic` checks its evidence can run before it runs anything.** Before the first fixture, the program each selected fixture's evidence command starts with is looked up on `PATH`; when one is missing, the bench refuses with the program and the fixtures that need it, and runs no fixture.
3. **The "Hex layers" tally counts the files the grade reads.** A file the grade excludes (by `analyze.exclude` or the built-in patterns) is not counted in any layer.
4. **The session context names Python.** A project whose root has `pyproject.toml`, `setup.py` or `setup.cfg`, and none of `Cargo.toml`, `go.mod` or `package.json`, is shown as language `python`.
5. **A subagent whose definition grants only tools that read is read-only.** When the subagent type has a definition (`.claude/agents/<type>.md` in the project, else in the user's home) whose `tools` list names only tools that read (`Read`, `Grep`, `Glob`, `LS`, `NotebookRead`, `WebFetch`, `WebSearch`), the pre-agent hook lets it run without a worktree, as it does for the fixed read-only types. A definition with no `tools` list, one that names `*` (every tool), or one that names any other tool (`Edit`, `Write`, `Bash`, an MCP tool, …) is treated as before: a tool hexa does not know to be read-only is assumed to write.

## Consequences

- A failure to reach the model, a benchmark with no runner, and a layer count are each reported as they are.
- A read-only reviewer runs in the main checkout, where it can read uncommitted work, instead of being sent to a worktree cut from HEAD.
- No check becomes weaker: an agent that can write, or one whose tools hexa does not know, is blocked or warned as before.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611221646_spec_2 --test adr_2611221646_spec_3`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2610270901: `claude-text`, a tool-less completion
- ADR-2611161800: a plan ends on main (the hooks)
- `hexa-cli/src/commands/hook/mod.rs`, `hexa-cli/src/commands/bench.rs`, `hexa-cli/src/commands/analyze.rs`, `hexa-infer/src/adapters/secondary/claude_code.rs`
