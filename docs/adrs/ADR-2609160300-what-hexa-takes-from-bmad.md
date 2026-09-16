---
id: ADR-2609160300
status: accepted
date: 2026-09-16
---
# ADR-2609160300: What hexa takes from BMAD

**Status:** Accepted
**Date:** 2026-09-16
**Drivers:** In a head-to-head build (docs/analysis/2609152230), BMAD-METHOD reached 2 probed defects in 29 minutes for $0 metered, against hexa's 0 defects in 72 minutes for $24.56 and rising. BMAD's review also found a class of fault hexa has no verb for: a stated requirement that nothing verifies.

## Context

hexa won the defect count and lost everything else that was measured. Three
differences in BMAD's method account for most of the gap, and none of them is
a matter of model quality.

### 1. BMAD found the gap in the gate; hexa cannot

BMAD's review reported that deleting the cache adapter from its own composition
root would have left every test and the functional gate green. The requirement
was in the brief and verified by nothing.

This was independently confirmed against a competing arm: deleting
`caching-link-store.ts` outright from the Spec Kit tree left `gate.sh` printing
PASS. The gate ran eleven correct checks and could not see a missing component.

hexa's guard against a worthless gate is `evidence_is_vacuous`, which catches a
command that exits 0 having run no tests. It cannot catch a gate that runs
eleven good checks and misses a twelfth requirement. **The project's central
claim is that the gate replaces the spec, and it has no way to ask whether the
gate covers the spec.**

### 2. BMAD reviews in-context; hexa always pays frontier

Measured over the trial window, hexa's adversarial pass dispatched 32 calls to
the `claude -p` path over 61 minutes for $24.41 — 99% of every dollar the tool
could account for, spent after the code already worked. BMAD's three review
layers ran inside the agent's own context: 262,758 session tokens, 207 tool
calls, no metered spend, two thirds of the defects caught.

hexa's tier configuration points T1, T2 and T2.5 at a local `gpt-oss-120b`.
During the trial the local model served **1 of 34 calls**. The expensive step
routes to the frontier by construction, so local hardware does not reduce its
cost.

### 3. BMAD records what it declined to fix; hexa does not

BMAD produced `deferred-work.md`: four findings it chose not to act on, each
with evidence. hexa's harden fixes a confirmed finding or drops it silently, so
a reader cannot tell a clean run from an unrecorded judgement call.

### A fourth defect, found while measuring

`hexa spend` reports the model as the literal string `claude-code`
(`hexa-exec/src/frontier.rs:27`). That is a code path, not a model. The frontier
call passes no `--model`, so whatever the CLI happened to default to is what was
billed, and the log cannot say which. Separately, `hexa build` records no spend
at all, so the $24.56 above is a floor. The verb that does the most work reports
nothing to its own accounting.

## Decision

Adopt three things from BMAD and fix the accounting.

1. **Deletion coverage: `hexa gate coverage --gate '<cmd>'`.** For each exported
   module in the target, remove it, run the gate, and restore. Any module whose
   deletion the gate survives is verified by nothing and is reported. This is
   mutation testing at component granularity and it is the runnable form of the
   question BMAD asked by reading. A gate that survives the deletion of a
   required component is a failed gate, by the same rule that rejects a vacuous
   one.
2. **A local lens before the frontier lens in `hexa harden`.** The hunt runs
   first against the configured tier model and only escalates candidates it
   cannot resolve. The frontier stays the refuter, not the hunter. Target: the
   same confirmed findings at a fraction of 32 frontier calls.
3. **`deferred.md` from every harden run.** Every candidate that was confirmed
   and not fixed, and every candidate refuted, with the evidence that decided
   it. A harden run that fixes everything writes an empty file and says so.
4. **The spend log names the model.** The frontier path passes an explicit
   model and records it. `hexa build` records its spend like every other verb.
   `hexa spend` stops reporting a code path where a model belongs.

## Consequences

- Item 1 gives the project the check its own pipeline rule implies and has
  never had. It is also slow: one gate run per module. It belongs in `harden`
  and in CI, not in the inner loop.
- Item 2 trades some hunt quality for cost. If the local lens finds materially
  fewer real defects, that is a measurable result and the decision reverses.
- Item 4 is the precondition for every other cost claim this project makes.
  Until it lands, no benchmark in this repository can say what it paid for.

## Built, 2026-09-16, and one claim corrected

Items 4 and 1 are built; 2 and 3 are not. What the build learned:

**Item 1 does not do what this ADR said it would.** The ADR claimed deletion
coverage was "the runnable form of the question BMAD asked by reading",
citing the cache adapter the gate could not see. That is wrong, and running it
proved so. Deleting a file breaks the import that names it, so the gate fails
and the module reads as covered. Verified directly: removing
`caching-link-store.ts` from the Spec Kit tree makes the server fail to start.

The original case needed the component **unwired** from the composition root,
not deleted. Deletion coverage answers "does anything the gate runs need this
file", which is a real and different question. A behaviourally redundant
decorator that is still imported is invisible to it. Catching that needs a
wiring mutation — replace a component with a pass-through in the composition
root — which is not built and is not this verb.

**What it did find**, on its first run against the Spec Kit arm: the only two
modules whose deletion the gate survived were its two ports. Both are
TypeScript interface files, erased before the program runs, so the gate could
never have depended on them. That is a property of the language, not a hole,
and reporting it as one was noise. The verb now classifies type-only modules
separately and only counts runtime modules as holes. On that tree it now
reports 0 of 9 runtime modules verified by nothing, and exits 0.

**A second defect, found by building this.** `hexa adr gates` judged a gate
naming a relative script as "not a command", because the runnability check
resolved the path from the process's working directory while the runner
executes gates from the repository root. This ADR's own sibling,
ADR-2609160100, records `./bench/selftest.sh` and was failing that check.
Fixed: the check now resolves against the root the gate will run in.

## Gate

`cargo test -p hexa-cli --lib gate_coverage && cargo test -p hexa-infer --lib spend_names_a_model && cargo test -p hexa-cli --lib adr_gates`

The `harden_lens_order` clause is removed until item 2 is built; a filter that
matches no test is a vacuous pass, which this project rejects.

Each module must exist and fail before the code that satisfies it.

## Implementation

Ordered by value per unit of work:

1. Item 4, the accounting. An hour, and every later measurement depends on it.
2. Item 1, deletion coverage. The novel capability; validate it against the
   trial fixtures, where the cache adapter is known to be uncovered.
3. Item 3, the deferred record. Small.
4. Item 2, the lens order. Needs a before-and-after defect count on the bench
   fixtures to justify itself.

## References

- `docs/analysis/2609152230-build-trial-results.md` — the head-to-head.
- ADR-2609160100 — a rubric is tested against known poles.
- `bench/` — where deletion coverage should be validated.
- CLAUDE.md § Key lessons — "a vacuous gate is a failed gate".
