---
id: ADR-2610221540
status: accepted
date: 2026-10-22
---
# ADR-2610221540: An ADR is compiled to a spec, the spec to sealed tests that fail, and only then is code generated

**Status:** Accepted
**Date:** 2026-10-22
**Drivers:** On 2026-10-22 the operator asked how well the tests guard what the ADRs decide, and named the missing piece: an ADR carries the design requirements, and something has to convert them into tests before the code is generated, or the code drifts from the decision. An audit the same day found:
- `hexa adr gates` reported 20 of 45 ADRs with no gate. Twelve of them have one, written `**Gate:**`, which the extractor does not read.
- One gate's TypeScript, Go and Rust cases print `skipping:` and pass on the forge that merges PRs.
- ADR-2609132122 states the gap and leaves it open: "Twice today a numbered decision went unimplemented while its gate passed."

## Context

The pipeline today goes from decision to gate to code. ADR-2609121400 removed the step between them on purpose. A written spec is prose, and prose cannot fail. So the gate replaces the spec. In `hexa build` the spec is synthesized during divergence and thrown away.

That was right about prose and wrong about the step. Three things go missing without it:

| Missing | Effect |
|---|---|
| **A translation of each requirement into checkable behaviour.** The ADR says *what must be true*. The gate is one command chosen by whoever writes the code. Nothing maps numbered decision §n to the cases that prove it. | A decision is skipped or reverted under a green gate (ADR-2609132122). |
| **Independence between the tests and the code.** The agent that writes the code writes or edits its gate in the same run. The test then encodes the agent's reading of the ADR, not the ADR. This is the first lesson in CLAUDE.md, "tests can mirror bugs". | Tests pass and the business logic is wrong in the way the tests are wrong. |
| **A point where a person checks the business logic before code exists.** Reviewing generated code to find out what it decided is the expensive, unreliable place to do it. | Wrong logic is found in harden or production, not at decision time. |

hexa already holds the shape of the missing layer and never finished it. `hexa spec` parses `given / when / then` scenarios with `negative_spec` flags (`hexa-cli/src/commands/spec.rs`). `docs/specs/` does not exist, though. No scenario becomes a test, and `impl_check` greps source for function names. That is prose with extra steps, which is why ADR-2609121400 was right to distrust it.

The decision below keeps ADR-2609121400's rule, "a spec that cannot be run does not exist". It satisfies the rule differently: the spec exists only as tests that run, that failed before the code, and that the code may not touch.

## Decision

1. **Every accepted ADR has a spec: `docs/specs/<ADR-id>.json`.** It extends the existing `SpecFile`. Each scenario gains:
   - `decision: ["§n", …]`, the numbered decisions it proves;
   - `kind: example | negative | property`;
   - `test: "<path>::<fn>"`, set when the test is generated.

   The spec is the operational form of the ADR. It is not disposable, and it is versioned with the ADR it serves.

2. **The spec is derived from the ADR and the ports, never from the implementation.** `hexa spec draft <ADR>` gives the spec author the ADR text, and the port and domain signatures the decision names, and nothing else. Port signatures are part of the decision: a new port is written with the ADR, before the spec. The author is a separate agent call from the one that writes code, at the tier `.hexa/project.json` names for it.

3. **Coverage is checked deterministically.** `hexa spec check <ADR>` fails unless each numbered decision has:
   - at least one `example` scenario;
   - at least one `negative` scenario;
   - for a decision that states a rule over all inputs, a `property` scenario.

   A decision with no testable behaviour is exempted in the spec with a reason, and the reason is printed on every run. No model is involved in this check.

4. **A person accepts the spec. That is where business logic is checked.** `hexa spec accept <ADR>` records who accepted which spec hash. Reading twenty `given / when / then` lines is the cheap, reliable place to find a wrong rule. `hexa adr accept` approves the decision. `hexa spec accept` approves what it means in behaviour.

5. **Scenarios compile to tests that must fail first.** `hexa spec tests <ADR>` generates one test per scenario, with the scenario id in the test's doc comment.
   - The generator sees the accepted spec and the port signatures, and not the implementation.
   - Property scenarios become property tests, which are the independent oracle CLAUDE.md asks for.
   - The tests are run before any implementation. Each must compile and fail. A test that passes against an unimplemented port tests nothing, and it is rejected.
   - The commit that holds them is recorded in the spec as `red`.

6. **The tests are sealed.** The spec records a SHA-256 for each generated test file. In `hexa build --spec <ADR>` and `hexa do run --spec <ADR>`, a sealed file is outside the write set of the guarded tool library, and an edit to it fails the run. The gate of a spec-driven build is the sealed tests plus the architecture grade. The code agent can only make the tests pass. It cannot change what passing means.

7. **A requirement changes through the ADR, never the test.** Changing behaviour means a new or amended ADR (append-only, ADR-2609151930), then a revised spec, a new `spec accept`, regenerated tests and a new seal. A sealed test edited by hand fails `hexa spec verify`.

