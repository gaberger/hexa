# ADR-2611221934: A Go line of comments is read whole, when finding a doc comment

**Status:** Completed
> 2026-11-22 Completed: landed on main at 93b2bbd; gate `cargo test -p hexa-analysis --test adr_2611221934_spec` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no new or changed functionality); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-22
**Drivers:** ADR-2611221553 §1 says a Go export is `hexa_public` when its doc comment carries the tag, and that "a comment that ends a line holding other code belongs to that line". `go_doc_tagged` (`hexa-analysis/src/treesitter_adapter.rs`) decides whether a comment ends a code line by looking at the one sibling before it, so it misreads a line that holds two comments: `A = 1 /* x */ // @hexa:public` followed by `B = 2` tags `B` (both comments are `A`'s line), and `/* @hexa:public */ // note` on the line above `func F()` does not tag `F` (the whole line is `F`'s doc). Found reviewing ADR-2611221643.
**Supersedes:** none. A defect in how ADR-2611221553 §1 is read.

## Context

`go_doc_tagged` walks back from a declaration over its preceding comment siblings, each ending on the line just above the next. It stops (no tag) at a comment whose previous sibling is a non-comment ending on the comment's line, since such a comment is that code's line comment. When the previous sibling is another comment on the same line, it does not look further back, so a second comment after code is read as a doc line; and when a line holds two comments, the walk requires the earlier one to end on the line above the later one, which it does not, so the run stops there.

The export record is `ExportDeclaration` in `hexa-analysis/src/domain.rs`, produced through `AstPort` in `hexa-analysis/src/ports.rs`.

## Decision

1. **A comment belongs to a code line when any code precedes it on its line**, whatever comments sit between: in `A = 1 /* x */ // @hexa:public`, both comments are `A`'s line comments, and neither is part of the doc comment of what follows.
2. **A line that holds only comments is one line of a doc comment.** Every comment on such a line is part of the run, and the line counts once when finding the line above it: `/* @hexa:public */ // note` directly above `func F()` is `F`'s doc comment, and tags `F`.
3. **Nothing else changes.** A doc comment of one comment per line, a blank line ending the run, a group's doc covering its specs (ADR-2611221553 §2), and every other language read the tag as before.

## Consequences

- A Go export is tagged by the doc comment Go itself would give it, in the two shapes above, as in every other.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221934_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611221553: Go reads the `@hexa:public` tag
- `hexa-analysis/src/domain.rs` (`ExportDeclaration`), `hexa-analysis/src/ports.rs` (`AstPort`)
