# ADR-2611230810: Python's files are named where the other languages' are

**Status:** Completed
> 2026-11-23 Completed: landed on main at a49ef72; gate `cargo test -p hexa-analysis --test adr_2611230810_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: Python brought up to what the other languages have, under his rule; he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** Three Python projects adopting hexa found four places where a list names Rust, Go and TypeScript files and no Python file, so a Python project is read worse than the same project in another language: an entry-point module's exports count as dead, `__main__.py` has no layer where `main.go` has one, a layer key `app/store.py` does not cover the package `app/store/` where `app/store.rs` would, and the rules `hexa init` writes never run on `.py`. ADR-2611211324 added Python; these lists were not extended with it.
**Supersedes:** none. Completes ADR-2611211324 for the lists it did not reach.

## Context

- `ENTRY_POINT_SUFFIXES` (`hexa-analysis/src/dead_export_finder.rs`) names the files whose exports are never dead: `index.ts`, `cli.ts`, `main.ts`, `composition-root.{ts,go,rs}`, `composition_root.rs`, `main.go`, `main.rs`, `lib.rs`. No `.py` file.
- `match_filename` (`hexa-analysis/src/layer_classifier.rs`) gives `main.rs`, `main.go` and `main.ts` the entry-point layer (`HexLayer::EntryPoint`). A file named `composition_root.*` is already the composition root, `.py` included.
- A layer key that names a source file also names its module, so it covers the same-named directory: `key_covers` (`layer_classifier.rs`) and `under_key` (`hexa-analysis/src/domain.rs`, for `analyze.adapters`) strip `.rs`, `.ts`, `.tsx` or `.go`; the adapter name taken from a file under an adapter folder (`domain.rs`) strips the same four. Python's package `app/store/` is the module `app/store`, as `app/store.py` would be.
- The rules file `hexa init` writes (`hexa-cli/assets/templates/ADR-rules.toml`) holds three rules that read text any language can hold (a hardcoded absolute path, a hardcoded host:port, a model name outside the inference boundary), each with `file_patterns = [".rs", ".go", ".ts", ".tsx"]`.
- A layer is `HexLayer` in `hexa-analysis/src/domain.rs`.

## Decision

1. **A Python entry-point file's exports are never dead.** `__main__.py`, `main.py`, `cli.py` and `composition_root.py` join the entry-point files, as `main.ts`, `cli.ts` and `composition-root.ts` are, at the project root or under any directory.
2. **`__main__.py` and `main.py` take the entry-point layer**, as `main.go`, `main.rs` and `main.ts` do.
3. **A layer key naming a `.py` file also names its module.** `.py` is stripped with the other four suffixes wherever a key covers the same-named directory (`key_covers`, `under_key`) and wherever an adapter is named from its file, so `app/store.py` covers `app/store/__init__.py` as `app/store.rs` covers `app/store/x.rs`.
4. **The three language-neutral rules `hexa init` writes read `.py`.** `.py` joins `file_patterns` in `no-hardcoded-absolute-path`, `no-hardcoded-host-port` and `no-model-name-outside-inference`, with their `violation_patterns` and `exclude_patterns` unchanged.
5. **Nothing else changes.** Rust, Go and TypeScript files are read as before; `__init__.py` is not an entry point (a package's `__init__.py` is its module, not a program's start); a project whose rules file was written before this ADR keeps its own `file_patterns`.

## Consequences

- An entry-point module of a Python project stops counting as dead, and its `__main__.py` has a layer.
- A Python project's layer map can name a package by its file, as the other languages' can.
- A new Python project's absolute paths, host:port literals and model names are checked as another language's are.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611230810_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611211324: hexa reads Python
- ADR-2611221705: a layer key that matches nothing is an error (why §3 now shows)
- `hexa-analysis/src/domain.rs` (`HexLayer`), `hexa-analysis/src/ports.rs`
