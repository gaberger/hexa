# ADR-2611230850: `hexa init --scaffold` and `hexa scaffold` emit Python, as they emit Rust, Go and TypeScript

**Status:** Completed
> 2026-11-23 Amended (pre-PR review): §1 adds the package's `__init__.py`. Without it hexa does not count the package as the project's own (ADR-2611211324 §2), so an import between two of its modules read as an outside import.
> 2026-11-23 Completed: landed on main at fa33fce; gate `cargo test -p hexa-cli --test adr_2611230850_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: Python brought up to what the other languages have, under his rule; he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** ADR-2611211324 §9 left Python out of the scaffold: "`hexa scaffold` and `hexa init --scaffold` emitting Python … Each is a separate ADR or gap." Projects adopting hexa in Python build the ports, use cases and adapters layout by hand, where a Rust, Go or TypeScript project gets a runnable, A+-graded skeleton from one command. This ADR is that separate decision.
**Supersedes:** none. Decides what ADR-2611211324 §9 named as out of scope.

## Context

- `SCAFFOLD_LANGS` (`hexa-cli/src/commands/init.rs`) names each language the scaffold emits with its gate command: `rust` (`cargo test`), `go` (`go test ./...`), `ts` (`npm install && npm test`). `scaffold_files` writes every embedded asset under `hexa-cli/assets/scaffold/<lang>/`, rendering `{{name}}` and `{{name_snake}}` in each file's contents (not in its path), dropping a `.tmpl` suffix, and never overwriting a file.
- Each language's skeleton is the same counter: a domain value, a store port, an increment use case, an in-memory secondary adapter that imports only the port (the port re-exports the domain type it carries), a composition root that wires them, and one test the gate runs. The scaffold is deterministic: the same name gives the same bytes (`scaffold_is_executable.rs`).
- `gitignore_lines` (`hexa-core/src/domain/project_floor.rs`) gives a new project's `.gitignore` lines per language: `target/` for Rust, `node_modules/` and `dist/` for TypeScript.
- A layer is `HexLayer` in `hexa-analysis/src/domain.rs`; Python files are classified by their folder names and `composition_root.py` by its name (ADR-2611211324, ADR-2611230810).

## Decision

1. **`python` is a scaffold language.** `--lang python` emits the counter in Python: a `pyproject.toml` (`[project]` with the name, a version and `requires-python = ">=3.10"`), a package named `{{name_snake}}` holding `__init__.py`, `domain/count.py`, `ports/store.py` (which re-exports the domain's `Count`), `usecases/increment.py`, `adapters/secondary/memory.py` (which imports only the port) and `composition_root.py`, and `tests/__init__.py` with `tests/test_increment.py`, a unittest test of the composition root.
2. **Its gate is the standard library's unittest:** `python3 -m unittest discover -s tests -t .`. It needs no install and no network, and its output (`Ran 1 test … OK`) is read by the evidence reader (ADR-2611211324 §5).
3. **A scaffold asset's path is rendered as its contents are.** `{{name}}` and `{{name_snake}}` in an asset's path under `scaffold/<lang>/` are replaced the same way, so a Python package directory can carry the project's name. No existing asset's path holds either.
4. **A new Python project's `.gitignore` ignores `__pycache__/` and `.venv/`**, as a Rust project's ignores `target/`.
5. **The Python scaffold holds the scaffold's promises:** its gate passes immediately, `hexa analyze` grades it A+ with every file in a layer and no violation, it satisfies the domain policy it ships, and two runs with one name write the same bytes.
6. **Nothing else changes.** The Rust, Go and TypeScript scaffolds write the same bytes as before; `--lang` still defaults to `rust`; an unknown `--lang` is refused as before, naming `python` among the known ones.

## Consequences

- A Python project starts from the same graded, runnable skeleton as the other languages.
- The scaffold's tests that iterate every language now run the Python one too; the gate needs only `python3`.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611230850_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611211324: hexa reads Python (§5, §9)
- ADR-2611230810: Python's files are named where the other languages' are
- ADR-2610281552: a new project's first commit (§3, the ignore lines)
- `hexa-analysis/src/domain.rs` (`HexLayer`), `hexa-core/src/domain/project_floor.rs` (`gitignore_lines`)
