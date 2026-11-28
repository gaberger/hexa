# ADR-2611221705: A layer key that matches no graded file is an error

**Status:** Completed
> 2026-11-22 Completed: landed on main at 687bc6a; gate `cargo test -p hexa-analysis --test adr_2611221705_spec` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no new or changed functionality), reported by a TypeScript user; he accepted the spec (re-accepted after corrections), and its sealed tests pass on main
**Date:** 2026-11-22
**Drivers:** Writing ADR-2611221645 S26, a fixture declared `analyze.layers` keys `src/domain/**` and `src/ui/**`. hexa reads a key as a path prefix, so neither matched a file: `src/ui/Button.tsx` went unclassified (only `src/domain/` has a built-in layer by its folder name), and the edge from the domain to it, a real violation, was not reported. Nothing said so: `config_errors` was empty and the grade held. a TypeScript user, which declares its portal's layers by path, asked for such a key to be reported; a declaration that silently matches nothing is a check that passes over an empty scope. CLAUDE.md: "A gate that degrades silently is worse than no gate."
**Supersedes:** none. A defect in how `hexa analyze` reads `analyze.layers`.

## Context

`LayerMap::from_project` (`hexa-analysis/src/layer_classifier.rs`) reads each `analyze.layers` key as a project-relative path prefix, matched on whole path segments; a key that names a source file (`src/store.rs`) also names its module (`src/store`). A layer name that is not a layer is already an error, "a declaration dropped silently would leave the grade claiming to cover code it does not check". A key that matches no file is dropped silently: it may be a glob (`src/ui/**`), a typo, a folder since moved, or a path `analyze.exclude` removes.

An `analyze.adapters` declaration that cannot be read is a config error (`ArchAnalysisResult::config_errors`), counted with the rule errors in the score (ADR-2610271649), so the mistake costs points instead of switching the check off.

## Decision

1. **An `analyze.layers` key that matches no file the grade reads is a config error.** After the grade collects its files, each key under which no graded file falls (by the same rule `LayerMap::classify` applies: the path itself or below it on a segment boundary, or the module a source-file key names) adds one entry to `config_errors` naming the key, e.g. `analyze.layers["src/ui/**"] matches no file the grade reads`. Each entry counts as a rule error in the score, as an `analyze.adapters` error does.
2. **A key holding a glob character says so.** When the key contains `*`, `?` or `[`, its error adds that keys are path prefixes, not globs, and names the prefix before the first glob character with its trailing `/` removed (for `src/ui/**`, `src/ui`).
3. **Nothing else changes.** A key that matches at least one graded file is read as before, the layers of every file are classified as before, and a project without `analyze.layers` grades as before.

## Consequences

- A project whose layer map has a stale, mistyped or glob key now loses points and is told which key, where it was graded as if the key did not exist.
- hexa's own 74 keys each match a graded file, so its grade is unchanged.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221705_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2610271649: an adapter importing another adapter in its own layer is a violation (`analyze.adapters`, config errors)
- ADR-2611221645: TypeScript path aliases are import edges (S26, where the gap was found)
- `hexa-analysis/src/domain.rs` (`ArchAnalysisResult::config_errors`), `hexa-analysis/src/ports.rs` (`ArchAnalysisPort`)
