---
id: ADR-2610221834
status: proposed
date: 2026-10-22
---
# ADR-2610221834: Harden fixes in parallel worktrees, lands each fix serially, and proves each regression test red

**Status:** Proposed
**Date:** 2026-10-22
**Drivers:** Hardening ADR-2610221540's three new files on 2026-10-22 took about 45 minutes, and the operator asked why hexa is so slow. The session transcripts give the time per phase. Checking the result by hand then showed the regression tests had never been proven red, and the gate had let a flaky test and two lint errors through.

## Context

Measured on 2026-10-22 from the `claude -p` session transcripts. The run was `hexa harden` on `hexa-exec/src/spec_author.rs`, `hexa-cli/src/commands/spec_ledger.rs` and `hexa-exec/src/direct_exec.rs`, one after another:

| Phase | Wall time per file | How it runs |
|---|---|---|
| hunt | 0.5–1.5 min | 4 lenses in parallel |
| verify | 0.3–0.7 min | 12–19 claims in parallel |
| fix | 12–22 min | one `claude -p` per fix, strictly one after another, 0.3–2.7 min each |
| gate between fixes | ~6 s | incremental `cargo test` |

About 80% of the time is the fix phase, because `run_review` (`hexa-exec/src/adversarial.rs`, phase 3) runs one agent at a time in the operator's working tree. The tree is shared, so a second agent cannot start until the first one's change has been gated and committed. The three targets also ran one after another, because `hexa harden` takes a single target. `--fast-gate` (ADR-2610160756) would not have helped here: the gate cost seconds, and the agents cost minutes.

Every fix prompt asks for "a regression test that fails without the fix", and nothing checks that it does. Checked by hand afterwards, by reverting each fix's code and keeping its test:

- 19 of 33 new tests failed without their fix.
- 13 did not compile without it, because they called a function the fix added.
- 1 passed without it.

The checking took longer than the hardening. It had to separate test hunks from code hunks inside files that hold `#[cfg(test)]` modules, which is fragile and was wrong twice.

The same pass showed the gate leaving three gaps:

- A test harden added fails about one full run in four. Each per-fix gate passed by luck, and the final gate caught it.
- Two clippy errors were committed, because the gate ran tests only.
- The gate left out `hexa-cli`'s unit tests, so the regression tests harden put in `spec_ledger.rs` never ran during the pass.

The gate was the operator's to choose. Harden gave them no project default to fall back on.

## Decision

1. **`hexa harden` takes one or more targets.** The hunt and verify phases run over all of them at once, and the confirmed findings form one pool to fix. A pass over three files waits for one hunt, not three.

2. **Each confirmed fix is written in its own git worktree, in parallel.**
   - Each worktree is made from the pass's base commit through the `Worktrees` port (`hexa-exec/src/ports.rs`) and holds one fix.
   - At most `--jobs <N>` agents run at once; the default is 4.
   - Each agent runs with its worktree as `cwd`, through `Frontier::run`. It never sees the operator's working tree, so it never sees their uncommitted files.

3. **Each fix is written test-first, in two steps in its worktree.**
   - **Step A:** the agent writes only a regression test for the finding. Hexa commits it in the worktree as `test(harden): <title> (failing)`.
   - **Step B:** a second agent session, given the finding and step A's commit, makes the fix. It may not change any file step A created under a `tests/` directory. A step-B change that does is refused, as a sealed test is (ADR-2610221540 §6).
   - Hexa commits step B in the worktree as `fix(harden): <title>`. Committing in a worktree uses a new port operation, `GitHistory::commit_in(&self, tree: &Path, paths: &[String], message: &str) -> Result<String, String>`, which returns the new commit's id.

4. **Fixes land on the branch one at a time, in finding order, and each one must show red, then green.** For each fix, in the operator's tree:
   - Hexa cherry-picks the step-A commit (`GitHistory::cherry_pick`) and runs the gate. **The gate must fail.** A gate that fails because the build fails counts as red, and is reported as "red: did not build".
   - Hexa then cherry-picks the step-B commit and runs the gate. **The gate must pass.**
   - If the gate passes with the test alone, that test does not catch the bug. Hexa resets the branch to where it was before the pair (`GitHistory::reset_to`), drops the fix, and reports it as "no red test". It does not commit it.
   - If the gate fails with the fix, the pair is dropped the same way and reported, as today.
   - The landing order is fixed, so the result does not depend on which agent finished first.

