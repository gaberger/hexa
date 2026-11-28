# ADR-2611240905: A guard scenario is proven green at its recorded commit, not red

**Status:** Proposed
**Date:** 2026-11-24
**Drivers:** Peer repositories (a Go SDK) report `hexa spec verify --ci` failing only on pending scenarios: one ADR there has 5 pending and another 31, each a test that exists, is sealed and passes, guarding behaviour that was already true. The same gap is recorded as `guard-test-is-a-mirror`, and its guide names the missing verb: a `guard` scenario kind that `--prove` requires green at the red commit. The same shape is pending in this repository (14 on ADR-2611221933, 2 on ADR-2611230850, 3 on ADR-2611231735).
**Supersedes:** none. Amends ADR-2611161600 (§3 and the guard consequence) and ADR-2610301500 (the `guard-test-is-a-mirror` row).

## Context

The gate's rule is that every test fails before the code (ADR-2609121400): that is what makes a test an independent oracle and not a mirror. A scenario that guards behaviour already in place cannot satisfy it, so ADR-2611161600 §3 sets such tests aside and leaves the scenario pending. The only retirement it names is to make the test also exercise the new behaviour, so it fails first. That is impossible after the fact: the red commit must be an ancestor of HEAD and holds the test as it was, and history is not rewritten.

The cost: `spec verify --ci` and `hexa loop stage done` treat pending as unproven, so a spec that is fully built and fully tested cannot finish. "Pending" currently conflates two different things: a scenario nobody has proven, and a guard whose proof is that it holds before and after.

## Decision

1. **A scenario has a kind, `feature` (the default) or `guard`,** in `hexa-core/src/domain/spec.rs`. A spec file without the field reads as `feature`, so every accepted spec keeps its meaning.
2. **A guard is declared in the spec, so a person accepts it.** `hexa spec draft` and `hexa spec accept` carry the kind; changing a scenario's kind is a scenario change and drops that scenario's acceptance like any other (ADR-2610281628 §1). An agent never sets a kind on an accepted spec.
3. **A guard is bound with `hexa spec red --guard <scenario>=<file>::<test>`,** which records the commit and refuses unless the test is **green** at that commit and green at HEAD. A `feature` scenario is still refused when green at its red commit.
4. **`spec verify --prove` accepts a bound guard green at its commit** and still rejects a `feature` scenario green at its red commit. A guard whose test is red at HEAD fails like any failing scenario.
5. **`spec verify --ci` and `hexa loop stage done` count a bound guard as proven, not pending.** An unbound guard stays pending. The verify report lists guards apart from features, so the count of scenarios proven by failing first is never inflated by guards.
6. **A bound guard's test is sealed like any sealed test,** and the gap `guard-test-is-a-mirror` is retired once this ships (its guide's `fix` line is removed with it).

## Consequences

- A fully built spec can finish under `--ci` while its guards are visibly a separate class of proof.
- The mirror check keeps its meaning for features: only a person-accepted guard kind exempts a green-at-red test.
- Risk: a feature mislabelled as a guard escapes the red-first rule. The person's acceptance of the kind (§2) and the separate report (§5) are the defences; the scenario is still graded green at HEAD.
- Existing specs and sealed tests are untouched; this repository's pending guards can be bound after acceptance.

## Implementation

**Gate:** to be written with the spec: `cargo test -p hexa-cli --test adr_2611240905_spec`. Build starts only after `hexa spec draft`, `hexa spec check`, a person's `hexa spec accept` and sealed failing tests (ADR-2609121400).

## References

- ADR-2609121400, ADR-2610221540, ADR-2610281628, ADR-2610301500, ADR-2611161600.
- Gap `guard-test-is-a-mirror`; `hexa-core/src/domain/guide.rs`; `hexa-cli/src/commands/spec_ledger.rs`.
