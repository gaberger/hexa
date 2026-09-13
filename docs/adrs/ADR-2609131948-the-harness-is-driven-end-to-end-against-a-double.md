# ADR-2609131948: The harness is driven end to end against a double

**Status:** Accepted
**Date:** 2026-09-13
**Drivers:** `adversarial.rs` is 1452 lines and 60 functions with 25 tests, every one of them on a pure function — parsing an envelope, ordering a verdict, rendering a line. Nothing drives `run_review`. Four defects in it shipped today and were caught by a person watching a terminal.

## Context

The four were not subtle, and each was a claim the report had no right to make:

- a hunt where no lens answered reported a clean file and committed;
- a fix phase treated a refusal as a fix applied, because the reply was `Ok`;
- a truncated generation became `Ok("")` and then `empty reply`;
- four parallel calls to one local model lost three lenses to a timeout.

Every one lives in the sequencing — which phase runs, what it concludes, what it does next — and
the sequencing is exactly what no test touches. The pure functions underneath are well covered, so
the tests were green while the thing they belong to was wrong. This is the shape the whole day was
about, in the harness's own test suite.

The obstacle was real: `run_review` spawns a CLI and makes HTTP calls. Both are already
injectable — `claude_binary()` reads `HEXA_CLAUDE_BINARY`, and the model endpoint comes from a
registry under `$HOME` — but ADR-2609131749 forbids a test writing the process environment, since
cargo runs tests on shared threads. That ADR also names the way out: *anything that truly needs to
set a variable can run in its own process.*

blacksheep's `tests/forward_source.rs` is the pattern, and it was written for the same reason: a
local `TcpListener` serving canned replies, so an adapter's behaviour is pinned without the
internet.

## Decision

1. **An integration test spawns `hexa harden` as a child process** with its own `HOME`,
   `HEXA_CLAUDE_BINARY` and project root. The variables are set on the child, so nothing in the
   test process's environment moves and ADR-2609131749 holds.

2. **Both model paths are doubled.** The frontier is a script that prints what the case requires,
   including the refusal text a spend limit produces. The local model is a `TcpListener` serving
   an OpenAI-shaped reply, including the `content: null` plus `finish_reason: length` shape that a
   reasoning model returns when its budget runs out.

3. **The assertions are on what the report claims**, not on which functions ran: whether it says a
   file was reviewed, who reviewed it, how many lenses answered, whether it committed, and the exit
   code. Those are the sentences an operator acts on and the sentences that were wrong.

4. **Each case is one of the four defects.** A test that would not have caught one of them is not
   worth the process spawn.

## Consequences

- The four defects become regressions rather than history. Each is now a named test that fails if
  the behaviour returns.
- An integration test that spawns a process and binds a port is slower and flakier than a unit
  test. It is bounded to the cases above rather than grown to cover the harness generally.
- `hexa-parser` remains at zero tests over 390 lines. That is the next gap and it is not this one.

## Gate

`cargo test -p hexa-cli --test harden_end_to_end`: a frontier that refuses and a local model that
answers reports the local model as reviewer and its findings; a frontier that refuses and a local
model that also refuses reports that nothing was reviewed, commits nothing and exits non-zero; and
a target whose reply is truncated is reported as a budget failure rather than an empty answer.

## Evidence

`cargo test -p hexa-cli --test harden_end_to_end 2>&1 | grep -E '^test |^test result'` at d6e29bc with uncommitted changes on 2026-09-13 19:49 UTC:

```text
test a_frontier_that_answers_is_used_and_named ... ok
test a_truncated_reply_is_reported_as_a_budget_failure_not_an_empty_answer ... ok
test a_pass_where_nothing_answered_reports_nothing_reviewed_and_does_not_commit ... ok
test a_frontier_that_declines_falls_back_and_the_report_names_the_local_reviewer ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```
