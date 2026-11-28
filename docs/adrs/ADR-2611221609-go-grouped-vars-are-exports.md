# ADR-2611221609: Go's grouped vars are exports

**Status:** Completed
> 2026-11-22 Completed: landed on main at 607833c; gate `cargo test -p hexa-analysis --test adr_2611221609_spec_2` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec, chose the fix under his rule (no hexa core behaviour change except real bug fixes and new languages; architectural correctness first), and its sealed tests, red before the code, pass on main.
**Date:** 2026-11-22
**Drivers:** While building ADR-2611221553 (Go reads `@hexa:public`), two of its sealed scenarios failed for a reason unrelated to the tag: an exported var declared inside a `var ( … )` group is not an export to hexa at all. The grade, the dead-export finder and the tag never see it. The owner's rule for hexa work: architectural correctness comes first, even when a fix changes a grade.
**Supersedes:** none. A defect in how `hexa analyze` reads Go exports.

## Context

`extract_go_exports` (`hexa-analysis/src/treesitter_adapter.rs`) records the exports of a Go file: functions, methods, type specs, const specs and var specs. For `const ( … )` and `type ( … )` the specs are direct children of the declaration. For `var ( … )` they are not: tree-sitter-go puts them one level down, in a `var_spec_list`. The extractor looks only at direct children, so a grouped var is never recorded. `var X = 1` is an export; the same `X` written inside `var ( X = 1 )` is not.

An export the grade cannot see is a boundary the grade cannot check. Its dead-export finder cannot report it unused, and `@hexa:public` cannot mark it. The flag and the record are `ExportDeclaration` in `hexa-analysis/src/domain.rs`.

## Decision

1. **A grouped var is an export like a grouped const.** Each spec in a `var ( … )` group whose name Go exports (it starts with an upper-case letter) is recorded as an export of kind `Value`, at its spec's line, exactly as a spec in a `const ( … )` group is.
2. **Nothing else about Go exports changes** (except what ADR-2611221623 decides: Go's Unicode export rule, and every name of a var spec). An ungrouped var, a const group, a type group, functions and methods are read as before, and a grouped var whose name Go does not export is not an export.

## Consequences

- A Go project with grouped exported vars now has those vars in its export graph. One that nothing in the project uses is now a dead export and costs a point of the grade, as an ungrouped one already did. This is the grade becoming correct, not stricter: the same var, written ungrouped, was already counted.
- `@hexa:public` (ADR-2611221553) can now mark a grouped var.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221609_spec_2` (the 9 scenarios that fail before the fix; S05, S10, S12, S13, S14, S15 and S16 already hold and stay pending as guards)

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611221553: Go reads the `@hexa:public` tag
- `hexa-analysis/src/domain.rs` (`ExportDeclaration`, `ExportKind`), `hexa-analysis/src/ports.rs` (`AstPort`)