5. **A fix that no longer applies cleanly is written again on the current branch, once.** If either cherry-pick conflicts, the pick is aborted, which leaves the tree as it was. The finding then gets steps A and B again in a new worktree made from the current head, followed by §4. A second conflict drops the finding and names it.

6. **The pair is the unit throughout.** The step-A commit is red on purpose and the step-B commit is green, so ADR-2610160756's batches, bisection and last-chance retry count, cut and retry a fix as its two commits together. A red step-A commit is never taken as a culprit.

7. **The project names its harden gate.** `.hexa/project.json → harden.gate` is the default when `--gate` is not given, and `--gate` overrides it. This repository's is `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`. A pass with no gate from either place is refused, as today.

8. **Every phase is timed, and the report says where the time went.** The report records the wall time of hunt, verify, fix, landing and the final gate, plus the longest single agent session. The verdict line prints them, so any later speed change can be measured against this run.

## Consequences

- For a run like the one measured, the fix phase drops from the sum of its agents' times to about ⌈fixes / jobs⌉ times the longest one, plus one gate per landing step. With 4 jobs, 29 fixes over three files would have taken about 8 longest-fix rounds instead of 29 sequential fixes. Hunt and verify happen once instead of three times.
- Every committed fix now has a red commit followed by a green one, which is the same evidence ADR-2610221540 requires of spec tests. A fix with a test that cannot fail is no longer committed. Some real fixes will be dropped for having a weak test. That is the intended trade: they are reported, not lost.
- **Cost:** two agent sessions per fix instead of one, and two gate runs per landing instead of one. The sessions run in parallel. Each gate run costs seconds here, but a project with a slow gate should pair this with `--fast-gate`.
- **Cost:** a worktree per running fix. A Rust worktree builds in its own directory unless it shares `CARGO_TARGET_DIR`. The agent's own builds there may be cold. Gating stays in the operator's tree, which is warm.
- **Known limit:** an inline Rust test (`#[cfg(test)]` in a source file) does not live under `tests/`, so §3's refusal cannot keep step B from editing it. §4 still requires the gate to fail at step A and pass at step B. An edit that weakens the test in step B is visible in that commit's diff, but nothing rejects it.
- **Known limit:** a step A that breaks existing behaviour instead of adding a test also makes the gate fail. The red check cannot tell the two apart. Step B must then repair what step A broke for the gate to pass, so nothing broken lands.
- The history gains a `(failing)` commit for each fix. A project that wants one commit per fix can squash the PR.
- The `claude -p` sessions still run with `--dangerously-skip-permissions`. Scoping them is ADR-2610161730, which is accepted and not built.

## Implementation

- `hexa-exec/src/ports.rs`: `GitHistory::commit_in`.
- `hexa-exec/src/adversarial.rs`:
  - phase 3 becomes a parallel step-A/step-B pool in worktrees, followed by serial landing with the red-then-green check;
  - a conflicted pair is written again once (§5);
  - batching works on pairs (§6);
  - the phase timings (§8).
- `hexa-cli/src/commands/build.rs` (where `harden` is defined): several targets, `--jobs`, and the `harden.gate` default.
- `.hexa/project.json`: this repository's `harden.gate`.

## Gate

`hexa spec verify ADR-2610221834`: the spec's sealed tests, each red before the code (ADR-2610221540 §5). Until the spec is drafted, accepted and sealed, this fails, which is the truth: nothing is built.

## References

- ADR-2610141733: harden commits each fix that passes the gate. Kept, with the pair as the unit.
- ADR-2610160756: batched gating and bisection. Kept, with the pair as the unit (§6).
- ADR-2610221540: an ADR is compiled to a spec and the spec to sealed red tests. This ADR applies the same red-before-green rule to harden's own regression tests.
- ADR-2610161730: scoping the delegate's permissions. Not built; noted under Consequences.
