# ADR-2611211324: hexa reads Python: the grade, the evidence and the test runner

**Status:** Completed
> 2026-11-21 Completed: landed on main at f8c1df5; gate `cargo test -p hexa-cli --test adr_2611211324_spec` passed (hexa loop stage done)
> 2026-11-21 Accepted: Accepted under craigjohnson's instruction in this session: the spec was accepted by craigjohnson, the 45 sealed scenario tests were red at 3b2a9ef and pass at 731c8a1, two pre-landing reviews' findings are fixed (e6ec56f), and hexa grades itself A+ 100/100 with 224/224 files in a layer.
**Date:** 2026-11-21
**Drivers:** Two Python projects are mandated to use hexa and cannot: `netbox-plugin` (a NetBox plugin) and `nautobot-ssot-plugin` (a Nautobot app). `hexa analyze` on a Python tree prints "language: unsupported (Python; hexa grades Rust, Go and TypeScript)" (`hexa-cli/src/commands/analyze.rs`), and `examples/stress/game-wordle-py` is kept as "a tree it cannot read".
**Supersedes:** none. Extends ADR-2611191120 (a Python test is already runnable through `test.runners.py`; this ADR makes it a built-in) and ADR-2611081151 §5 (the runners the evidence reader knows).

## Context

hexa's two gates are a command that exits 0 on counted tests, and an architecture grade. For Python both are blind or refuse:

1. **The grade does not parse Python.** `Language` in `hexa-analysis/src/domain.rs` and in `hexa-graph/src/extract/code.rs` has TypeScript, Go and Rust. A `.py` file is `Unknown`: no imports, no exports, no cycles, no boundary check. `collect_source_files` lists `.py` files, so the layer inventory counts them, but nothing grades their imports. A grade for them would be a claim about files that were not read (ADR-2609140020).
2. **The evidence reader does not know pytest.** `read_evidence` in `hexa-core/src/domain/evidence.rs` reads cargo, go, node, bun, vitest and one bash script. pytest's `== 5 passed in 0.12s ==` and unittest's `Ran 5 tests ... OK` are "unrecognised", which fails a gate unless it is declared `--uncounted`: the exit-status-alone pass the reader exists to refuse.
3. **A Python test has no built-in runner.** `runner_for` in `hexa-core/src/domain/test_runner.rs` returns `None` for `py`. A project can configure one (ADR-2611191120 §2), but then a scenario needs hand-written config per project, and the single-test rule (exactly one test ran and passed) is the project's to get right.

A pilot of hexa 26.11.15 on `netbox-plugin` (Django, with a `package.json` that exists only for a browser test tool) found three more defects: `hexa analyze .` scanned 0 files and still printed "A+ 100/100" with a green exit; the language was read as TypeScript from `package.json`; and a Django layout (`models.py`, `views.py`, `migrations/`, `management/commands/`) has nothing the grade can classify by folder name.

Python's layout differs from the three languages in ways the grade must handle: imports name modules, not paths (`from nautobot_ssot.ports import x`); relative imports are common (`from ..ports import x`); there is no export keyword; and packages are rarely under `src/` (both target projects keep their package at the repository root). Both targets are also framework apps (Django models, Nautobot jobs) where a layer is declared, not guessed, so `analyze.layers` is the normal way in.

## Decision

