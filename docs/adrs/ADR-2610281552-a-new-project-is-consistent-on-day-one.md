---
id: ADR-2610281552
status: proposed
date: 2026-10-28
---
# ADR-2610281552: a new project is consistent on day one

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** A project made by `hexa dev new --lang rust` on 2026-10-28 failed `hexa adr doctor` with 12 errors before a line was written: the shipped `.hexa/ADR-rules.toml` and the `/hexa-adr-create` skill cite hexa's own decisions (`ADR-2609121400`, `ADR-2609211430`, `ADR-2609211600`, `ADR-2026-04-05-0900`), which are not in the new project's ledger. The same project had no `.gitignore`, so its first `git add -A` committed `target/`.

## Context

A check that fails on a fresh project teaches its user to ignore it. The citations are correct: they name the decisions hexa's rules come from. They are hexa's, not the project's, and the doctor has no way to tell the two apart.

## Decision

1. **The binary knows hexa's own decisions.** `hexa-cli`'s build reads the file names in `docs/adrs/` of the hexa source tree and embeds the ids `adr_ids(file_names)` in `hexa-core/src/domain/project_floor.rs` returns: for each name matching `ADR-*.md`, the id it starts with (the name up to the first `-` after `ADR-`'s number, or the whole stem), in sorted order without duplicates. A build without that directory embeds none.
2. **A citation resolves against the project's ledger, then hexa's.** `cited_ledger(id, project_ids, hexa_ids)` in `hexa-core/src/domain/project_floor.rs` returns `Project` when the project's ledger holds the id, else `Hexa` when hexa's does, else `Dangling`. `hexa adr doctor` reports only `Dangling` citations as `DanglingCitation`. In hexa's own repository the two ledgers are the same set, so nothing it checks today goes unchecked.
3. **A new project ignores its build output.** `gitignore_lines(lang)` in `hexa-core/src/domain/project_floor.rs` gives the lines a language's build output needs: `rust` → `target/`; `ts` → `node_modules/`, `dist/`; `go` → none. Every language adds `.hexa/loop.json` and `.hexa/loop.lock`, which are one checkout's session state. `merged_gitignore(existing, lang)` returns the file to write: the existing text with each missing line appended once, or `None` when nothing is missing. `hexa init` (and so `hexa dev new`) writes it.

## Consequences

- `hexa adr doctor` passes on a project hexa has just made.
- A new project's first commit holds its source, not its build.
- A citation of a hexa decision in a user's own text is resolved, not reported; it names a real decision.

## Implementation

`hexa-core/src/domain/project_floor.rs`, `hexa-cli/build.rs`, `hexa-cli/src/commands/adr/doctor.rs`, `hexa-cli/src/commands/init.rs`.

## Gate

`hexa spec verify ADR-2610281552`: the spec's sealed tests, each red before the code.

## References

- ADR-2609151930: every cited id must resolve.
- ADR-2609121400: `hexa new` gives a project that runs.
