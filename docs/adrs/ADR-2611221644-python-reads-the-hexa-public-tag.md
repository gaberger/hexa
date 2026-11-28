# ADR-2611221644: Python reads the `@hexa:public` tag

**Status:** Completed
> 2026-11-22 Completed: landed on main at 44ace7b; gate `cargo test -p hexa-analysis --test adr_2611221644_spec_2` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec, chose the fix under his rule (real bug fixes only; architectural correctness first), and its sealed tests, red before the code, pass on main
**Date:** 2026-11-22
**Drivers:** a Python SDK, a published Python library, graded B 80 after its conversion: all 24 dead exports were names in its modules' `__all__`, the public API that users outside the repository import. hexa's answer for a library's public API is the `@hexa:public` tag, and in Python it does nothing: `extract_python_exports` always writes `hexa_public: false`. The project had to add a test that imports each public name to make the grade count them as used.
**Supersedes:** none. Completes the Python support of ADR-2611211324 the way ADR-2611221553 completed Go's.

## Context

ADR-2610092245 builds on `@hexa:public` as a tag the tree-sitter adapter reads from the comment before a declaration, and the dead-export finder never reports a tagged export (`hexa-analysis/src/dead_export_finder.rs`). TypeScript, Rust and now Go read it. Python does not.

A Python module's exports (`extract_python_exports` in `hexa-analysis/src/treesitter_adapter.rs`) are its top-level `def`s, `class`es and assignments whose name does not start with `_`, or, when the module assigns `__all__`, exactly the names `__all__` lists. A Python comment is a `#` line; a decorated function's comment sits above its first decorator.

Treating every name in `__all__` as public would be wrong for a module that is internal to the project: `__all__` there limits `import *`, it does not promise an outside caller, and its unused names are dead code the grade should report. The tag says which names have outside callers.

## Decision

1. **A Python export is public when its comment carries the tag.** A top-level `def`, `class` or assignment that is an export is `hexa_public` when any line of the run of `#` comment lines directly above it contains `@hexa:public`. For a decorated `def` or `class`, the run is the one directly above its first decorator. A blank line ends the run, and a `#` comment that ends a line holding code belongs to that line, not to what follows.
2. **A tag above `__all__` marks every name it lists.** When the run of `#` lines directly above the `__all__` assignment carries the tag, every export `__all__` lists is `hexa_public`.
3. **Nothing else changes.** A Python export with no tag is graded as before, and TypeScript, Rust and Go read the tag as before.

## Consequences

- A Python library marks its public API once, above `__all__`, or per definition, and the grade stops counting it as dead without a test that exists only to import it.
- An internal module's `__all__` stays graded: its unused names are still reported.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221644_spec_2`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611211324: hexa reads Python
- ADR-2611221553: Go reads the `@hexa:public` tag
- `hexa-analysis/src/domain.rs` (`ExportDeclaration::hexa_public`), `hexa-analysis/src/ports.rs` (`AstPort`)
