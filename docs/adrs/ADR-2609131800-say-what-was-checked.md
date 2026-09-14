# ADR-2609131800: Say what was checked

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** `hexa go` printed `→ tests failing — fix with: cargo test --workspace` on a workspace where `cargo test --workspace` reports 819 passed and 0 failed. The verb whose entire job is to say what to do next sent the operator to fix nothing.

## Context

`check_tests` runs `cargo test --workspace --no-run`. That compiles the test binaries and does not
execute one of them. Its two branches disagree about what it measured: success prints
`tests compile`, which is exactly right, and failure prints `tests failing`, which names an
activity that never happened.

The failure was also not a compile error. `hexa go` had just spawned a background rebuild — it
prints `binary stale — rebuild spawned` two lines above — and the `--no-run` invocation then
contended with it for the build lock. So a verb raced work it had itself started, and reported the
contention as a property of the code.

Two wrongs in one line, and both are today's pattern: reporting on an activity that was not
performed (ADR-2609131646, ADR-2609131617), and stating a definite result where the check could not
reach one (ADR-2609131655).

## Decision

1. **A check names the activity it performed.** A compile check reports compilation:
   `tests compile` or `tests do not compile`. If a check wants to report on tests passing, it runs
   them.

2. **An inconclusive check says so and does not accuse the code.** When the command cannot run — a
   held lock, a missing toolchain, a spawn failure — the line is `could not check` with the reason,
   and it does not count as a problem to fix. A next-action list exists to be acted on, and an item
   that cannot be acted on does not belong in it.

3. **A verb does not race work it spawned.** The rebuild `hexa go` starts is checked for, and the
   compile check is skipped with `skipped — a rebuild is in flight` rather than run into the lock.

## Consequences

- `hexa go` stops sending an operator after passing tests, which is the whole cost of the current
  behaviour and is paid every time the binary is stale.
- Distinguishing "does not compile" from "could not check" is the same distinction ADR-2609131646
  drew between an empty answer and no answer. Three verbs now share it; it is the house rule.
- The verb still never runs the suite. Running 819 tests to answer "what next" is the wrong trade,
  and saying `tests compile` is an honest smaller claim.

## Also removed

`scripts/validate-readme.sh` calls `hexa readme validate` and names
`repo_readme_is_accurate` as "the canonical test for CI". Neither exists: the verb is gone from the
CLI and the test matches nothing. Its header confidently lists six categories of checks it
performs — numeric counts, asset existence, link resolution, entity references, CLI command
existence — and it performs none of them, exiting on a clap usage error. Nothing in the repository
or the workflows references it. A validator that cannot run is worse than no validator, because its
presence is taken for coverage, so it is deleted rather than left as a promise. Rebuilding README
validation is worth doing and is not this ADR.

## Gate

`cargo test -p hexa-cli checked`: a successful compile reads `tests compile`; a failed compile reads
`tests do not compile` and counts as a problem; an unrunnable check reads `could not check` with its
reason and counts as no problem; and a rebuild in flight skips the check rather than racing it.

## Evidence

`cargo test -p hexa-cli checked 2>&1 | grep -E 'checked_tests::|test result: ok. 4'` at 29ea857 with uncommitted changes on 2026-09-13 18:03 UTC:

```text
test commands::go::checked_tests::a_compile_check_reports_compilation_never_test_results ... ok
test commands::go::checked_tests::only_a_problem_counts_as_a_next_action ... ok
test commands::go::checked_tests::each_state_is_marked_differently ... ok
test commands::go::checked_tests::a_rebuild_in_flight_skips_the_check_rather_than_racing_it ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 268 filtered out; finished in 0.00s
```
