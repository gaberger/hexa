# Gate-first versus spec-first: pre-registered trial

Written before the run. Not edited after. The run is a separate document.

## Why this document exists

Every number hexa has published about gate-first development comes from hexa
building hexa. The 36-task workplan that was wrong in four places, the 44 dead
specs, the 777 lines from one gate: all of it is one team's own repository. A
developer who does not already believe the claim is right to discount it.

This trial is the first that can lose. It is run on code hexa's authors did
not write, on tasks they did not choose, against a spec-first arm run the way
people actually run it, with the ground truth held out from both arms and the
reviewer blind to which arm produced what.

## The claim under test

A gate written before the code, and enforced by the tool that makes the
change, produces fewer defects that survive to review than a spec written
before the code and enforced by reading. And when the code later drifts, the
gate detects it and the spec does not.

Two claims, two measures. Both are pre-registered below.

## Subject

A public repository, chosen by rule rather than by taste, so the choice
cannot favour the tool:

- A language hexa can parse: Rust, Go, or TypeScript.
- Between 20,000 and 100,000 lines of source. Large enough that
  localisation is real work; small enough that a task fits one session.
- A test suite that is green at the chosen commit and runs in under five
  minutes.
- At least 40 issues closed by a merged pull request that added or changed a
  test, in the last two years.
- Nobody on this project has contributed to it.

Selection rule: sort GitHub search results for the language by stars, take
the first repository that satisfies every criterion above, record the search
query and the date. If it is later disqualified for a reason not on this list,
the disqualification and the reason go into the results document.

## Tasks

Ten issues from the subject, chosen by rule:

- Closed by exactly one merged pull request.
- That pull request added or changed at least one test.
- The issue text alone is enough for a maintainer to know what to do. Judged
  by one person before the run, blind to the fix, and recorded.
- Selected as the ten most recent that qualify. No cherry-picking.

For each issue, the ground truth is the tests the maintainer's pull request
added or changed, checked out from after the merge and run against the
pre-merge tree with each arm's change applied. Neither arm sees those tests.
Neither arm sees the pull request. Both arms see the issue text and the
repository at the parent commit of the merge.

This is the SWE-bench shape. It is used because it is the accepted way to
judge a change against what the maintainers actually wanted, and because it
gives a pass or fail that nobody in this project decides.

## The two arms

Same model, same tier configuration, same wall-clock cap of 45 minutes per
task, same token budget recorded from `hexa spend`. Run on the same machine,
alternating arms per task so drift in the model or the machine does not favour
one side.

**Arm S, spec-first.** The operator writes a behavioural spec for the issue in
the project's existing style, Given/When/Then, before any code. An agent is
given the spec and the repository and asked to implement it. The operator
reviews the result by reading the diff and the spec side by side, and may send
it back once. The spec is the guidance artifact. Nothing runs it.

This is how spec-driven development is practised with an agent today. The
arm is not hexa's retired workplan pipeline; using that would be arguing
against a tool nobody uses.

**Arm G, gate-first.** The operator writes a gate for the issue before any
code: a test, or a command, that fails on the parent commit and would pass if
the issue were fixed. It is written from the issue, not from the maintainer's
fix, which the operator has not seen. The change is made with `hexa do run`
where one file suffices and `hexa build` where it does not, against that gate.
Then `hexa harden` runs on the result. The gate is the guidance artifact. It
runs.

Both arms may use any tool for localisation. If a hexa verb is missing for
something the gate arm needs, that is recorded as a finding against hexa and
the operator does it by hand, timed.

## Measures, in order of weight

1. **Held-out pass.** Do the maintainer's tests pass against the arm's
   change? Binary per task. The primary measure.
2. **Surviving defects.** After both arms finish a task, a reviewer who does
   not know which diff came from which arm examines each and lists defects:
   wrong behaviour, missed case, broken invariant, dead code. `hexa harden`
   is run on both diffs by a third person and its confirmed findings are
   added. Count per arm.
3. **Drift detection.** After the run, each arm's change is mutated: three
   single-token mutations per task in the changed lines, chosen by a rule
   from the mutation-repair trial. For each mutation: does the arm's
   guidance artifact detect it? For a gate, run it. For a spec, the answer is
   no by construction, and the trial records it rather than assumes it, by
   asking the same blind reviewer whether the spec would have caught the
   mutation on a re-read.
4. **Time to runnable.** Minutes from task start until a stranger could check
   out the branch and run the suite green.
5. **Guidance written.** Lines of spec versus lines of gate. Reported, not
   weighted.

## What counts as a result

- **Gate-first wins** if arm G's held-out pass count is at least arm S's and
  arm G has strictly fewer surviving defects summed over the ten tasks.
- **Spec-first wins** if the reverse holds.
- **No result** otherwise, and the document says so.

Drift detection is reported separately. It tests the second claim and is
expected to be one-sided; if it is not, that is the most interesting finding
in the trial.

Ten tasks is small. The results document reports the per-task table, not
only the sums, and states that a difference of one or two tasks is noise.

## What would make this trial invalid

Recorded now so they cannot be discovered later as excuses:

- The operator writing the gate has seen the maintainer's fix.
- A task where the gate the operator wrote is itself wrong. That is scored as
  a gate-arm failure, not excluded.
- Either arm exceeding the time cap. Scored as a failure for that task.
- The blind reviewer learning which arm produced a diff. That task's defect
  count is discarded.
- Changing any criterion above after the first task starts.

## What this trial does not show

It does not show that gate-first is better for greenfield work; both arms
start from an existing codebase. It does not show anything about teams; one
operator runs both arms. It does not show that hexa is the best tool for
gate-first; it shows whether the method beats the other method with the tool
that exists.

## Costs

Twenty runs of up to 45 minutes, two harden passes per task, and three
mutation runs per arm per task. Inference cost is recorded per task from
`hexa spend` and published with the results.

## Publication

The results go in `docs/analysis/` next to this file, with the per-task
table, the selection query, the ten issue numbers, every gate and every spec
as written, the reviewer's defect lists, and the arm that lost on any
measure. If the trial is abandoned, the reason is published in the same
place.
