# ADR-2609131427: The harness says what it is doing while it does it

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** `hexa harden` printed one header line and then nothing for nine minutes. The session watching it could not tell a model call from a hang, killed it, and found a half-applied fix in the tree. The project owner: *"hexa never tells me what it's doing — why can't we get status updates?"* and *"I want hexa loop and you reporting status as it executes."*

## Context

The adversarial harness runs three to five phases, each of which waits on a model for minutes at
a time on a local backend. It reported at the end, as a summary. That is fine for a run that
finishes and useless for a run that is in progress, and every run is in progress for most of its
life. Nothing distinguished "waiting on the model" from "stuck", and the loop, which is supposed
to say where the work stands, said nothing about a run at all.

## Decision

1. **Every phase announces itself, repeats itself while it waits, and reports when it is done.** The
   harness exposes a `Reporter`; a `Phase` is started with a message, emits a heartbeat every thirty
   seconds with its elapsed time, takes notes as results arrive — per lens, per claim, per fix, per
   gate — and finishes with a summary and its total. Dropping a phase stops its heartbeat. Silence
   never means anything.

2. **The CLI prints each event with elapsed time and flushes it**, so a run whose output is piped to
   a file, which is how a session watches it, shows progress as it happens and not at exit.

3. **The loop records what is running.** Each event updates the session's entry with the verb, the
   phase, the last message and the start time; `hexa loop` and the hooks show
   `running harden/verify 4m12s: 3 claims, default refute`. The record is cleared when the run
   ends. This is the same entry ADR-2609131408 made per session, so another session sees the run
   too.

4. **The report-only entry points stay.** `run_review` and `run_build` are unchanged for callers that
   want only the result; `run_review_with` and `run_build_with` take the reporter.

## Consequences

- A person watches the terminal; a session polls the output file; both read the same lines. A
  killed run leaves a last line saying which phase it was in.
- Thirty seconds is the heartbeat, chosen against three-minute model calls: six lines per call,
  none of them news, all of them proof of life.
- The heartbeat is a spawned task per phase; a phase that is dropped without finishing stops it.
  A run that panics stops all of them with the runtime.

## Gate

`cargo test -p hexa-exec progress && cargo test -p hexa-cli running`: a phase with a 20ms heartbeat produces at
least three beats in 110ms, reports its note and its finish with an elapsed time, and beats no more
after finishing; the loop's status line renders a running entry with its elapsed time and renders
nothing once it is cleared.

## Evidence

`(cargo test -p hexa-exec progress && cargo test -p hexa-cli running) 2>&1 | grep -E 'progress_tests|running_and_for_how_long|test result: ok. 1 passed'` at 45ae7b9 with uncommitted changes on 2026-09-13 14:20 UTC:

```text
test adversarial::progress_tests::a_phase_heartbeats_while_it_waits_and_reports_when_done ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 87 filtered out; finished in 0.17s
test commands::loop_cmd::sessions_see_each_other::the_status_line_shows_what_is_running_and_for_how_long ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 248 filtered out; finished in 0.00s
```
