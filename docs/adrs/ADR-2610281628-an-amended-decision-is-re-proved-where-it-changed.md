---
id: ADR-2610281628
status: proposed
date: 2026-10-28
---
# ADR-2610281628: an amended decision is re-proved where it changed

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** ADR-2610281516 changed how `claude-text` is called (JSON output, the prompt on stdin) and how a rejected test generation is handled (repaired, not refused at once). That supersedes five accepted scenarios of ADR-2610270901, which name `--output-format text` and "the prompt argument", and it breaks the sealed test of ADR-2610221540 S15b, whose fake backend serves exactly one reply. The scenarios had to change and the test had to be re-proved, but hexa had one way to do either: re-accepting a changed spec drops every scenario's test, red commit and seal, and regenerating the whole file cannot be red, because the unchanged scenarios' code already exists and passes.

## Context

A test binds one scenario, and its red commit proves it was written before that scenario's code. Neither fact changes when another scenario of the same spec changes. `hexa spec verify --prove` already counts a test that did not build at its red commit as red ("written first"); the generator alone demanded a test that runs and fails, which a test written before its API cannot do.

## Decision

1. **Re-accepting keeps what did not change.** `rebind(previous, accepted)` in `hexa-core/src/domain/spec.rs` returns the accepted spec with each scenario's `test` and `red` carried over from `previous` when that scenario's id exists there with the same `decision`, `kind`, `description`, `given`, `when` and `then`; a scenario that changed, and one that is new, has none. A scenario whose own `red` is unset keeps the spec-level `red` it was proved at, written onto the scenario. `hexa spec accept` uses it. A seal is dropped only for a file that held the test of a scenario that changed or was removed.
2. **Named scenarios are re-proved alone.** `hexa spec tests <ADR> --only <ids> --because "<reason>"` generates tests for exactly those scenarios into a new file, `<crate>/tests/<adr>_spec_<n>.rs` with the first free `n`, proves each red (§3), binds each scenario to its new test with that red commit, and seals the new file. `without_tests(source, names)` in `hexa-core/src/domain/spec.rs` removes each named `#[test]` function, with its doc comments and attributes, from the file that held it, and that file is sealed again. All of it is one commit, whose message names every file and the reason. `--because` is required when a named scenario still has a test; a scenario with none (§1) may be named without it.
3. **A test written before its API is red.** With `--red-at <commit>`, a test that does not build at that commit counts as red there when the whole new file builds at HEAD, as `--prove` counts it. A test that builds and passes at the red commit is still refused.

## Consequences

- A decision can be amended without throwing away the proof of every scenario it did not change.
- A sealed test that a later decision made wrong is replaced on the record, with its reason, never edited.
- The generator and `--prove` agree on what red means.

## Implementation

`hexa-core/src/domain/spec.rs` (`rebind`, `without_tests`), `hexa-exec/src/spec_author.rs`, `hexa-cli/src/commands/spec.rs`, `hexa-cli/src/commands/spec_ledger.rs`.

## Gate

`hexa spec verify ADR-2610281628`: the spec's sealed tests, each red before the code.

## References

- ADR-2610221540: the gate-first pipeline, S15b.
- ADR-2610270901: `claude-text`, S01, S08, S09, S11, S12.
- ADR-2610281516: the decision that supersedes them.
- ADR-2610281442: `--red-at` and `--replace`.
