# Gate-first versus spec-first: pre-registered trial

Written before the run. Not edited after the first task starts. The run is a
separate document.

**Amendment 1, 2026-09-15, before any task.** The spec arm was first written
as a homemade Given/When/Then spec. That would have been a straw man. The
comparison the project actually needs to win is against the published
spec-driven methods people use with agents today: BMAD-METHOD, GitHub's Spec
Kit, and the requirements/design/tasks shape Kiro popularised. The arms
section below is rewritten to run those methods as their authors document
them, including their own test and QA steps. The measures gained one row,
test independence, because those methods do produce tests and the trial has
to say what is different about a gate.

## Why this document exists

Every number hexa has published about gate-first development comes from hexa
building hexa. The 36-task workplan that was wrong in four places, the 44 dead
specs, the 777 lines from one gate: all of it is one team's own repository. A
developer who does not already believe the claim is right to discount it.

This trial is the first that can lose. It is run on code hexa's authors did
not write, on tasks they did not choose, against a spec-first arm run the way
people actually run it, with the ground truth held out from both arms and the
reviewer blind to which arm produced what.

**Amendment 2, 2026-09-15, before any task.** The task design was wrong for
the claim. Ten bug-fix issues measure repair, and this project's claim is not
about repair: it is that *the architecture of what gets built holds up as the
system grows*. A single-file fix cannot show that, and the pilot run
(`2609151930-gate-vs-spec-pilot-run.md`) demonstrated exactly that emptiness by
returning 3-of-3 ties on two of three measures. The trial is therefore split
into two task types, and the architecture type carries the weight. See
"Task type A" and "Task type B" below.

## The claim under test

A gate written before the code, and enforced by the tool that makes the
change, produces fewer defects that survive to review than a spec written
before the code and enforced by reading. And when the code later drifts, the
gate detects it and the spec does not.

Two claims, two measures. Both are pre-registered below.

## Task type A: architecture under growth (the weighted type)

Three systems, each built once per arm and then grown by five successive
change requests delivered in a fixed order. This is where the claim lives.

**The build.** One challenge, one gate, stated identically to every arm. Each
system must need at least two primary adapters and two secondary adapters, so
that boundaries are load-bearing rather than decorative. Candidates, fixed
now: a link shortener with an HTTP API and a CLI over a file store and an
in-memory cache; a rate limiter with an HTTP middleware and a library API over
Redis-shaped and in-memory backends; a job runner with a CLI and an HTTP
status endpoint over a filesystem queue and a stub notifier.

**The five changes**, written before the build and not derived from any arm's
design, each delivered to every arm in the same words:

1. Add a second secondary adapter behind an existing port.
2. Replace a secondary adapter with a different implementation. Nothing in
   the domain may change.
3. Add a primary adapter that reuses an existing use case unchanged.
4. Add a domain rule that every primary adapter must now enforce.
5. Remove a feature end to end.

**Measures for type A**, in order of weight:

1. **Boundary integrity after each change.** `hexa analyze --json` run by the
   operator on every arm's tree after every change. Violations, cycles, and
   the grade, as a time series over six points. This is the primary measure.
   Note the asymmetry under test: the gate-first arm may run the analyzer
   during development, the spec-driven arms may not, because their methods do
   not prescribe it. That asymmetry *is* the claim. The analyzer is the
   measuring instrument for all three arms regardless.
2. **Change 2 in isolation.** Did the domain diff stay empty when the adapter
   was swapped? Binary. A method that requires touching the domain to change
   a database has not delivered ports and adapters whatever its file layout
   says.
3. **Blast radius.** Files touched per change, and how many lie outside the
   layer the change belongs to.
4. **The gate still passes** after each change, and the suite is green.
5. **Cost and wall clock** per arm per change.

A method wins type A if its grade never falls below its own build-time grade
across all five changes and it has strictly fewer total violations than the
others at the final point.

## Task type B: repair in unfamiliar code (the secondary type)

The ten-issue SWE-bench-shaped design, kept as written below, demoted to
secondary. It measures whether a method can make a correct small change. The
pilot suggests it will not discriminate much, and it is retained because a
method that wins type A while failing type B would be worth knowing about.

Subject selection, task selection, and held-out ground truth for type B are
unchanged and specified in the next two sections.

## Subject and tasks for type B

### Repository, chosen by rule

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

## What is actually different

Spec-driven methods are not "no tests". BMAD has a QA agent and stories that
call for tests; Spec Kit's tasks include test tasks; Kiro's tasks do too. The
trial therefore does not compare tests against no tests. It isolates three
properties of the gate-first method and measures each:

1. **The guiding artifact runs.** A gate is a command. A PRD, an
   architecture document, a story file, a spec.md, a requirements.md: none
   of them can be executed, so none of them can go red when the code
   drifts.
