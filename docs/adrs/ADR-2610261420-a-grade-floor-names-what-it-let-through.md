---
id: ADR-2610261420
status: accepted
date: 2026-10-26
---
# ADR-2610261420: A grade floor names what it let through; the scaffold fails on any violation

**Status:** Accepted
**Date:** 2026-10-26
**Drivers:** Issue #20. `hexa analyze . --grade A` exited 0 on a tree with a boundary violation. One violation costs 10 points: 100 − 10 = 90, which is an A. The README and getting-started guide showed `--grade A` as the gate, and `hexa scaffold` enforced only the letter, while its own prompt tells the model that "a use case importing an adapter is a failed build here".

## Context

`hexa analyze` already has the strict gate: `--exit-code` exits 1 on any
boundary violation or rule error. `--grade` is a letter floor by design: "a
grade with no floor passes by definition, and a reader who wants B to fail says
so" (`analyze.rs`). hexa's own CI runs both: `--grade A+ --exit-code`.

The defect was in two places that treated the letter as if it meant "no
violations":

1. **The docs.** README, GETTING-STARTED, the development-workflow guide and the
   glossary presented `--grade A` alone as the CI gate.
2. **`hexa scaffold`.** It compared `got < want` only, so a scaffold with exactly
   the violation its prompt forbids graded A, met the default floor A, and
   printed "is built, gated and graded".

And `hexa analyze --grade A` said nothing when it passed with a violation. A
pass that hides what it let through is the gate that degrades silently.

## Decision

1. **`--grade` keeps its meaning.** It is a letter floor. Making it fail on any
   violation would make every floor below A+ meaningless as a floor.
2. **A floor that passes with violations says so.** It prints
   `! grade A passes the floor A with 1 boundary violation — --grade is a letter
   floor; add --exit-code to fail on any`. The count is the one the score was
   computed from. `all_violation_count` sums the import scan and the deep pass,
   which see the same edge twice, so it reads 2 for one violation. It stays
   the "any?" signal for `--exit-code`.
3. **`hexa scaffold` fails on any boundary violation**, whatever the floor. The
   pass condition is `gates_met`: test gate passed, grade at or above the floor,
   zero boundary violations. That is what its prompt has always told the model.
4. **The docs show the gate that is enforced:** `--grade A --exit-code` for CI,
   and the scaffold's gate as "grade earned and no import crosses a boundary".

## Consequences

- A CI job that runs `--grade A` alone keeps passing as before, and now prints
  a warning that names the flag it is missing.
- A `hexa scaffold` run that used to report success with one violation now
  fails. That is the intended change: its prompt already called it a failure.
- The double count in `all_violation_count` is noted, not fixed. It is only used
  as "any?" and in the score fallback when the deep pass did not run.

## Implementation

- `hexa-cli/src/commands/analyze.rs`: `graded_violations`, and the warning after
  the floor holds.
- `hexa-cli/src/commands/scaffold.rs`: `gates_met`, and the violation count on
  the grade line and in the failure message.
- `README.md`, `docs/GETTING-STARTED.md`, `docs/guides/development-workflow.md`,
  `docs/reference/glossary.md`.
- Gates, written first and seen failing:
  `hexa-cli/tests/grade_floor_names_what_it_let_through.rs` (drives the binary on
  a one-violation tree: `--grade A` passes and warns; `--grade A --exit-code`
  exits 1; a clean tree passes silently), and
  `scaffold::tests::one_boundary_violation_fails_the_scaffold_even_above_the_floor`.

## Gate

`cargo test -p hexa-cli --test grade_floor_names_what_it_let_through && cargo test -p hexa-cli one_boundary_violation_fails_the_scaffold`: `--grade A` passes a one-violation tree and warns, naming `--exit-code`; `--grade A --exit-code` exits 1; a clean tree passes silently; the scaffold fails on one violation above its floor.

## References

- Issue #20; related #19 / ADR-2610261356
- ADR-2609211430 — rule errors cost the grade
