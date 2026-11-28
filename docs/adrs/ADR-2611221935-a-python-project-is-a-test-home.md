# ADR-2611221935: A Python project is a test home for `spec draft` and `spec accept`

**Status:** Completed
> 2026-11-23 Completed: landed on main at 23c20f1; gate `cargo test -p hexa-cli --test adr_2611221935_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no new or changed functionality); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-22
**Drivers:** a Python SDK (a `pyproject.toml` project with pytest tests) ran `hexa spec draft` and was refused: "no Cargo.toml, package.json or test.runners at …". ADR-2611191120 §9 says `spec draft` and `spec accept` "refuse only where no test could be bound", and ADR-2611211324 §8 gave `.py` tests a built-in pytest runner, so a test can be bound in that project; the refusal predates Python support. The workaround was a `test.runners.py` entry naming the runner hexa already has.
**Supersedes:** none. Completes ADR-2611191120 §9 for the language ADR-2611211324 added.

## Context

`require_a_rust_project` (`hexa-cli/src/commands/spec_ledger.rs`) lets `spec draft` and `spec accept` proceed when the root has a `Cargo.toml` or a `package.json`, or `.hexa/project.json` configures a `test.runners` entry; otherwise it refuses, naming those three. `hexa analyze` detects a Python project by a `pyproject.toml`, `setup.py`, `manage.py` or `requirements.txt` at its root (ADR-2611211324 §4; the session header reads `pyproject.toml`, `setup.py` or `setup.cfg`, ADR-2611221646 §4), and `hexa spec red` binds a `.py` test through the built-in pytest runner. The runner rule is `hexa-core/src/domain/test_runner.rs`.

## Decision

1. **A root `pyproject.toml`, `setup.py` or `setup.cfg` is a test home.** `spec draft` and `spec accept` proceed in such a project as they do under a `Cargo.toml` or `package.json`.
2. **The refusal names it.** Where `spec draft` or `spec accept` still refuses, its message lists a Python manifest among the test homes, and names pytest among the runners `hexa spec red` uses for a hand-written test.
3. **Nothing else changes.** A project with a `Cargo.toml`, a `package.json` or a `test.runners` entry passes as before; one with none of these and no Python manifest is refused as before; `hexa spec tests` still generates Rust tests only.

## Consequences

- A Python project drafts and accepts its specs without a `test.runners` entry that only restates the built-in runner.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611221935_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611191120: the test runner is the project's (§9)
- ADR-2611211324: hexa reads Python (§8, the pytest runner)
- `hexa-core/src/domain/test_runner.rs`
