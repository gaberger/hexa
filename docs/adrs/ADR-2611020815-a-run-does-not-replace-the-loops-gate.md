# ADR-2611020815: A run does not replace the loop's gate

**Status:** Accepted
**Date:** 2026-11-02
**Drivers:** In strata on 2026-11-02, the loop held strata's storage-spike ADR with a gate that runs the bench smoke and checks the steady-stall attribution. A `hexa do run` fixed an unrelated flaky server spec with `--evidence "/tmp/s28-gate.sh && cargo test -p strata-server …"`. `direct.rs` wrote that evidence into the loop as its gate. The loop then said "spike work, gated on a /tmp script that tests the HTTP server", and `hexa loop stage done` (ADR-2609132059) would have closed the spike on a green server test. It was caught only because someone read `hexa loop` by eye.

## Context

ADR-2609131408 fixed the same failure between sessions: two sessions erased each other's gate
because the file held one entry. Its premise is that *a decision, a gate and a stage belong to a
body of work*. Within one session the premise still breaks. `hexa do`, `hexa build` and
`hexa harden` each call `update_loop` with `{"gate": <this run's evidence>, "stage": …}` on every
run, which merges blindly. A run's evidence is a fact about that run. The loop's gate is the
operator's decision about when the work is finished. One replacing the other is silent: nothing
is printed, and every later reader (the hooks, `hexa loop`, `stage done`) trusts the replacement.

Since ADR-2609132059, the gate is not only displayed — `stage done` runs it. So a replaced gate is
not a misleading line; it is a done-check that measures something else and still passes.

Recording the run's evidence when no gate is recorded is still right: it is how a loop started
by `hexa build` gets its gate, and the pre-edit hook relies on a gate being present.

## Decision

1. **A run records its evidence as the gate only when the loop has none, or the same one.** An
   absent gate, `null` or an empty string counts as none. In that case the gate and the run's stage
   are written, as before.
2. **A different recorded gate is kept, and so is the stage.** The run is not the loop's work, so
   it does not move the loop's stage either. The run's evidence and stage are written to
   `last_run: {gate, stage, at}` in the session's entry instead, so the run remains visible.
3. **Keeping it is said, not silent.** The command prints one line naming the gate that was kept,
   and that the run's evidence was recorded as `last_run`.
4. **`hexa loop gate '<cmd>'` is unchanged.** It is the operator's decision and always replaces
   the gate. Changing the work's gate is that command's job, not a side effect of running one.
5. `hexa do`, `hexa build` and `hexa harden` (including its `fast_gate`) all go through the one
   function that applies this, `record_run_gate` (callers use its printing wrapper
   `note_run_gate`). No caller writes `gate` through `update_loop`.

## Consequences

- A `hexa do` for side work inside a loop no longer moves the loop. The loop's gate and stage
  describe the loop's work until the operator changes them.
- A loop with no gate still picks one up from the first run, as today.
- An operator who *meant* a run's evidence to become the new gate has to say so with
  `hexa loop gate`. The printed line tells them that.
- `harden`'s `fast_gate` follows the gate: it is written only when the gate was.

## Gate

`cargo test -p hexa-cli run_gate`: with no gate recorded, a run records its gate and stage; with
the same gate recorded, the stage moves; with an empty gate recorded, it counts as none; with a
different gate recorded, the gate and stage are unchanged and `last_run` holds the run's gate and
stage; and `hexa-cli/src/commands/{direct,build}.rs` contain no `update_loop` call that writes
`"gate"`.
