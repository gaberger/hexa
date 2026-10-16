---
id: ADR-2610160756
status: proposed
date: 2026-10-16
---
# ADR-2610160756: Harden gates each fix fast and each batch in full, and finds a breaking fix by bisection

**Status:** Proposed
**Date:** 2026-10-16
**Drivers:** weave ran `hexa harden` with its full gate (`bun run typecheck && node --import tsx src/cli.ts doctor && bun run test`, about 5,300 tests). That gate takes 1m27s–1m39s on an idle host (runs on 2026-10-15). ADR-2610141733 §6 runs it after every fix. A pass over weave's sandbox runners kept 38 fixes, so it spent about an hour on gates alone. Two passes running at once on one host on 2026-10-16 slowed each other further. The operator asked whether hexa could work in batches without losing confidence.

## Context

The fix loop in `run_review` (`hexa-exec/src/adversarial.rs`, phase 3) does this for each confirmed finding:
- it snapshots the tree;
- it asks the frontier for a fix plus a regression test;
- it runs the whole `--gate`;
- it then commits the fix alone, or restores the snapshot.

After the loop it runs the same gate once more as the final gate. With *n* fixes that is *n* + 1 full gates. When the last fix committed, the last of those reruns a tree that has just passed.

Running the full gate after every fix buys three things:

1. **Each fix commit is green on its own.** Only someone bisecting the branch later depends on that.
2. **A red gate names its culprit.** Only one fix changed since the last green.
3. **A red fix is dropped without touching the others.** Since §6, each fix starts from a green tree.

What ships is the branch head, and the final gate covers that in any design. Of the three, only 1 needs a full gate per fix. 2 and 3 can be had for far fewer full gates when red is rare. Fixes reach this loop only after a separate verifier confirmed the bug, so red should be rare; it is.

The per-fix gate also has a gap. A fix the gate rejects is reverted and never tried again. If it failed only because a later fix was missing, it stays lost.

Most of what the per-fix gate catches is caught by a far smaller command: the fix's own regression test, the tests of the files it touched, and the type checker. What that command misses is a fix breaking something in a file it did not touch. In weave that has happened through shared sockets, globals and patched built-ins. A full gate over the batch catches it before anything ships.

## Decision

1. **`hexa harden --fast-gate <cmd>` turns on batched gating.** Without it, the loop is unchanged: a full `--gate` after every fix (ADR-2610141733 §6). Opting in keeps a project's existing guarantee until it asks for the trade.

2. **Every fix is gated by the fast gate.** `{touched}` in the command is replaced by the fix's changed paths, shell-quoted and space-separated. A project can then run only its relevant tests, e.g. `bun run typecheck && node --test $(scripts/tests-for {touched})`.
   - The rules for `--evidence` apply. It runs under `pipefail`. A fast gate whose output `evidence_is_vacuous` rejects (it ran no tests) counts as failed, not passed.
   - A fix that passes is committed alone, as today. The commit carries a `Hexa-Gate: fast` trailer, so history says which commits were proven only within their batch.
   - A fix that fails is restored to the snapshot, as today.

3. **A batch is closed with one full gate.** A batch is the fixes committed since the last full gate that passed.
   - It closes after `--batch <N>` fixes, default all of them, and at the end of the pass.
   - If the full gate passes, the batch stands and its head is recorded as the new green base.

4. **A red batch is bisected over its prefixes.** Commits apply in order, so the search runs over prefix length: the base plus fixes 1..*k*.
   - Each step resets the scratch state to the base and applies the prefix by cherry-pick.
   - Hexa then runs the full gate. The search finds the smallest red prefix *k*, and fix *k* is the culprit.
   - The culprit is dropped. Fixes *k*+1..*n* are reapplied on top of prefix *k*−1. A fix that no longer applies cleanly is dropped too, and named.
   - The rebuilt batch gets one full gate. If it is still red, the search runs again.
   - Cost: one bad fix in *n* costs about log₂(*n*) + 1 full gates. Today's design costs *n*.
   - The search assumes that once a bad fix is in, every longer prefix stays red. Where a later fix repairs it, the full batch passes and no search starts.

