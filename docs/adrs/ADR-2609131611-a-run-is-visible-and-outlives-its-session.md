# ADR-2609131611: A run is visible to everyone and outlives the session that started it

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** ADR-2609131427 made the harness report its phases and put the running record on the session's own status line. The project owner, watching from a second terminal: *"why don't I see hexa loop status?"* — because the awareness line ADR-2609131408 gives other sessions carries the ADR and the gate and not the run. And earlier the same hour a harden pass was killed mid-fix because the tool that launched it had a ten-minute ceiling, which is the failure both those ADRs exist to end.

## Context

Three gaps, one theme: the status has to reach whoever is watching, and the run has to survive
long enough to produce one.

- A run shows on the status line of the session that started it and nowhere else. Every other
  session in the checkout sees `also here: session 15e0cf79 · stage harden · ADR … · gate …` and
  cannot tell a run in flight from an idle session.
- A run that is killed leaves its last record behind. `running harden/fix 3m22s` stayed on the
  line after the process was gone, and the elapsed time kept climbing, so the loop reported a run
  that had not existed for half an hour.
- A run lives in the process tree of whatever started it. A host tool with a timeout, a closed
  terminal, or a hangup takes the run with it, mid-fix, with the tree half-edited. Working around
  it by hand — `setsid nohup … > ~/.hexa/runs/…` — is what a verb is for.

## Decision

1. **The awareness line carries the run.** A session with a run in flight appears to the others as
   `also here: session 15e0cf79 · running harden/fix 4m12s: fixing … · ADR … · gate …`. The run is
   the first thing on the line, because it is the thing that is happening.

2. **A record that has stopped reporting is stale, not running.** Every phase updates the record at
   least once a heartbeat, so a record whose last update is older than two heartbeats is from a
   process that is no longer reporting. It reads `stalled harden/fix, last seen 14:29` on both the
   status and the awareness line. A run is never reported as live on the strength of a number that
   nothing is incrementing.

3. **`hexa harden --detach` and `hexa build --detach`** re-run the same command in its own process
   group with its output in `~/.hexa/runs/<verb>-<target>-<timestamp>.log`, print the log path and
   the pid, and exit. The run then belongs to no terminal and no host tool. Watching it is
   `tail -f` on the path it printed, and `hexa loop` in any session in the checkout.

## Consequences

- Two people, or a person and a session, watch the same run from anywhere in the checkout.
- The stale rule is the one that makes the running line trustworthy: without it every crashed run
  is indistinguishable from a slow one, and a status you cannot trust is worse than none.
- A detached run's exit code reaches no one; its log and its final report are the record. A run
  that must gate a script stays in the foreground.
- Two heartbeats is sixty seconds against model calls of three to five minutes, which is well
  inside a phase and well outside its reporting interval.

## Gate

`cargo test -p hexa-cli visible_run`: a running record appears on the other session's awareness
line ahead of its ADR; a record last updated more than two heartbeats ago reads as stalled with the
time it was last seen, on both lines; a cleared record shows on neither; and `--detach` parses for
both verbs and is dropped from the command the detached process runs.

## Evidence

`cargo test -p hexa-cli visible_run 2>&1 | grep -E 'visible_run::|test result: ok. 5'` at 3bfc55b with uncommitted changes on 2026-09-13 16:15 UTC:

```text
test commands::build::visible_run::a_runs_log_is_named_for_its_verb_target_and_start ... ok
test commands::build::visible_run::detach_is_dropped_from_the_command_the_detached_process_runs ... ok
test commands::loop_cmd::visible_run::a_record_that_stopped_reporting_is_stalled_not_running ... ok
test commands::loop_cmd::visible_run::another_sessions_run_is_on_its_awareness_line_first ... ok
test commands::loop_cmd::visible_run::a_cleared_run_shows_on_neither_line ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 259 filtered out; finished in 0.00s
```
