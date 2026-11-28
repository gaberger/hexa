# ADR-2611251938: Python is graded where a project says it is Python, and a name its module uses is not dead

**Status:** Proposed
**Date:** 2026-11-25
**Built:** both decisions, 2026-11-26, 07db8ed (17 sealed scenarios)
**Drivers:** Two projects that are not Python projects fail their grade on the build that adds Python. One, in Rust and TypeScript with 80 helper scripts and no Python manifest, grades A+ 100 on the released hexa and F 20 on this one: 444 dead exports, all Python, 442 of them names used again in their own file, and 71 scripts in no layer. A second, in Rust with 24 scripts and no Python manifest, falls from A+ to B in the same way. Nothing told either that Python had started to count.
**Supersedes:** none. Amends ADR-2611211324 §1 (every `.py` file is graded) and §3 (every public name that nothing imports is a dead export). Neither has been released.

## Context

ADR-2611211324 §4 already decides what makes a project's language Python: a `pyproject.toml`, `setup.py`, `manage.py` or `requirements.txt`. §1 grades every `.py` file whether or not the project is one. A script beside a Rust or TypeScript tree is not a module of a hexagon, has no layer, and its top-level names are not an interface anyone could import.

Python has no export keyword, so §3 calls every public top-level name an export. In the other three languages a project clears a name used only in its own file by dropping `export` or `pub`. In Python the only ways are to rename it with a leading underscore or to write `__all__`.

## Decision

1. **Python is graded only where the project says it is Python.** The `.py` and `.pyi` files are read when the project has a Python manifest (ADR-2611211324 §4), or when an `analyze.layers` key covers at least one of them. Otherwise none is parsed or graded: no import, export, cycle, import-policy or layer-coverage finding comes from them, and they do not cap the score, as before ADR-2611211324. `hexa analyze` then prints how many Python files it did not grade and how to have them graded. A manifest is looked for where `hexa analyze` already looks for one.
2. **A Python name its own module uses is not a dead export.** In a module that assigns no `__all__`, a top-level public name that nothing imports, but that the module's own code refers to somewhere other than where it is defined, is not reported. A mention only in a comment or a string is not a use, and a definition that refers only to itself does not use itself. In a module that assigns `__all__`, the names it lists are its exports as before.

## Consequences

- A project with helper scripts and no Python manifest grades as it did on the released hexa.
- A repository that keeps real Python code with no manifest is graded once it declares a layer for any of it; until then the printed count says what was left out.
- Ungraded Python is also outside the text rules and import policies of `.hexa/ADR-rules.toml`. This differs from the released hexa, which ran a text rule naming `.py` over such files.
- A layer key over a whole folder that holds one `.py` opts all the project's Python in.
- Graded Python includes `.pyi` files. A stub beside its `.py` describes that module, so its names are judged in the module alone. A stub with no `.py` is what an import of the module reaches, and its own imports are edges, so one used only for a type hint can report a cycle or a boundary crossing the compiled module does not have.
- Decision 2 is weaker than the rule for the other three languages: a name used locally and meant for others, that nothing imports, is no longer reported. A name that nothing at all refers to still is.
- Two other differences from the released hexa on the first project stay, and are not changed here. Its two TypeScript cycles are real: the released hexa reports the same two files as a cycle when one is named `.ts` and misses it when it is named `.tsx`, because it does not resolve a `.js` import to a `.tsx` file. Its three ADR-rule errors come from ADR-2611230900 reaching a file it declares in a layer.

## Implementation

Spec after acceptance, tests first.

**Gate**, written first: `hexa spec verify ADR-2611251938` — every scenario of the accepted spec passes, and none is a mirror.

## References

- ADR-2611211324 (hexa reads Python), ADR-2611230900 (a layer policy covers declared files), ADR-2611221645 (TypeScript path aliases).