1. **`Language::Python` is a language in both enums**, detected by extension `.py` and `.pyi`, parsed with tree-sitter (`tree-sitter-python`). The grammar is a new dependency of `hexa-analysis` and `hexa-graph`, in `Cargo.toml` only; no other crate names it. The sites to extend are the ones the TypeScript, Go and Rust variants already have in `hexa-analysis/src/domain.rs`, `treesitter_adapter.rs`, `treesitter_api.rs`, `import_policy.rs`, `path_normalizer.rs`, and `hexa-graph/src/extract/code.rs`.
2. **Imports resolve to project files.** `import a.b`, `from a.b import c` and relative `from . import x` / `from ..ports import y` become an `ImportStatement` whose `resolved_path` is `a/b.py` or `a/b/__init__.py`, searched under the project root and each package root (`src/`, and each top-level directory holding an `__init__.py`). An import that resolves to no project file (stdlib, third-party) is not an edge, and is never a violation. `from a import b` resolves to `a/b.py` when that exists, else to `a/__init__.py`. An import inside `if TYPE_CHECKING:` is an edge, because the layering rule is about knowledge, not runtime.
3. **A Python export is a top-level public name.** A module's exports are its `__all__` when it assigns one, else every top-level `def`, `class` and assignment whose name does not start with `_`. A name re-exported by `from .x import y` in an `__init__.py` counts as consumed. Dead-export detection then runs on Python by the same rule as the other three.
4. **`hexa analyze` reads a Python project.** A `pyproject.toml`, `setup.py`, `manage.py` or `requirements.txt` makes the language `python`, and it is not displaced by a `package.json` that is the project's only JavaScript tooling: where several manifests are present, `hexa analyze` names every language it found and grades each one's files, never picking one by manifest order alone. `migrations/`, `.venv/`, `venv/`, `site-packages/`, `node_modules/` and `__pycache__/` are excluded from the grade by default; `tests/` and `test_*.py` files are test code, excluded from the layer count the way hexa already excludes test code by folder (`test_code_is_excluded_by_folder_not_by_substring`). The layer scan covers the package directories, not only `src/`. A file outside the folder convention (`models.py`, `jobs.py`, `views.py`) needs an `analyze.layers` entry, or it costs the grade, as for the other languages. The "unsupported" line is removed.
5. **The evidence reader counts pytest and unittest.** `read_evidence` reads pytest's summary (`== 3 failed, 5 passed, 1 skipped in 0.31s ==`, and the `-q` form `5 passed in 0.12s`) and unittest's (`Ran 5 tests in 0.002s` followed by `OK` or `FAILED (failures=1)`). `no tests ran` and `collected 0 items` are vacuous. A pytest run that reports only `skipped` or `xfailed` passed zero tests, so it is vacuous. `KNOWN_RUNNERS` names pytest and unittest.
6. **A grade over no parsed source is not a grade, in any language.** When `hexa analyze` parses zero files it prints "not graded", and exits non-zero under `--exit-code` and `--grade`; it never prints a letter. The pilot's 0 files graded A+ 100/100 is the failure this refuses.
7. **unittest and Django counts.** `Ran N tests` with `OK` counts N minus the skipped as passed; `OK (skipped=33)` reads the 33 as skipped, not passed; `Ran 0 tests` is vacuous; `FAILED (failures=a, errors=b)` fails. A run through `invoke` or `nautobot-server test` is read by its output, as every runner is.
8. **`py` has a built-in runner.** `runner_for("py", …)` returns a Python runner: `<path>.py::<test name>` runs `python3 -m pytest <file>::<name> -q` from the nearest directory at or above the file holding a `pyproject.toml`, `pytest.ini`, `tox.ini` or `setup.cfg`. It passes only if exactly one test ran and passed (zero is vacuous, more than one is ambiguous), by the rule of ADR-2611191120 §2. A `test.runners.py` entry wins over it, which is how a framework runner such as `nautobot-server test` or `invoke unittest` is named. A file under `tests/` or named `test_*.py` or `*_test.py` is a test file that may be sealed (`is_test_file`).
9. **Out of scope, stated so it is not mistaken for done:** `hexa spec tests` generating Python (unittest, pytest, Django `TestCase`) tests, so a Python scenario is bound to a test written first and seen to fail (`hexa spec red`), as for vitest; `hexa scaffold` and `hexa init --scaffold` emitting Python; `hexa init` saying in its output that `lifecycle_enforcement` defaults to "mandatory" and what that blocks. Each is a separate ADR or gap.
10. **The Python example is graded.** `examples/stress/game-wordle-py` is graded A+ by `hexa analyze`, or its violations are the planted ones, and its README is corrected to say so. The same run against each of the two target repositories is recorded in this ADR's **Built:** line: grade, files with a layer, and the `analyze.layers` each needed.

## Consequences

**Good.** A Python project gets the same two gates as the others: counted tests, and a grade that has read its imports. The mandate stops being unmeetable for `netbox-plugin` and `nautobot-ssot-plugin`.

