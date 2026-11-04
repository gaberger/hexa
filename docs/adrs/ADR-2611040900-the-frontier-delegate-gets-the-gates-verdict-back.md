# ADR-2611040900: The frontier delegate gets the gate's verdict back

**Status:** Proposed
**Date:** 2026-11-04
**Drivers:** In the bench/ab A/B run (`bench/ab/RESULTS.md`), `hexa do run` with a Claude delegate was told to untangle `game-sokoban-rs-tangled` under `cargo test && hexa analyze . --grade A`. The delegate made one attempt, the grade gate failed, and hexa reverted the edit and stopped: `✗ failed evidence fail · 1 step(s)`. On the same task the same model succeeded on its own. The gate had the answer (it names every failing edge), and nothing handed it back. In the entangled-refactor family, the gate's verdict is exactly what separated hexa from the agent alone.

## Context

`DirectTask.max_attempts` in `hexa-exec/src/direct_exec.rs` says "Max edit→verify attempts before giving up (default 3)". Only the single-shot path (`--fast`) honours it. The Claude delegate path, `claude_attempts` in `hexa-exec/src/direct_react.rs`, runs the delegate once, runs the evidence once, and on a failure restores the target and every `--also` path. `DirectResult.attempts` is always 1 there.

A failed gate is not a dead end. Its output says what is wrong: a failing test's assertion, or `hexa analyze`'s "✗ src/a.rs → src/b.rs (adapters must not import from domain directly)". A person given that output fixes it. The delegate is never shown it.

## Decision

1. **The Claude delegate makes up to `max_attempts` attempts, default 3.** `claude_attempts` honours `DirectTask.max_attempts` as the single-shot path does. `--attempts` on `hexa do run` sets it for both paths.
2. **A failed attempt's edits are kept for the next attempt.** Between attempts nothing is restored, so the next attempt fixes forward from where the last one stopped. Only when the last attempt fails are the target and every `--also` path restored, as today.
3. **The next attempt is told why the last one failed.** Its prompt carries the evidence command, the fact that it did not exit 0, and the tail of its output, at most 4000 characters, the same text as `DirectResult.evidence_output`.
4. **A vacuous pass is a failure for this purpose.** When `evidence_is_vacuous` rejects the output, the next attempt is told that the gate ran nothing, and is given another attempt.
5. **The first attempt that passes ends the run.** It is committed as today, and `DirectResult.attempts` is the number of attempts spent.
6. **Every attempt's usage is recorded** under the same spend source, so `hexa spend` counts a three-attempt run as three calls.

## Consequences

- A delegate that gets the gate nearly right gets two more chances, with the gate's own words, before hexa gives up.
- A run can cost up to `max_attempts` times what it costs today. `--attempts 1` restores today's behaviour exactly.
- The operator's tree is still restored only after the last failure, so a failed run leaves nothing behind, as today.

## Gate

`cargo test -p hexa-exec claude_attempts`, with a fake delegate:

- one that fails the evidence once and then passes is committed with `attempts == 2`;
- the second attempt's prompt contains the first attempt's evidence output;
- one that always fails is restored after exactly `max_attempts` attempts, and a delegate given `max_attempts: Some(1)` runs once;
- a vacuous pass on the first attempt leads to a second attempt.
