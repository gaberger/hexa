---
id: ADR-2610281442
status: proposed
date: 2026-10-28
---
# ADR-2610281442: a sealed test can pass, and its writer sees the API it tests

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** Running the whole loop on an example app (`~/projects/hexa-egm-example`, 2026-10-28) sealed tests that no implementation can pass. The test writer is shown only the port and domain files an ADR names, so for a decision about a use-case constructor it sealed a helper that is an unconditional `todo!`/`panic!` — 11 tests of 41, in 4 of 7 generations. A test that panics fails before the code and after it alike, so the red check accepted it: a vacuous gate, which ADR-2610221540 forbids. Once the code existed, the broken tests could not be replaced either: `hexa spec tests` requires every new test to fail at HEAD, and HEAD held the code. The same wall holds ADR-2610271736's S24.

## Context

ADR-2610221540 §5 seals tests that fail before the code exists. "Fails before" is half of what a gate needs; the other half is "can pass after". Nothing checks the second half. The writer is kept from the implementation so it cannot mirror it (ADR-2610221540 §2), but a signature is not an implementation: withholding the public API of a use case or an adapter does not stop mirroring, it stops the test from being written.

## Decision

1. **The test writer sees the public signatures of every file the ADR names, never a body.** `public_signatures(source)` in `hexa-core/src/domain/spec.rs` returns a Rust file's `pub` items — structs, enums, traits, `impl` blocks and functions — with every function body replaced by `{ … }` and every private item and `mod tests` removed. `hexa spec tests` gives the writer the ADR, the port and domain files whole (as today), and `public_signatures` of each other `.rs` file the ADR names in backquotes. `hexa spec draft` is unchanged: the spec author still sees port and domain files only.
2. **A generated test that cannot pass is refused.** `unpassable_sites(test_source)` in `hexa-core/src/domain/spec.rs` returns, with line numbers, every `todo!(` and `unimplemented!(`, and every `panic!(` that is the first statement of a function or closure body. `hexa spec tests` refuses a generation with any such site, naming each, the same way it refuses one that leaves a scenario without a test. A `panic!` inside an `unwrap_or_else`, a `match` arm or an assertion message is not a site.
3. **Red can be proved at an earlier commit.** `hexa spec tests <ADR> --red-at <commit>` runs the red check in a worktree at `<commit>`, which must be an ancestor of HEAD, with the generated file added. Every test must fail there. hexa records the resulting commit (`<commit>`'s tree plus the test file) under `refs/hexa/red/<ADR>`, records it as the spec's `red`, and commits the test file onto HEAD. `hexa spec verify --prove` checks red at that commit as it does any other.
4. **A sealed test file is replaced only through the verb.** `hexa spec tests <ADR> --replace --red-at <commit>` removes the sealed file and regenerates it under §2 and §3, in one commit whose message names the removed file and the reason given with `--because "<text>"`, which is required. The acceptance is untouched: the spec did not change, only its tests.

## Consequences

- The writer can construct what it tests, so it stops stubbing constructors.
- A placeholder cannot reach a seal; a gate that cannot pass is refused before it is recorded.
- A defective sealed test can be repaired after the code exists, honestly: red is proved at a commit that predates the code, and the replacement is recorded with its reason.
- Not decided here: a test that defeats itself by searching `tests/` for text its own file holds (S41 in the example). That is a scenario's wording, and is fixed in its spec.

## Implementation

`hexa-core/src/domain/spec.rs` (`public_signatures`, `unpassable_sites`), `hexa-exec/src/spec_author.rs` (the writer's inputs, the refusal, the red check at a commit), `hexa-cli/src/commands/spec.rs` and `hexa-cli/src/commands/spec_ledger.rs` (`--red-at`, `--replace`, `--because`).

## Gate

`hexa spec verify ADR-2610281442`: the spec's sealed tests, each red before the code.

## References

- ADR-2610221540: the gate-first pipeline; §2 the writer's inputs, §5 red.
- ADR-2610271736: S24, the first sealed test this repairs.
- ADR-2610281011: the design stage the example ran through.
