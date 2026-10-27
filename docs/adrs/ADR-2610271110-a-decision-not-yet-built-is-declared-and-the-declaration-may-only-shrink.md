---
id: ADR-2610271110
status: proposed
date: 2026-10-27
---
# ADR-2610271110: A decision not yet built is declared, and the declaration may only shrink

**Status:** Proposed
**Date:** 2026-10-27
**Drivers:** Forgejo CI on `main` has failed on every commit since `a1b6d85` (2026-10-27), and three pull requests were merged over it. `hexa adr gates --ci` fails on two gates whose code was never written. ADR-2610161730 (Accepted 2026-10-16) names `cargo test -p hexa-exec --test delegate_scope`, and no such test exists. ADR-2610221834 (Proposed) names `hexa spec verify ADR-2610221834`, which fails by design until it is built. Both failures are true. But a gate that is red for a known reason hides the next red one: nobody reads why CI failed once it always fails.

## Context

`run_gates` (`hexa-cli/src/commands/adr/mod.rs`) runs each ADR's recorded gate and fails `--ci` on any gate that fails, is unrunnable, cannot run here, or is not proven here. It has no notion of a decision that is approved but not built. ADR status does not supply one. About 40 gates pass today on ADRs marked Accepted, not Completed, so enforcing only Completed ADRs would stop enforcing them.

The ledger already has a pattern for debt that may only fall: legacy ADRs without a spec are counted, and the count is ratcheted (`LEGACY_CEILING`, ADR-2610221540 §9).

## Decision

1. **A project declares what it has not built.** `.hexa/project.json → adr.unbuilt` maps an ADR id to one line saying why it is not built yet. The types are `Unbuilt`, `Outcome` and `Verdict` in `hexa-core/src/domain/gate_ledger.rs`.
2. **A declared decision's failing gate is reported, not failed.** When a declared ADR's gate fails, is unrunnable, cannot run here or is not proven here, its verdict is `Unbuilt { reason }`. `hexa adr gates` prints it on its own line with the reason, counts it separately in the summary, and does not fail `--ci` for it. It is never counted as passed.
3. **A declared decision whose gate passes fails the run.** It is built, so the declaration is now false. The failure says to take it off `adr.unbuilt`. The list cannot hide a gate that works.
4. **The declaration is checked.** An id in `adr.unbuilt` that names no ADR, or names one that records no gate, fails the run. `declaration_errors` says which ids and why.
5. **Undeclared ADRs are judged as before.** For every ADR not in `adr.unbuilt`, `judge` gives the verdict `run_gates` gives today, `--ci` included. Declaring nothing changes nothing.
6. **hexa's own list may only shrink.** A ledger test pins hexa's `adr.unbuilt` at at most two entries, ADR-2610161730 and ADR-2610221834. A new entry fails it. Building one and removing it is the only way the list changes.
7. **The judgement is pure.** `judge` and `declaration_errors` in `hexa-core/src/domain/gate_ledger.rs` decide. `run_gates` runs the gates, reads the declaration and prints.

## Consequences

- `hexa adr gates --ci` passes on `main` once ADR-2610161730 and ADR-2610221834 are declared. Every other gate stays enforced.
- Each declared decision is visible on every run, with its reason, until it is built.
- `hexa spec verify --ci` is unchanged. ADR-2610221540's spec is accepted and sealed separately.

## Gate

`hexa spec verify ADR-2610271110`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2609132122: every ADR's recorded gate is run.
- ADR-2610221540 §9: the ledger is ratcheted, and a skip is not a pass.
- ADR-2610161730, ADR-2610221834: the two decisions declared.
