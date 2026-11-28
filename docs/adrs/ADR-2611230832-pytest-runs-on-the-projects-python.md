# ADR-2611230832: The built-in pytest runner uses the project's Python, as vitest uses the project's vitest

**Status:** Completed
> 2026-11-23 Completed: landed on main at becd759; gate `cargo test -p hexa-exec --test adr_2611230832_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: Python brought up to what the other languages have, under his rule; he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** A Python SDK managed with uv (`uv.lock`, `.venv/`) could not bind a sealed test through the built-in runner: it runs the system `python3 -m pytest`, which cannot import the package ("No module named 'client_sdk'"). Every uv or virtualenv project needs a `test.runners.py` override that restates the built-in runner with its own interpreter. The TypeScript runner already runs the project's own tool: `npx vitest` from the test's package, never one it fetches (ADR-2611191120). Python should run the same way.
**Supersedes:** ADR-2611211324 §8, in which interpreter runs the test (it said `python3`); the rest of §8 stands.

## Context

The plan for a `.py` test is built in `hexa-exec/src/test_exec.rs` (`Runner::Pytest`): it checks the test's function is in the file, finds the nearest directory at or above the file that holds a `pyproject.toml`, `pytest.ini`, `tox.ini` or `setup.cfg` (the repository root if none), and runs `python3 -m pytest <file>::<name> -q` there, judged by pytest's summary (`Judge::Pytest`). The plan is `hexa_exec::test_exec::plan_test(root: &Path, r: &str, cfg: &TestConfig, prefix: &str) -> Result<Planned, TestRun>`, whose `Planned::cmd` is the shell command it will run; `TestConfig` is in the same module. The runner rule is `hexa-core/src/domain/test_runner.rs`.

## Decision

1. **A uv project runs pytest through uv.** When a `uv.lock` is at or above the directory pytest runs from (no higher than the repository root), the command is `uv run python -m pytest <file>::<name> -q`, run from that directory.
2. **A virtualenv project runs pytest on its interpreter.** Otherwise, when a `.venv/bin/python` (or `venv/bin/python`) is at or above that directory, no higher than the repository root, the command uses that interpreter: `<that>/bin/python -m pytest …`.
3. **Otherwise `python3`, as before.** With neither, the command is `python3 -m pytest …`, as ADR-2611211324 §8 decided.
4. **Nothing else changes.** Where pytest runs from, how the test is found, how its output is judged (exactly one test passed), and a `test.runners.py` entry winning over the built-in runner are as before. A project whose environment's tool is missing (`uv` not installed) fails with the command's own error, as a missing vitest does; hexa does not fall back to `python3`.

## Consequences

- A uv or virtualenv project binds and runs its sealed tests with the built-in runner, and drops a `test.runners.py` that only named its interpreter.
- A project with neither runs exactly as before.

## Implementation

**Gate:** `cargo test -p hexa-exec --test adr_2611230832_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611211324: hexa reads Python (§8, the pytest runner)
- ADR-2611191120: the test runner is the project's
- `hexa-core/src/domain/test_runner.rs` (`Runner`, `runner_for`)