2. **The test is written before the implementation and not derived from
   it.** In the spec-driven methods, tests are written by the implementing
   agent during or after implementation, from the same understanding that
   produced the code. That is the mirror-test failure: the test encodes the
   same misreading as the code and passes anyway.
3. **The tool refuses the commit.** `hexa do run` reverts on a red gate.
   The spec-driven methods rely on a review step, human or agent, reading
   the result against the documents.

If the trial finds no difference, one or more of these properties does not
matter as much as this project believes. That is a result.

## The arms

Same model, same tier configuration, same wall-clock cap of 90 minutes per
task for every arm (raised from 45 because the spec-driven methods run
several agent roles in sequence and a shorter cap would penalise them for
their shape rather than their output), same token budget recorded from
`hexa spend`. Run on the same machine, arms rotated per task so drift in the
model or the machine does not favour one.

Each spec-driven arm is run as its authors document it, at a version pinned
and recorded in the results document, using its brownfield workflow where it
has one, with every role and step it prescribes including its own review, QA
and test steps. The operator follows the method's documentation and does not
add gate-first practices to it. Where the method leaves a choice to the
operator, the operator takes the method's default or recommended path and
records the choice.

**Arm B, BMAD-METHOD.** The method's brownfield flow: document the existing
project, produce the planning artifacts it calls for, shard into stories,
implement each story with its developer role, and pass through its QA role.
The story files and QA output are the guidance artifacts.

**Arm K, Spec Kit.** GitHub's spec-driven workflow: constitution if the
method calls for one, specify, plan, tasks, implement. The spec, plan and
task files are the guidance artifacts.

**Arm G, gate-first.** The operator writes a gate for the issue before any
code: a test, or a command, that fails on the parent commit and would pass
if the issue were fixed. It is written from the issue, not from the
maintainer's fix, which the operator has not seen. The change is made with
`hexa do run` where one file suffices and `hexa build` where it does not,
against that gate. Then `hexa harden` runs on the result. The gate is the
guidance artifact.

Kiro's requirements/design/tasks shape is not a separate arm. Spec Kit's
artifacts are close enough in kind that a third spec-driven arm would add
cost without adding a distinct claim. If Spec Kit and BMAD disagree with each
other on the primary measure, that disagreement is reported and a Kiro arm
is added in a follow-up.

All arms may use any tool for localisation. If a hexa verb is missing for
something the gate arm needs, that is recorded as a finding against hexa and
the operator does it by hand, timed. If a spec-driven method's documented
step cannot be completed on the subject, that is recorded the same way.

## Measures for type B, in order of weight

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
5. **Guidance written.** Lines of planning and spec artifacts versus lines of
   gate. Reported, not weighted.
6. **Test independence.** For every test in an arm's final diff: was it
   written before the implementation, and by something other than the
   implementing step? Recorded per test from the session transcript. This
   is the direct measure of property 2. A spec-driven arm can score well
   here if its method genuinely front-loads tests; the trial records what
   happened, not what the method's documentation says should happen.

## What counts as a result

Type A decides. Type B is reported alongside and cannot overturn it.

- **Type A** is decided by the rule stated in its own section: grade never
  below build-time grade across all five changes, and strictly fewer total
  violations at the final point. Change 2, the adapter swap with an untouched
  domain, is reported separately for every arm whatever the overall result.
- **Gate-first wins type B** if arm G's held-out pass count is at least each
  spec-driven arm's and arm G has strictly fewer surviving defects than each,
  summed over the ten tasks.
- **A spec-driven method wins type B** if it beats arm G on both measures the
  same way. Each is scored separately; BMAD and Spec Kit are not pooled.
- **An arm terminated by anything other than the time cap** (a spend limit, a
  crashed host, a revoked key) scores *no result* for that task, for every
  arm, and the task is rerun or dropped. The pilot needed this rule and did
  not have it.
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

Type B does not show anything about greenfield work; it starts from an
existing codebase. Type A is greenfield by construction and shows nothing
about arriving in a large unfamiliar system. It does not show anything about teams; one
operator runs both arms. It does not show that hexa is the best tool for
gate-first; it shows whether the method beats the other method with the tool
that exists.

## Costs

Type A: three systems times three arms times six deliveries (build plus five
changes) is 54 agent runs, the dominant cost. Type B: thirty runs of up to 90
minutes, a harden pass per arm per task, and three
mutation runs per arm per task. The spec-driven arms will spend more, since
their methods run more roles; the spend is reported per arm and is itself a
finding. Inference cost is recorded per task from
`hexa spend` and published with the results.

## Publication

The results go in `docs/analysis/` next to this file, with the per-task
table, the selection query, the ten issue numbers, the pinned version of each
spec-driven method, every gate and every planning artifact as written, the reviewer's defect lists, and the arm that lost on any
measure. If the trial is abandoned, the reason is published in the same
place.
