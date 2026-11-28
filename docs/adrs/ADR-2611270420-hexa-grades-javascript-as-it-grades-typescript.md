# ADR-2611270420: hexa grades JavaScript as it grades TypeScript

**Status:** Proposed
**Date:** 2026-11-27
**Drivers:** a TypeScript project's report that `hexa analyze` passes JavaScript it never grades (forward-snow, hexa 26.11.19), and Craig's rule that Rust, Go, Python and JavaScript are held to one standard. A scratch project reproduces it: a `domain/` file imports an adapter and `fs` in four spellings, `.ts`, `.js` with ES `import`, `.js` with `require`, and `.mjs`. Only the `.ts` file counts in the grade (`Analysed 2 files`, `coverage 2/2`, one violation, grade B 89). The import scan before it warns about the `.js` ES import and does not see `.mjs` or the `require`; neither reaches the grade.
**Supersedes:** none. It extends how `hexa analyze` reads source files, on the ground ADR-2611211324 did for Python.

## Context

`Language::from_path` (`hexa-analysis/src/domain.rs`) already maps `.js` and `.jsx` to `Language::TypeScript`, but `SOURCE_EXTENSIONS` in `hexa-analysis/src/analyzer.rs` names `ts`, `tsx`, `go`, `rs`, `py` and `pyi`, so a JavaScript file is scanned for imports and then left out of the graded analysis: no layer, no coverage, no cycle, no dead export. `.mjs` and `.cjs` are in neither. The TypeScript adapter (`hexa-analysis/src/treesitter_adapter.rs`) reads `import`/`export … from`, dynamic `import()` and `import x = require()`, and not a CommonJS `require("x")` call, which is how most Node JavaScript imports. A report that says nothing about the files it did not read reads as clean, which is the failure ADR-2611081151 names for gates.

## Decision

1. **`.js`, `.jsx`, `.mjs` and `.cjs` are graded source.** They are in `SOURCE_EXTENSIONS` and in the import scan, and `hexa analyze` reads them with the TypeScript adapter (JavaScript's syntax is a subset of what its grammar reads), so each has a layer, counts toward coverage, and takes part in boundary checks, cycles, dead exports and orphans exactly as a `.ts` file does. The same folders are skipped as for TypeScript (`node_modules`, build output), and `analyze.exclude` removes a path from the grade as it does for every language.
2. **A CommonJS `require("x")` with a string-literal argument is an import edge**, in JavaScript and TypeScript files alike. A `require` whose argument is not a string literal makes no edge and is not guessed at.
3. **A relative specifier resolves to the project file it names**, trying `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs` and a folder's `index` file with each, and a `.js` specifier resolves to the `.ts` file the TypeScript compiler would map it to when that file exists, else to the `.js` file. A `jsconfig.json` `paths` map is read as a `tsconfig.json` one is (ADR-2611221645).
4. **CommonJS exports are exports.** `module.exports = { a, b }`, `module.exports.a = …` and `exports.a = …` export `a` and `b` for dead-export detection; `module.exports = f` exports the module's default. An ES `export` is read as in TypeScript.
5. **JavaScript test files and entry points follow TypeScript's.** `*.test.*` and `*.spec.*` in the four extensions and anything under `__tests__/` are test files; `index`, `cli`, `main` and `composition-root` in the four extensions are entry points.
6. **The domain's I/O deny list and `@hexa:public` apply to JavaScript.** An `fs`, `node:*` or `child_process` import or `require` in `domain/` is a policy violation under the same entries as TypeScript's (ADR-2609211430), and `@hexa:public` in a JavaScript file keeps its export from the dead-export check (ADR-2611221553).
7. **No JavaScript file is read as clean without being read.** `hexa analyze . --file <path.js>` checks that file's imports, and a JavaScript file left out of the grade by `analyze.exclude` is counted as excluded, as for every language.
8. **Not decided here:** a JavaScript scaffold (`hexa init --scaffold --lang javascript`), and the checks that need types (ports, unused ports, the API contract), which JavaScript has nothing to attach to.

## Consequences

- A project with JavaScript files is graded on them, so its grade can fall: violations, cycles and dead exports that were there and unseen are now reported. `analyze.exclude` keeps a path out, and nothing is graded that the project did not already contain.
- A JavaScript-only project that had no gradable files is now graded, not "not graded: no source files".
- TypeScript files that `require()` gain edges and can change likewise.
- A JavaScript file with a dynamic `require(name)` still has edges it hides; that is said here, not guessed.

## Implementation

**Gate:** to be written with the spec: `cargo test -p hexa-cli --test adr_2611270420_spec`. Build starts only after `hexa spec draft`, `hexa spec check`, a person's `hexa spec accept` and sealed failing tests (ADR-2609121400).

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2609211430 (the domain's I/O deny list), ADR-2611081151 (a check that cannot read its evidence fails), ADR-2611211324 (Python read as the others), ADR-2611221553 (`@hexa:public`), ADR-2611221645 (TypeScript path aliases are import edges)
- `hexa-analysis/src/analyzer.rs` (`SOURCE_EXTENSIONS`), `hexa-analysis/src/domain.rs` (`Language`), `hexa-analysis/src/treesitter_adapter.rs`, `hexa-analysis/src/path_normalizer.rs`, `hexa-analysis/src/dead_export_finder.rs`
