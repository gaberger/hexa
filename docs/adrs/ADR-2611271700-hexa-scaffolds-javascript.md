# ADR-2611271700: `hexa init --scaffold` and `hexa scaffold` emit JavaScript, as they emit Rust, Go, TypeScript and Python

**Status:** Proposed
**Date:** 2026-11-27
**Drivers:** ADR-2611270420 brought JavaScript to the standard of the other languages in `hexa analyze` and `hexa graph` and left its scaffold undecided: "a JavaScript scaffold … is not decided here." Projects in JavaScript build the ports, use cases and adapters layout by hand, where a Rust, Go, TypeScript or Python project gets a runnable, A+-graded skeleton from one command. This ADR is that separate decision, on the pattern ADR-2611230850 set for Python. Craig's direction (2026-11-27): it is a new language on the existing pattern and changes neither the core behaviour nor the architecture.
**Supersedes:** none. Decides what ADR-2611270420 §8 named as out of scope.

## Context

- `SCAFFOLD_LANGS` (`hexa-cli/src/commands/init.rs`) names each language the scaffold emits with its gate command. `scaffold_files` writes every embedded asset under `hexa-cli/assets/scaffold/<lang>/`, rendering `{{name}}` and `{{name_snake}}` in each file's contents and path, dropping a `.tmpl` suffix, and never overwriting a file that exists.
- Each language's skeleton is the same counter: a domain value, a store port, an increment use case, an in-memory secondary adapter that imports only the port (the port re-exports the domain type it carries), a composition root that wires them, and one test the gate runs. The scaffold is deterministic (`scaffold_is_executable.rs`).
- The TypeScript scaffold needs `npm install` for the compiler. JavaScript needs no compiler: ES modules run as written, and Node ships a test runner, `node --test`, which hexa's evidence reader already reads (`hexa-core/src/domain/evidence.rs`, `KNOWN_RUNNERS`). A prototype of the skeleton below passes `node --test` (4 tests) with no install and grades A+ 100/100 under `hexa analyze`, every file in a layer, no dead export.
- `gitignore_lines` (`hexa-core/src/domain/project_floor.rs`) gives a new project's `.gitignore` lines per language.
- JavaScript has no interface. The port's contract is a JSDoc `@typedef` beside the domain type it re-exports; hexa's `unused ports` check reads TypeScript interfaces and has nothing to read here, which is why no port is reported unused.

## Decision

1. **`js` is a scaffold language.** `--lang js` emits the counter in JavaScript, in the layout the TypeScript scaffold uses: a `package.json` (the project's name, version `0.1.0`, `"type": "module"`, `engines.node` `>=22`, a `test` script, and no dependency of any kind), `src/core/domain/count.js`, `src/core/ports/counter-store.js` (which re-exports the domain's `Count` and documents the store's contract as a JSDoc `@typedef`), `src/core/usecases/increment.js`, `src/adapters/secondary/in-memory-counter-store.js` (which imports only the port), `src/composition-root.js` and `src/counter.test.js`. The adapter imports nothing from the domain or the use cases.
2. **Its gate is Node's own runner:** `node --test`. It needs no install and no network, and its output (`tests 4 … pass 4`) is read by the evidence reader. A run that executes no test is not a pass.
3. **Every import in the scaffold is a relative specifier with its `.js` extension, or a `node:` built-in.** ES modules require the extension; the scaffold names no package from a registry, so it has nothing to install.
4. **A new JavaScript project's `.gitignore` ignores `node_modules/`**, as TypeScript's does, and not `dist/`: nothing is compiled.
5. **The JavaScript scaffold holds the scaffold's promises:** its gate passes immediately, `hexa analyze` grades it A+ with every file in a layer, no violation and no dead export, it satisfies the domain policy it ships, and two runs with one name write the same bytes.
6. **Nothing else changes.** The Rust, Go, TypeScript and Python scaffolds write the same bytes as before; `--lang` still defaults to `rust`; an unknown `--lang` is refused as before, naming `js` among the known ones; an existing file is never overwritten.
7. **Not decided here:** the `Project:` header of `hexa analyze` names a `package.json` project `typescript` whether its sources are TypeScript or JavaScript. That label is read by `hexa doctor` and is a separate change.

## Consequences

- A JavaScript project starts from the same graded, runnable skeleton as the other languages, with nothing to install.
- The scaffold's tests that iterate every language now run the JavaScript one too; its gate needs only `node` (22 or later).
- The scaffold is JavaScript, not TypeScript with the types removed: a team that wants types uses `--lang ts`.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611271700_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611230850: hexa scaffolds Python (the pattern); ADR-2611270420: hexa grades JavaScript (§8 left this open)
- ADR-2610281552: a new project's first commit (§3, the ignore lines)
- `hexa-cli/src/commands/init.rs` (`SCAFFOLD_LANGS`), `hexa-core/src/domain/project_floor.rs` (`gitignore_lines`), `hexa-core/src/domain/evidence.rs`
