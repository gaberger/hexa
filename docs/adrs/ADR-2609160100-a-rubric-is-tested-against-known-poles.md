---
id: ADR-2609160100
status: accepted
date: 2026-09-16
---
# ADR-2609160100: A rubric is tested against known poles, or it is speculation with a table

**Status:** Accepted
**Date:** 2026-09-16
**Drivers:** The build trial's defect probes were ad-hoc scripts. One destroyed the data it then reported missing, producing a published number that was wrong against this project's own tool. When the probes were rebuilt with a self-test, it immediately found two of six could not fail at all, and fixing one of those found a real defect in an implementation the broken probe had cleared.

## Context

This project already refuses a gate that exits 0 having run no tests
(`evidence_is_vacuous`). It applied no such rule to the instruments it uses to
judge other things. The build trial measured three implementations with six
shell probes that had never been shown to detect anything.

Two failures followed directly. `p5` overwrote a store file and then reported
the absent record as data loss, scoring an arm for a defect the probe had
caused. `p2` sent an oversize URL rather than an oversize body, so two
different rejections were indistinguishable and the probe passed everything.

Both were found by a reader's question and by a self-test, not by any process
in the trial.

## Decision

A measuring instrument in this repository is fit to produce a number only when
it has been shown to discriminate between a known-good and a known-bad
reference.

1. **Two poles, in tree.** `bench/fixtures/sound` is correct.
   `bench/fixtures/unsound` carries every defect the probes test, each tagged
   `DEFECT-Pn` at the line that causes it.
2. **Every probe must pass the sound pole and fail the unsound pole.**
   A probe that passes both is VACUOUS; one that fails the sound pole is
   BROKEN. `bench/selftest.sh` reports either and exits non-zero.
3. **The runner refuses to run** until the self-test passes, so no table is
   ever produced by an instrument that has not been validated.
4. **A probe whose defect is unreachable is retired, not kept green.** It moves
   to `probes/retired/` with the evidence that the defect cannot occur.
5. **The behavioural gate must pass on both poles.** If the gate can tell the
   fixtures apart, the defects are not the kind this bench exists to find.

## Consequences

- A new probe cannot be added without adding its defect to the unsound
  fixture; otherwise it reports VACUOUS and blocks the bench. That is the
  intended cost.
- Published numbers from `bench/run.sh` carry the claim that each column could
  have come out the other way.
- The rule generalises beyond this bench. Any future scoring instrument in this
  repository, including the architecture grader's detectors, is subject to the
  same question: what known-bad input makes it fire?

## Gate

`./bench/selftest.sh`

Exits 0 only when every probe passes the sound fixture, fails the unsound
fixture, and the behavioural gate is green on both.

## Implementation

`bench/{selftest.sh,run.sh,gate.sh,README.md,CHALLENGE.md}`,
`bench/probes/*.sh`, `bench/fixtures/{sound,unsound}`.

## References

- ADR-2609132122 — a recorded gate is run, or it is prose.
- `docs/analysis/2609152230-build-trial-results.md` — the trial whose numbers
  this ADR corrects.
- CLAUDE.md § Key lessons — "a vacuous gate is a failed gate".