5. **A culprit is confirmed before it is blamed.** Before dropping fix *k*, hexa reruns the full gate on prefix *k*−1 (expected green) and on prefix *k* (expected red).
   - If either does not reproduce, the gate is flaky. Hexa stops bisecting and keeps the batch out of the green base.
   - It reports the flake, naming both prefixes and both outputs. It does not drop an innocent fix.
   - Today a flaky gate quietly reverts a good fix.

6. **A dropped fix is tried once more, last.** After the final batch is green, each fix dropped by §4 or rejected by the fast gate is reapplied once on the final tree. Hexa gates it with the fast gate and a full gate together. A fix that failed only because a later fix was missing now lands; any other is reported as before.

7. **The last full gate is the final gate.** When the last batch's full gate ran on the final tree and passed, that tree is not gated again. `gate_run` and `gate_passed` record that run.

8. **The report says how each fix was proven.** `fixed` keeps meaning gated and committed. The verdict line adds:
   - the number of full gates run;
   - the batches;
   - the fixes dropped by bisection, each with its culprit evidence;
   - the flakes found.

## Consequences

- A pass with *n* fixes and no breaking fix runs one full gate instead of *n* + 1. Each fast gate costs seconds. On weave's sandbox pass that is about an hour of gates cut to a few minutes.
- What ships is gated in full exactly as before. The branch head has always passed the full gate, and every fix still passed a gate that ran its own regression test.
- **Given up:** a commit inside a batch may fail the full gate on its own. Its `Hexa-Gate: fast` trailer says so. A project that needs every commit green can squash the PR or leave `--fast-gate` off.
- **New risk:** a fast gate that misses what a fix touches. It is bounded by the batch gate, which costs log₂(*n*) extra full gates to locate the fault, but never lets it ship. A badly written fast gate can still say little; the vacuity check applies only to its output.
- A flaky full gate now stops the pass and is reported, instead of silently reverting a fix.
- Projects that never pass `--fast-gate` see no change.

## Implementation

- `hexa-exec/src/adversarial.rs`:
  - phase 3 is split into a per-fix step (fast gate, commit with trailer, restore) and a batch step (full gate, prefix bisection, confirmation, rebuild);
  - the last-chance retry;
  - skipping the redundant final gate.
- `hexa-exec/src/direct_exec.rs`: substituting `{touched}` into a gate command, with each path shell-quoted.
- `hexa-cli/src/commands/build.rs`:
  - `--fast-gate <cmd>` and `--batch <N>`;
  - the new report fields.
- `hexa-git`: new cherry-pick and reset-to-commit operations for the prefix rebuild (it has only `worktree.rs` today), exposed through a port as ADR-2609241707's grade requires. Work happens on the current branch: the base is a commit hexa made itself, and the operator's uncommitted files are never touched (ADR-2610141733 §6 still holds them back).

**Gate**, written before the code: `cargo test -p hexa-exec --test harden_batched_gate`, a scripted frontier and gate in a scratch repository:

- five clean fixes with `--fast-gate`: five commits, each with `Hexa-Gate: fast`, exactly one full-gate run, and a final gate that is not run again;
- five fixes, the third breaking a test in a file it did not touch: the fast gate passes it, the batch is red, bisection blames fix 3 in at most ⌈log₂ 5⌉ + 1 + 2 full gates, and fixes 1, 2, 4 and 5 are committed on a green head;
- fix 2 that passes only once fix 4 is in: dropped by the fast gate, retried last, and committed;
- a gate scripted to flake once: reported as a flake, no fix dropped, the batch not counted as green;
- a fast gate that ran 0 tests: the fix is rejected as vacuous;
- `{touched}` with a path containing a space reaches the command as one argument;
- without `--fast-gate`: the full gate runs after each fix, as today (ADR-2610141733's `harden_commits_every_fix` still passes unchanged).

## References

- ADR-2610141733: harden commits each fix that passes the gate. Kept; this ADR changes which gate a fix must pass.
- ADR-2609131646: a gate that never ran is not a failed gate.
- ADR-2609121400: the executable gate replaces the written spec.
