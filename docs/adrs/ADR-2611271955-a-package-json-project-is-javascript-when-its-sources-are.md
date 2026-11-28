# ADR-2611271955: a `package.json` project is JavaScript when its sources are

**Status:** Proposed
**Date:** 2026-11-27
**Drivers:** ADR-2611271700 left one thing undecided (§7): `hexa analyze` names a `package.json` project `typescript` whether its sources are TypeScript or JavaScript. A project scaffolded with `hexa init --scaffold --lang js` is told, by the first command it runs, that it is a TypeScript project. Four surfaces say so: the `language:` line of `hexa analyze`, the `type:` line of `hexa doctor`, the `Language:` line of the session-start fingerprint (two code paths), and the inventory table's language column. Craig, 2026-11-27: "we can do that."
**Supersedes:** none. Decides what ADR-2611271700 §7 named as undecided.

## Context

- `hexa analyze` (`hexa-cli/src/commands/analyze.rs`) sets the project's language from the manifest: a `package.json` is `typescript`; with a Python manifest beside it, `typescript` is listed only when the project has TypeScript sources (a `package.json` is often tooling only).
- `hexa doctor` (`hexa-cli/src/commands/doctor/mod.rs`) prints `typescript (package.json)` for any `package.json`. The session-start fingerprint reads the language in `detect_language_framework` (`hexa-analysis/src/fingerprint_extractor.rs`) and, as a fallback, in `manifest_language` (`hexa-cli/src/commands/hook/mod.rs`); both name `typescript`.
- ADR-2611270420 reads JavaScript with the TypeScript grammar and keeps `Language::TypeScript` for it. The layer inventory's language column is that `Language`, so it says `typescript` for a `.js` file. About forty places match on `Language::TypeScript` (the adapter, the import policy, the API reader); a separate `JavaScript` variant would touch each, and is not a change to a label.
- `graded_files_sync` (`hexa-analysis/src/analyzer.rs`) is the grade's own list of source files: it leaves out `node_modules`, build output, test files and `analyze.exclude`.

## Decision

1. **A `package.json` project's language is `javascript` when it has graded JavaScript source (`.js`, `.jsx`, `.mjs`, `.cjs`), no graded TypeScript source (`.ts` or `.tsx`, the TypeScript the grade reads) and no `tsconfig.json` at its root; otherwise it is `typescript`.** A `package.json` with no source, or with TypeScript among its sources, or a `tsconfig.json` and no source yet, is `typescript`, as it is today.
2. **The rule is one function, used by every surface that names the language:** the `language:` line of `hexa analyze`, the `type:` line of `hexa doctor`, and the `Language:` line of the session-start fingerprint, in both code paths that set it.
3. **Beside a Python manifest, the `package.json` language is listed only when the project has graded sources of it:** `typescript` when it has TypeScript, else `javascript` when it has JavaScript, as `typescript` alone is today. A `package.json` that is tooling only is not listed.
4. **"Graded source" is the grade's own list,** so a file in `node_modules`, in build output, a test file, or a path in `analyze.exclude` does not make a project JavaScript.
5. **The layer inventory's language column, and the `language` of its JSON rows, are unchanged.** They name the grammar that reads a file, and JavaScript is read by the TypeScript grammar (ADR-2611270420).
6. **Nothing else changes.** Grades, scores and findings are the same; the Rust, Go and Python labels are the same; no `Language` variant is added.

## Consequences

- A JavaScript project is told it is a JavaScript project, at once, in the three places a user reads it.
- The inventory table can still say `typescript` for a JavaScript file's row. The header names the project; the column names the grammar. That is stated here, not hidden.
- The session-start fingerprint now reads the source list once for a `package.json` project (no other language pays this); it is the grade's own walk, which skips `node_modules`.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611271955_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611271700 (§7, which this decides); ADR-2611270420 (JavaScript is read with the TypeScript grammar); ADR-2611251938 (a language is listed only where the project has it)
- `hexa-cli/src/commands/analyze.rs`, `hexa-cli/src/commands/doctor/mod.rs`, `hexa-cli/src/commands/hook/mod.rs` (`manifest_language`), `hexa-analysis/src/fingerprint_extractor.rs`, `hexa-analysis/src/analyzer.rs` (`graded_files_sync`)
