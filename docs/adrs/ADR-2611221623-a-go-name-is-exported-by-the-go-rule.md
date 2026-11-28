# ADR-2611221623: A Go name is exported by Go's rule, and every name a var spec declares is one

**Status:** Completed
> 2026-11-22 Completed: landed on main at 14fdf51; gate `cargo test -p hexa-analysis --test adr_2611221623_spec_2` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec, chose the fix under his rule (no hexa core behaviour change except real bug fixes and new languages; architectural correctness first), and its sealed tests, red before the code, pass on main.
**Date:** 2026-11-22
**Drivers:** Building ADR-2611221609 (Go's grouped vars are exports) fixed two more defects in how `hexa analyze` reads Go exports. A property scenario of that ADR drew the name `Ä` and found hexa does not treat it as exported, and a scenario with `var C, D = 2, 3` found hexa records `C` and not `D`. Both were fixed, but ADR-2611221609 decides only grouped vars, and says an ungrouped var is "read as before". This ADR records the two fixes as decisions, so the code and the record agree.
**Supersedes:** none. Its decision 3 is superseded by ADR-2611221643 §1. Amends what ADR-2611221609 §2 and ADR-2611221553 §3 say does not change.

## Context

`is_go_exported` (`hexa-analysis/src/treesitter_adapter.rs`) decides whether a Go name is an export. It asked whether the first character is an ASCII upper-case letter. The Go specification exports a name whose first character is a Unicode upper-case letter, so `Äpfel`, `Ωmega` and `Ärger` are exports in Go and were not in hexa.

A var spec may declare several names: `var A, B = 1, 2`. hexa took the first. `B` was not an export: not graded, never dead, and it could not carry `@hexa:public`. The record is `ExportDeclaration` in `hexa-analysis/src/domain.rs`.

## Decision

1. **A Go name is exported when its first character is a Unicode upper-case letter**, as the Go specification says, not only an ASCII one. This applies to every Go export hexa reads: functions, methods, types, consts and vars.
2. **Every name a var spec declares is an export of its own**, each at the spec's line and of kind `Value`, grouped or not. `var A, B = 1, 2` exports `A` and `B`.
**Superseded (was decision 3), by ADR-2611221643 §1:** a const spec recorded only its first name (`const A, B = 1, 2` exported `A`). ADR-2611221643 makes every name of a const spec an export, as for a var spec; this decision and its scenarios (S15-S18) were retired from the spec.

## Consequences

- A Go project that names an export with a non-ASCII capital, or declares several vars in one spec, now has those names in its export graph; an unused one is a dead export, as the same name written another way already was. This is the grade becoming correct.
- ADR-2611221609 §2 ("an ungrouped var … read as before") and ADR-2611221553 §3 ("a Go project that does not use the tag grades exactly as it does today") hold except for what this ADR decides.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221623_spec_2` (11 scenarios, red at 92ceac7, a commit off 4a958b8 before the fix; S03, S04, S05, S12, S15, S16 and S18 already held and stay pending as guards)

The code is in the tree (commit feedc03); its tests are proved red at the commit before it.

## References

- ADR-2611221609: Go's grouped vars are exports
- ADR-2611221553: Go reads the `@hexa:public` tag
- `hexa-analysis/src/domain.rs` (`ExportDeclaration`, `ExportKind`), `hexa-analysis/src/ports.rs` (`AstPort`)