8. **Drift is a CI failure.** `hexa spec verify [--prove]` runs on every PR on both forges. For each spec it checks that:
   - the hashes match the seal;
   - each scenario's test runs alone with `--exact` and reports exactly one test passed (zero is vacuous);
   - no test printed a `skipping:` line, which is reported as *not proven here* and fails under `--ci`.

   With `--prove` it also checks out each `red` commit in a temporary worktree and requires the test to fail there. A test green at its red commit is reported as a mirror.

9. **The existing ledger is ratcheted, and the mechanical faults are fixed now.** ADRs accepted before this one have no spec. They are listed as legacy, and their count may only fall; a test pins the ceiling, as `hexa_grades_itself_by_ratchet` does. Until each one is backfilled:
   - `hexa adr gates` also reads a `**Gate:**` / `**Gate**` line;
   - a skipped case is *not proven here*, as in §8;
   - ADR-2609132158's gate asks the forge for runs on the branch prefixes in use.

## Consequences

- Each numbered decision traces to an accepted scenario, and each scenario to a sealed test that failed before the code. "Is what we decided still true" is answered per decision, in CI, on every PR.
- The tests and the code come from different inputs and different agent calls, and the code side cannot write to the tests. The mirror failure needs the spec author, the test generator and the accepting person to make the same mistake.
- **The business logic is only as right as the accepted spec.** This moves the correctness question to a short, readable artifact and puts a person on it. It does not answer the question by itself.
- `hexa build` changes shape. Divergence and red-team still propose designs. They design to a fixed spec and fixed tests instead of synthesizing a throwaway spec. This amends step 3 of the pipeline in ADR-2609121400. Its two gates stand.
- Writing an ADR costs more: a spec review per decision. Generated tests cost inference at T2 or above.
- `--prove` checks out and builds old commits, so it is slow. CI runs it only on specs a PR touches.
- **What this does not do:** judge whether the ADR asked for the right thing. It makes what was asked executable and makes drifting from it fail.

## Implementation

Phased. Each phase has its own gate, written before its code.

1. **Ledger fixes (§9).** These are mechanical and need no new port.
   - The `**Gate:**` extraction, the *not proven here* outcome and the corrected ADR-2609132158 gate go in `hexa-cli/src/commands/adr/`.
   - The forge CI gains `HEXA_TEST_NPM`, `HEXA_TEST_CARGO_NET`, Node and Go, so the adapter gate runs where PRs merge.
   - **Gate:** `cargo test -p hexa-cli --test the_ledger_reads_every_gate_and_no_skip_passes`.
2. **Spec format and the deterministic checks (§1, §3, §8 without `--prove`, §9 ratchet).**
   - The scenario schema and the coverage rules are pure domain code: parsing, coverage and seal comparison, with no I/O.
   - `hexa spec check | verify` are the verbs.
   - **Gate:** `cargo test -p hexa-cli --test a_decision_is_covered_by_a_sealed_spec`.
3. **Draft, accept and generate (§2, §4, §5).**
   - Spec authoring and test generation are use cases behind the existing inference port. Tier selection only: no model id outside `hexa-infer`.
   - **Gate:** a fixture ADR is drafted, accepted and generated against a double inference adapter. The generated tests compile and fail against the unimplemented port, and the red commit is recorded.
4. **Sealing in build and do (§6, §7), and `--prove` (§8).**
   - Sealed paths are removed from the guarded tool library's write set.
   - **Gate:** a `hexa do run --spec` whose agent edits a sealed test fails the run and commits nothing. A test green at its red commit is reported as a mirror.
5. **Backfill**, starting with the ADRs whose gates already name a test per decision. Each backfill lowers the ratchet.

This ADR adds no external dependency. A property-test crate for generated Rust tests needs its own ADR in phase 3.

## References

- ADR-2609121400: hexa is a scaffolding system with two gates. This keeps its rule that a spec which cannot run does not exist, and amends step 3 of its pipeline.
- ADR-2609132122: a recorded gate is run or it is prose. Its open gap is closed by §3 and §8.
- ADR-2609132158: a gate that never runs on the branch is not a gate. Its gate is corrected by §9.
- ADR-2609151930: an accepted decision is append-only and remembered. Hence §7.
- ADR-2609211245: the TypeScript gate runs where the network is. Its skip shape is §8.
- ADR-2610100005: the HTTP adapter is generated from the contract. Its gate skips on the forge. It is also the precedent for generating code from a fixed, checked artifact.

## Gate

`cargo test -p hexa-cli --test the_ledger_reads_every_gate_and_no_skip_passes`

This is phase 1's gate, red at `bea3be4`. Each later phase appends its gate here as a new `## Gate`
section, which `hexa adr gates` reads as current, with every earlier phase's test carried into
the command.

## Gate (phase 2)

`cargo test -p hexa-cli --test the_ledger_reads_every_gate_and_no_skip_passes --test a_decision_is_covered_by_a_sealed_spec`

Phase 2's test is red at `f4268d3`. It covers the spec format, `hexa spec check | accept | seal | verify`, and the
spec ratchet. Phase 1's test is carried in.
