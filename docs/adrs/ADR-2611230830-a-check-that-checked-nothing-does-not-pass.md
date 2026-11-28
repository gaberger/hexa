# ADR-2611230830: A `--ci` check that checked nothing does not pass

**Status:** Completed
> 2026-11-23 Completed: landed on main at 18392dd; gate `cargo test -p hexa-cli --test adr_2611230830_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no architectural change); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** A project designing its release gate around `hexa adr gates --ci` and `hexa spec verify --ci` asked what either does when there is nothing to check. Both exit 0: in a repository whose `docs/adrs/` holds no ADR, `hexa adr gates --ci` prints `0 passed · 0 failed · …` and `hexa spec verify --ci` prints `0 passed · 0 failed · 0 not proven here · 0 pending`. A release job gated on them passes having run nothing. CLAUDE.md: "A vacuous gate is a failed gate. `evidence_is_vacuous` rejects 'running 0 tests'; every new gate shape needs the same guard." These two never got it.
**Supersedes:** none. Applies the vacuous-gate rule to the two `--ci` runs that lacked it.

## Context

- `run_gates` (`hexa-cli/src/commands/adr/mod.rs`) runs every ADR's recorded gate and prints a summary of passed, failed, unrunnable, cannot-run-here, not-proven-here, unbuilt (declared) and without-a-gate counts. With `--ci`, it fails on a failed or skipped case (ADR-2610221540 §8) and on what ADR-2610271110 decides for declared ADRs.
- `verify` (`hexa-cli/src/commands/spec_ledger.rs`) runs every sealed scenario's test and prints passed, failed, not proven here and pending. With `--ci`, it fails on a case this machine could not run, and on a pending scenario of a Completed ADR.
- The verdict types are in `hexa-core/src/domain/gate_ledger.rs` (`Verdict`, `Outcome`) and `hexa-core/src/domain/spec.rs`.

## Decision

1. **`hexa adr gates --ci` fails when no gate ran.** When the run judged no ADR's gate (no gate passed, failed or was declared unbuilt; every ADR was without a gate, or there was no ADR), `--ci` exits non-zero and says that no ADR records a gate, so nothing was checked.
2. **`hexa spec verify --ci` fails when no scenario was checked.** When the run found no scenario at all (no spec, or specs with no scenarios), `--ci` exits non-zero and says that no spec holds a scenario, so nothing was checked. A run with pending scenarios is not this case: it checked them and reports them as it does today.
3. **Nothing else changes.** Without `--ci`, both print what they print today and exit as they do today; a `--ci` run that checked at least one gate or one scenario is judged as before; `hexa spec verify <ADR>` for one ADR is judged as before.

## Consequences

- A release job gated on `--ci` fails, with a reason, in a repository that records no gate or no spec, instead of passing over an empty scope.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611230830_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2610221540: the spec is a gate (§8, `--ci`)
- ADR-2610271110: a decision not yet built is declared
- `hexa-core/src/domain/gate_ledger.rs` (`Verdict`, `Outcome`), `hexa-core/src/domain/spec.rs`
