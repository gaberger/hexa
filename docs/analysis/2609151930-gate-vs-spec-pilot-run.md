# Pilot run: gate-first versus BMAD versus Spec Kit

**Date:** 2026-09-15
**Pre-registration:** [`2609152100-gate-vs-spec-trial-preregistered.md`](2609152100-gate-vs-spec-trial-preregistered.md)
**Status:** Pilot. One task, not one of the ten. Run to validate the harness
before the real trial, on a subject the operator is not blind to.

## Result, stated first

**Gate-first did not win. By the pre-registered rule, both spec-driven arms
did.**

All three arms passed the held-out check. Both spec-driven arms finished with
a clean test suite. The gate-first arm finished with a failing test in its own
crate and a diff more than twice the size of either. The rule says a
spec-driven method wins if it matches gate-first on held-out pass and has
strictly fewer surviving defects. BMAD and Spec Kit each satisfy that.

This is a pilot with one task and a terminated arm, so it settles nothing
about the method. It is published because a pre-registration that only gets
published when it wins is not a pre-registration.

## Setup

Three clones of this repository at `9c61f2d`, the commit before the
runs-count fix, with all later history stripped. Each clone got one method and
the same issue: the bug report an operator would have written for the 8% pass
rate, with no diagnosis and no file named.

The two spec-driven arms had hexa's hooks, skills and method rules removed and
a neutral project description in their place. The only hexa in their
environment was the binary under repair, which they were forbidden to invoke.
Each arm ran as a separate agent in its own worktree, same model, 90-minute
cap.

**Held-out check**, written before any arm ran and derived from the issue, not
from any fix: seed a store with two real runs and three subagent hook rows,
run the arm's built binary, require the counts to describe the two runs and no
bare rows to be listed. It fails on the pre-fix tree and passes on the fix
this repository shipped.

## Measures

| | Gate-first | Spec Kit | BMAD |
|---|---|---|---|
| Held-out check | pass | pass | pass |
| `hexa-exec` lib tests | **FAILED, 1 of 126** | ok, 111 | ok, 119 |
| `hexa-cli` lib tests | ok, 283 | ok, 283 | ok, 283 |
| Clippy | clean | clean | clean |
| Diff | **1228 +, 101 −, 5 files** | 495 +, 38 −, 6 files | 511 +, 81 −, 10 files |
| Non-equivalent mutants caught | 3 of 3 | 3 of 3 | 3 of 3 |
| Tests written before the change | all 3 | all 5 | 8 of 14 |
| Wall clock | ~31 min, terminated | 23 min | 28 min |

## What each arm did

**Gate-first** wrote a black-box gate first, a test that drives the built
binary against a seeded store, ran it, recorded it red, and committed it
before touching any source. `hexa do run` then fixed the file in one step in
about 40 seconds. Four minutes in, the issue was fixed and the gate was green.

Then `hexa harden` ran for 25 minutes. It found two real bugs unrelated to the
issue: a Go test filter matching zero tests read as non-vacuous, defeating the
vacuous-gate guard, and the `steps` field was never persisted, so every run
read back `steps: 0` next to `attempts: 3`. Both are genuine. Neither was
asked for.

Harden also added a test, `commit_root_target_without_package_does_not_sweep_nested_dirty_files`,
which fails in the arm's final state. It does not exist in the other two arms.
The arm's gate was the single do-runs integration test, so the broken crate
test passed straight through it.

**Spec Kit** ran constitution, then its bug extension: assess, fix, test. Five
tests, all written red before the change inside the fix step. Its own bug-test
step reported the verdict `verified`. Smallest diff, shortest time.

**BMAD** ran project-context, then bmad-build routed to its oneshot path, then
bmad-code-review. Fourteen tests, eight before the change and six added by its
two review passes. It found a second defect with the same root cause that
neither other arm addressed: the history window truncated to 200 raw rows
before filtering, so hook events could push real runs out of view entirely.
It also wrote a deferred-work file naming what it chose not to fix.

## What the pilot actually shows

**The harness works.** The held-out check discriminated correctly, failing
pre-fix and passing post-fix. The mutation harness ran. Both are reusable for
the real trial.

**Two of the three pre-registered measures did not discriminate.** Held-out
pass was 3 for 3. Mutants caught was 3 for 3. With every arm writing tests,
neither measure separated the methods on a task this small. The real trial
needs harder tasks or a finer defect measure, or those two measures will
report ties for ten tasks running.

**The thing that separated the arms was scope discipline, and gate-first lost
it.** Gate-first produced 2.5 times the diff of either spec arm and the only
failing test. The cause is visible: the adversarial pass is unbounded by the
issue, and the gate that was supposed to catch its mistakes was narrower than
the code it was allowed to change. That is this project's own lesson about
narrow gates, demonstrated against the project.

**Test-independence did not separate them either.** Gate-first wrote its test
before the implementation, from the issue, by a different step. But so did
Spec Kit, all five of them. BMAD wrote eight of fourteen before. The claim
that spec-driven methods write tests only from the implementing agent's
understanding did not hold for either method as run here.

## What invalidates this pilot

Recorded rather than argued:

- **The gate-first arm was killed** by an API spend limit at about 31 minutes,
  mid-harden. Its final state is whatever harden last committed, not a state
  it chose. The failing test may be something harden's own fix loop would have
  caught had it been allowed to finish. The other two arms ran to completion.
- **One task, and it is hexa's own bug.** The operator wrote the gate, the
  issue and the held-out check, and knows the shipped fix. Not blind, not
  independent, not generalisable.
- **The spec arms were told which workflow to run.** Spec Kit was pointed at
  its bug extension and BMAD at its help skill. A real operator choosing
  wrongly is part of a method's cost and this pilot excluded it.
- **Defect counts are objective test results only.** No blind reviewer looked
  at the three diffs, because there was no second person. Measure 2 in the
  pre-registration calls for one and this pilot did not have it.

## What changes before the real trial

1. **The gate arm's gate must cover what the arm is allowed to change.** If
   harden may touch a whole crate, the gate is the crate's suite, not one
   integration test. This is a rule about running the method correctly, and it
   goes into the arm's instructions rather than being fixed after the fact.
2. **Drop or replace the mutation measure** unless harder tasks make it
   discriminate. Three arms at 3 of 3 measured nothing.
3. **Get a blind reviewer.** Without one, measure 2 collapses to "does the
   suite pass", which is a much weaker claim than the pre-registration makes.
4. **Add a diff-scope measure with weight.** It was the only measure that
   separated the arms and it was listed as unweighted reporting.
5. **Budget for termination.** Three agents on a frontier model exhausted a
   spend limit inside 30 minutes. Ten tasks times three arms will not fit
   without a plan.

## Artifacts

The three arm directories are kept at `/home/gary/projects/trial/{hexa,speckit,bmad}`,
each a git repository with its own `TRIAL-LOG.md`, its method's artifacts, and
its commits. The held-out check and mutation harness are in this session's
scratch directory and should be moved into the repository before the real
trial.
