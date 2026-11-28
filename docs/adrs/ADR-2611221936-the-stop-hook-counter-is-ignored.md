# ADR-2611221936: The Stop hook's counter is session state, kept out of git

**Status:** Completed
> 2026-11-22 Completed: landed on main at ddb17c4; gate `cargo test -p hexa-cli --test adr_2611221936_spec` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no new or changed functionality); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-22
**Drivers:** a Python SDK found `.hexa/stop-blocks.json` untracked after `hexa init`: the Stop hook writes it, and neither `.hexa/.gitignore` nor the root `.gitignore` that `hexa init` writes names it, so it shows in every `git status` and can be committed. hexa's own repository ignores it by hand. ADR-2610281552 §3: a new project's first commit holds its source, "not its build output, one checkout's session state, or a secret".
**Supersedes:** none. A defect in what `hexa init` ignores.

## Context

- The Stop hook (`hexa-cli/src/commands/hook/mod.rs`, `stop_until_finished`) counts, per session id, how many times in a row it has blocked a stop, in `.hexa/stop-blocks.json`. It is one checkout's session state.
- `ensure_ignored` (`hexa-cli/src/commands/task_ledger.rs`) writes `.hexa/.gitignore` with the task ledger's runtime files (`tasks.json`, `tasks.lock`, `.tasks.json.*.tmp`), adding only the lines missing (ADR-2611051850). `hexa init` calls it, and so does the task ledger when it writes.
- The project floor's ignore lines are `hexa-core/src/domain/project_floor.rs` (`merged_gitignore`).

## Decision

1. **`.hexa/.gitignore` names `stop-blocks.json`.** `ensure_ignored` adds it with the task ledger's lines, so `hexa init`, and any later write of the task ledger in an existing project, leaves it ignored. A line already there is not written twice.
2. **Nothing else changes.** The lines `ensure_ignored` already writes, the root `.gitignore`, and what the Stop hook records are as before.

## Consequences

- A project's `git status` no longer shows the Stop hook's counter, and it cannot be committed by accident.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611221936_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2610281552: a new project's first commit (§3, the ignore lines)
- ADR-2611051850: the task ledger (its runtime files are ignored)
- ADR-2611161800: a plan ends on main (the Stop hook, §5)
- `hexa-core/src/domain/project_floor.rs`