**Bad.** Python's dynamic imports (`importlib.import_module`, Django's string-named apps and settings, Nautobot's `register_jobs`) are invisible to a static reader. The grade can pass a project whose real coupling runs through them. Dead-export detection will over-report on framework entry points (models, jobs, signals, management commands) that are reached by name from configuration; those need `analyze.layers` or an entry-point declaration, and the first run on each target will say how many. A new native grammar dependency adds build time.

**Mitigation.** Decision 10 measures it on the two real projects before this is called built, and a false dead-export on a framework entry point is a defect to fix, not a list to add to.

## Implementation

| Phase | Work | Gate |
|---|---|---|
| 1 | `Language::Python` and the grammar; imports and exports (decisions 1 to 3) | sealed per-language tests, as `circular_deps_per_language` and `dead_exports_per_language` do for the three |
| 2 | `analyze` reads Python; layer scan over package dirs (decision 4) | `hexa analyze examples/stress/game-wordle-py` |
| 3 | Evidence reader, the empty grade, and the `py` runner (decisions 5 to 8) | sealed tests over pytest and unittest transcripts |
| 4 | The two target projects (decision 10) | `hexa analyze` on each, recorded; `hexa analyze .` on hexa stays A+, 100/100 |

Every phase ends with `cargo check --workspace`, `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611191120: a test names its runner (`test.runners`, the single-test rule)
- ADR-2611081151: the evidence reader
- ADR-2609140020: a grade is a claim about files that were read
- `hexa-core/src/domain/test_runner.rs`, `hexa-core/src/domain/evidence.rs`, `hexa-analysis/src/domain.rs`

**Gate:** `cargo test -p hexa-cli --test adr_2611211324_spec && cargo test -p hexa-core --lib domain::evidence`

**Built:** phases 1 to 4 on 2026-11-21, at bdf80f1 on the local branch `feat/python` (not upstream). The 45 sealed scenario tests in `hexa-cli/tests/adr_2611211324_spec.rs` were red at 3b2a9ef and pass; S46 is held by the CI grade step. One fixture amendment is recorded in the spec (S05: one edge per target). hexa grades itself A+ 100/100, 224 of 224 files in a layer: five dead exports already on main (`compress.rs`, `direct_exec.rs`, `wiring.rs`) were cleared, and the two `.py` build scripts (`scripts/build-diagrams.py`, `scripts/diagram_lib.py`) are named in `analyze.exclude`. Two reviews before landing found, and e6ec56f fixed with a test each: `hexa graph consumers` printed SAFE TO REMOVE for an imported Python module (the graph did not resolve Python imports); the pytest runner counted a test that never ran (no such test, none collected, a conftest that does not import, a usage error, no pytest installed) as red, and named a parametrized case NotFound; `src/`-layout Python cycles were folded into one node and never reported; the import policy classed every Python import as external; and `/migrations/` left migrations out of the grade in every language, not only Python. Outside this ADR, and not built by it: `hexa spec tests` generating Python tests, `hexa scaffold` and `hexa init --scaffold` emitting Python, and `hexa init` explaining `lifecycle_enforcement` (decision 9; recorded as gaps).

netbox-plugin (54c73e52, tracked files): read as python (pyproject.toml); 262 files analysed, 917 import edges; grade F 0/100; 0 of 262 files with a layer; analyze.layers needed: every package directory, none declared yet (netbox_plugin/utilities 147 files, management 33, change_control 22, api 6, graphql 4, templatetags 2, plus scripts/ and development/ to exclude); 759 dead exports, most of them Django and NetBox entry points reached by name: an open defect under this ADR's Mitigation (a false dead export on a framework entry point is a defect to fix), not yet fixed.

nautobot-ssot-plugin (0739bd2, tracked files): read as python (pyproject.toml); 72 files analysed, 123 import edges; grade F 0/100; 0 of 72 files with a layer; analyze.layers needed: none declared yet (nautobot_ssot/integrations 30 files, management 11, api 3, and models.py, jobs.py, views.py, urls.py, navigation.py, __init__.py, plus scripts/ to exclude); 52 dead exports.
