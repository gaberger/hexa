# ADR-2611221643: Go's exports are read whole, by every reader

**Status:** Completed
> 2026-11-22 Completed: landed on main at 0a5b049; gate `cargo test -p hexa-cli --test adr_2611221643_spec_2` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec, chose the fix under his rule (real bug fixes only; architectural correctness first), and its sealed tests, red before the code, pass on main
**Date:** 2026-11-22
**Drivers:** ADR-2611221609 and ADR-2611221623 fixed how the analyzer reads Go exports (grouped vars, every name of a var spec, Go's Unicode export rule) and left three readers behind, recorded as gaps: a const spec still records only its first name, the `@hexa:api` contract reader keeps an ASCII-only export rule, and the code graph still ignores grouped vars and second names. The owner's rule: architectural correctness first.
**Supersedes:** ADR-2611221623 §3 (a const spec records its first name).

## Context

- `extract_go_exports` (`hexa-analysis/src/treesitter_adapter.rs`) reads every name of a var spec but only the first of a const spec: in `const A, B = 1, 2`, `B` is not an export, so it is never graded, never dead, and cannot carry `@hexa:public`. ADR-2611221623 §3 decided that on purpose, to keep its change small.
- The `@hexa:api` contract reader (`hexa-analysis/src/treesitter_api.rs`) keeps a Go struct field only when its first letter is ASCII upper-case. Go exports a field whose first letter is any Unicode upper-case letter, so an exported field named `Ärger` is missing from the API contract.
- The code graph (`hexa-graph/src/extract/code.rs`, `collect_go_consts`) reads the specs that are direct children of a const or var declaration and takes each spec's first name. A var group's specs sit in a `var_spec_list`, so grouped vars are never graph entities, and neither is a spec's second name. `hexa graph consumers` then under-reports who uses them, while the analyzer grades them: the two disagree on what a Go export is.

The export record is `ExportDeclaration` in `hexa-analysis/src/domain.rs`.

## Decision

1. **Every name a const spec declares is an export**, each at the spec's line and of kind `Value`, grouped or not, as for a var spec. `const A, B = 1, 2` exports `A` and `B`. Only the identifiers are names: the commas between them are not.
2. **The API contract reader uses Go's export rule for struct fields**: a field is exported when its first character is a Unicode upper-case letter, as in the analyzer.
3. **The code graph reads Go's consts and vars as the analyzer does**: the specs of a var group (inside `var_spec_list`) and of a const group, and every name each spec declares, each an entity of kind `Const` at the spec's line. A Go file uses its package's names with no import, so a name one `.go` file uses is linked to the entity of that name another file of the same package declares; without that link `hexa graph consumers` saw no same-package user of any Go entity and called used code safe to remove (ADR-2609122048).

## Consequences

- An unused second name of a const spec is now a dead export, as the same name written alone already was.
- An API contract with an exported non-ASCII field now lists it.
- `hexa graph consumers` sees the users of grouped vars and of every name in a multi-name spec, so it no longer reports them safe to remove while the analyzer grades them.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611221643_spec_2`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611221609: Go's grouped vars are exports
- ADR-2611221623: a Go name is exported by Go's rule, and every name a var spec declares is one
- `hexa-analysis/src/domain.rs` (`ExportDeclaration`), `hexa-analysis/src/ports.rs` (`AstPort`)
