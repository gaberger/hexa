# ADR-2609122048: a tool that reports "nothing found" must prove it looked

**Status:** Accepted
**Date:** 2026-09-12
**Epoch:** hexa
**Drivers:** An audit ran every verb in the CLI. Three commands answered confidently while checking nothing, and one of those answers tells an agent to delete the core crate.

## Context

Every verb of the CLI was run and its exit code and output recorded. Most
behave. Three do not, and they share one defect: each reports a clean result
after failing to perform the check it claims to perform.

**`hexa graph consumers hexa-core/src/lib.rs` prints SAFE TO REMOVE.**
Fifteen files name `hexa_core::`. The import resolver handles relative
TypeScript paths and Rust `self::`, and returns `None` for everything else.
A Rust cross-crate `use` names the crate, not the file, so no edge ever
reaches a crate root. The one command whose stated purpose is "who depends on
this, before you delete it" gives a false all-clear on every crate root in
every Rust workspace.

**`hexa dev test arch` passes without running the analyzer.** It looks for a
TypeScript CLI that was deleted (`npx hexa`, `bun run src/cli.ts`), then for
the binary, and accepts an answer only if the output contains `Grade:`. The
analyzer prints `Architecture grade:`. No candidate matches, the analyzer is
skipped, and the category passes on a `cargo test` that runs zero tests.

**A missing tool is reported as a failing test.** `cargo_test` and
`cargo_check` map any spawn error to `false`, so on a machine without cargo
the report reads "hexa-core tests fail" seven times over. The tests did not
fail. They never ran.

The project's own lesson names this class: a gate that degrades silently is
worse than no gate. These three degrade silently in the tool that enforces
that lesson on everyone else.

Three smaller defects come from the same audit. `hexa dev test lint` runs
`bun run check` in a repository with no TypeScript. `hexa spec list`, `hexa
spec show` and `hexa adr specs` exit 1 when their directory is absent, which
makes "this project has no specs" indistinguishable from a failure. `hexa
docs check` exits 1 on a warning with zero errors.

## Decision

1. **A check that cannot run says so, and does not report a result.** Absent
   tooling is its own outcome, distinct from pass and from fail. `cargo not
   found` is not a failing test. An analyzer that could not be located fails
   the architecture category loudly rather than skipping it.

2. **The binary locates itself.** A command that needs to run hexa uses
   `std::env::current_exe()`, never a package manager, never a source path,
   never PATH. There is no second implementation to fall back to.

3. **Import resolution covers the languages the analyzer claims.** Rust
   `crate::`, `super::` and cross-crate `<crate>::` paths resolve to files,
   the same as relative TypeScript imports already do. A crate root imported
   by name is imported.

4. **Absence is a result, not an error.** A missing `docs/specs/` directory
   means the project has no specs. The command says that and exits 0. Exit 1
   is reserved for a question that was asked and answered badly.

5. **A warning does not fail a command.** Only an error sets a non-zero exit.

## Consequences

`graph consumers` gains edges it never had, so verdicts change from SAFE TO
REMOVE to BLOCKED across Rust workspaces. That is the correction, and it
makes the command stricter than it was.

`hexa dev test arch` starts failing wherever the analyzer's grade is below A,
which was always its stated contract and has been unenforced.

A project with no specs and no workplans now reports empty rather than
erroring, so those exit codes stop being usable as a "directory exists"
probe. Nothing in the repository used them that way.
