# ADR-2611221553: Go reads the `@hexa:public` tag

**Status:** Completed
> 2026-11-22 Completed: landed on main at a59e696; gate `cargo test -p hexa-analysis --test adr_2611221553_spec_2` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec, chose the fix under his rule (no hexa core behaviour change except real bug fixes and new languages; architectural correctness first), and its sealed tests, red before the code, pass on main.
**Date:** 2026-11-22
**Drivers:** a Go SDK, a published Go library being split into the hexagonal layout, has about 230 exports with no caller inside the module because they are its public API: about 219 domain builders, constants and helpers, and 12 use-case methods. The dead-export finder counts each one, the deduction caps at 20, and the grade cannot pass B (80). hexa's answer for a library's public API is the `@hexa:public` tag, and in Go it does nothing.
**Supersedes:** none. A defect against ADR-2610092245, which builds on the tag as "a comment tag [that] works the same way in Rust, Go and TypeScript".

## Context

The tree-sitter adapter marks an export public when the comment before it contains `@hexa:public` (`has_hex_public_annotation` in `hexa-analysis/src/treesitter_adapter.rs`), and the dead-export finder never reports a public export (`hexa-analysis/src/dead_export_finder.rs`). The flag is `ExportDeclaration::hexa_public` in `hexa-analysis/src/domain.rs`.

TypeScript and Rust exports read it. Go exports do not: `extract_go_exports` writes `hexa_public: false` at all four places it records an export (functions, methods, type specs, const and var specs), and never calls the check. ADR-2610092245 says the tag works the same way in Go. It does not.

A Go doc comment is the run of `//` lines directly above a declaration, with no blank line between them and it. Each line is its own comment node in the tree, and the tag may be on any of them. For a declaration inside a group (`type ( … )`, `const ( … )`, `var ( … )`), the comment that documents the whole group sits above the group; a comment that documents one spec sits above that spec, inside the group.

## Decision

1. **A Go export is public when its doc comment carries the tag.** A function, a method, a type spec, or a const or var spec that Go exports (a name starting with an upper-case letter) is `hexa_public` when any line of the run of comment lines (`//` or `/* … */`, as a Go doc comment may be either) directly above its declaration contains `@hexa:public`. A comment that ends a line holding other code is that line's comment, not a doc line. The run ends at the first line that is not a comment, or at a blank line: a comment separated from the declaration by a blank line is not its doc comment.
2. **In a group, the group's doc comment covers every spec in it.** A spec inside `type ( … )`, `const ( … )` or `var ( … )` is `hexa_public` when the tag is in its own doc comment (the run directly above the spec, inside the group) or in the doc comment of the group (the run directly above `type`, `const` or `var`).
3. **Nothing else changes** (except what ADR-2611221609 and ADR-2611221623, which fix how Go exports are read, decide). An export with no tag is graded as before, so a Go project that does not use the tag grades exactly as it does today. TypeScript and Rust read the tag exactly as they do today.

## Consequences

- A Go library can mark its public API the way a TypeScript or Rust one can, and the grade stops counting it as dead.
- A tag on a group marks every spec in it. A whole `const ( … )` block of exported constants takes one tag.
- The tag still means "reached from outside this repository": a tagged export that is truly unused is not reported.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221553_spec_2` (the 15 scenarios that fail before the code; the 8 that already hold, S06, S08, S09, S15, S19, S20, S21, S23, are guards and stay pending)

| Phase | Work | Decisions |
|---|---|---|
| 1 | `extract_go_exports` reads the doc-comment run for functions, methods, type, const and var specs, and the group's doc comment | 1, 2 |
| 2 | Untagged Go, TypeScript and Rust unchanged | 3 |

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2610092245: the API contract is a tagged driving port (the `@hexa:public` tag)
- `hexa-analysis/src/domain.rs` (`ExportDeclaration::hexa_public`), `hexa-analysis/src/ports.rs` (`AstPort`)
