# ADR-2611221706: A `.test.tsx` file is a test, as a `.test.ts` file is

**Status:** Completed
> 2026-11-22 Completed: landed on main at b71ce4b; gate `cargo test -p hexa-analysis --test adr_2611221706_spec` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no new or changed functionality), reported by a TypeScript user; he accepted the spec (re-accepted after corrections), and its sealed tests pass on main
**Date:** 2026-11-22
**Drivers:** a TypeScript user, specifying its React portal's zero dead exports, found on a planted copy that an export used only by `Button.test.tsx` counts as dead, while the same export used only by `Button.test.ts`, or by a `.test.tsx` under `__tests__/`, does not. Nothing decides that `.tsx` tests differ; the two lists that name test files were written before the grade read `.tsx`, and name only the `.ts` forms.
**Supersedes:** none. A defect in how `hexa analyze` recognises TypeScript test files.

## Context

Two lists in `hexa-analysis/src/analyzer.rs` name test files by suffix:

- `EXCLUDE_PATTERNS`, what the grade leaves out, holds `.test.ts` and `.spec.ts` (and the folders `/tests/`, `/test/`, `/__tests__/`).
- `TEST_PATTERNS`, read by `is_test_file`, picks the files whose imports count as uses for the dead-export finder (`collect_test_files`): `.test.ts`, `.spec.ts`, `_test.go`, `.test.rs`, and any file under those folders.

The grade reads `.tsx` files. So `src/ui/Button.test.tsx` outside a test folder is graded as source: it takes a layer (or costs coverage), its own exports can be reported dead, and its imports are counted as ordinary source imports, not as test uses. An export only the test uses is then dead. The dead-export rule is `hexa-analysis/src/dead_export_finder.rs`.

## Decision

1. **A file whose name ends in `.test.tsx` or `.spec.tsx` is a test file**, wherever it is: the grade leaves it out, as it leaves out `.test.ts` and `.spec.ts`, and its imports count as uses for the dead-export finder, as a `.test.ts` file's do.
2. **Nothing else changes.** `.test.ts`, `.spec.ts`, the test folders and every other language's test files are read as before, and a project with no `.test.tsx` or `.spec.tsx` file grades as before.

## Consequences

- A React project whose tests sit beside their components is graded on its components only, and an export its tests alone use is no longer dead.
- A `.test.tsx` file no longer counts toward coverage or holds a layer, so a project's file count can fall.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221706_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2609241707: the grade is capped at the share of files with a layer (why `/test/` became test code)
- `hexa-analysis/src/domain.rs` (`ArchAnalysisResult`, `ExportDeclaration`), `hexa-analysis/src/ports.rs` (`ArchAnalysisPort`)
