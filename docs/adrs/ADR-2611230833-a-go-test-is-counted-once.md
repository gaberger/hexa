# ADR-2611230833: A passing Go test is counted once, with or without `-v`

**Status:** Completed
> 2026-11-23 Amended (pre-PR review): §2 added: a `-v` run of one test with subtests counted each `--- PASS: TestX/sub` line as another test (ADR-2611191120 §2 says exactly one).
> 2026-11-23 Completed: landed on main at 5b36dd2; gate `cargo test -p hexa-core --test adr_2611230833_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no architectural change); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** A Go SDK bound a sealed test with `test.runners.go = go test -count=1 -v -run '^'{name}'$' ./$(dirname {file})`. The test passed, and `hexa spec verify` refused the scenario: "matched 2 tests; a scenario names one". `read_evidence` counts the `--- PASS: TestX` line as one passed test and the package line `ok  	pkg	0.01s` as another, so a `-v` run of one test reads as two, and of N tests in one package as N+1. A configured runner passes only when exactly one test ran (ADR-2611191120 §2), so every Go project that runs its tests verbosely is refused for a correct test, and `hexa loop gate` counts are one too high per package.
**Supersedes:** none. A defect in how ADR-2611191120's counted runs read `go test`.

## Context

`read_evidence(exit_ok: bool, output: &str, uncounted: bool) -> Evidence` in `hexa-core/src/domain/evidence.rs` reads a run's output line by line. For `go test` it counts `--- PASS` as one passed test, `--- FAIL`, `FAIL<tab>pkg` and a bare `FAIL` each as one failed test, `[no test files]` and `[no tests to run]` as recognised with no tests, and a package line starting `ok  <tab>` or `ok<tab>` as one passed test. Without `-v`, `go test` prints only the package line for a passing package, so that line is the only count there is; with `-v` it prints a `--- PASS` line per test and then the package line. `Evidence::Passed { tests }` carries the count that ADR-2611191120 §2 judges (`judge_counted`, `hexa-core/src/domain/test_runner.rs`).

## Decision

1. **A package line counts only for a package whose tests were not listed.** A Go `ok` package line counts as one passed test only when no `--- PASS` or `--- FAIL` line was read for the run before it since the previous package line; when such lines were read, the package line adds nothing. So a `-v` run of one passing test is one test, a `-v` run of N passing tests in one package is N, and a run without `-v` is counted as today.
2. **A subtest counts toward its parent.** A `--- PASS` or `--- FAIL` line whose test name holds a `/` (`--- PASS: TestX/case`, a `t.Run` subtest) is part of the test before the `/`, and counts no test of its own; a failing subtest still fails the run.
3. **Nothing else changes.** A failed Go test still fails the run; `[no test files]` and `[no tests to run]` are read as before; every other runner's lines are read as before.

## Consequences

- A Go test bound with a verbose `go test` runner is judged as one test, and `hexa loop gate`'s count of a verbose Go run is the number of tests it ran.

## Implementation

**Gate:** `cargo test -p hexa-core --test adr_2611230833_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611191120: a test names its runner (§2, exactly one test)
- `hexa-core/src/domain/evidence.rs` (`read_evidence`, `Evidence`), `hexa-core/src/domain/test_runner.rs` (`judge_counted`)
