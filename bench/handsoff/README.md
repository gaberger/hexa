# bench/handsoff: can agents ship with no human after the gate?

The claim under test: once a person has written the gate (the command that
must exit 0), the hidden holdouts and the locked files, hexa and its model
ladder deliver correct, well-shaped code with no branches, no PRs and no
review. A person is involved only when a task escalates.

```bash
python3 bench/handsoff/run_handsoff.py --bench bench/handsoff/tasks --out runs --runs 3
python3 bench/handsoff/grade_handsoff.py --runs runs
```

Each task is a folder holding `task.toml` (see `example-task.toml`), `repo/`
(the starting tree, which must pass `hexa analyze . --grade A`), and
`holdout/` (hidden tests, copied in only by the grader, never seen by the
agent).

The grader runs in its own process after the agent is done and gives every run
one outcome: correct, wrong-but-green, misshapen, tampered, escalated, or
silent failure. The deciding number is **wrong-but-green**: a change that
passed the gate the agent saw but fails the holdouts it did not.

## Parallel: several agents, one main

```bash
python3 bench/handsoff/run_handsoff.py --bench bench/handsoff/tasks --out runs/seq --runs 3
python3 bench/handsoff/run_handsoff.py --bench bench/handsoff/tasks --out runs/par --runs 3 --parallel 4
python3 bench/handsoff/grade_handsoff.py --runs runs
```

Tasks that share a starting repo (or name the same `batch`) run together, N
at a time, each agent in its own git worktree off a shared main. A finished
change goes through a merge queue: rebase onto the latest main, rerun the gate
and `hexa analyze . --grade A`, fast-forward main only if both pass. A conflict
or a failed re-gate sends the task back a round. Every step is logged to the
batch's `events.jsonl`.

Parallel runs are graded on the integrated main, so a change that passes alone
but clashes with another counts as wrong-but-green. Run the same tasks both
ways and the grader reports conflict rate, re-gate failure rate, the
wrong-but-green delta between the modes, and throughput.

## Is the rubric fit to judge anything?

```bash
python3 bench/handsoff/selftest.py
```

The self-test builds nine toy tasks, runs them both ways against
`selftest/fake_hexa.py` (a scripted stand-in: no model, no network), and checks
that every outcome comes out as built: a wrong-but-green change, a misshapen
one, a tampered lock, an escalation, a rebase conflict, a failed re-gate, and a
pair that is correct alone but clashes once integrated. A grader that cannot
report those on tasks made to produce them is not fit to grade real runs. This
is the same rule as `bench/selftest.sh`: a measure is only worth running if it
can fail.

## Notes

- Task fixtures with Rust, Go or TypeScript under `bench/handsoff/` will count
  toward this repo's own architecture grade. Add `bench/handsoff/tasks` to
  `analyze.exclude` in `.hexa/project.json` (as `bench/fixtures` is) before
  committing real ones.
- hexa's spend log is global, so in parallel mode spend and tokens are recorded
  per batch, and only rounds and time cap each agent.

Point `HEXA_BIN` at another hexa binary to run against it. Escalations call
`HANDSOFF_NOTIFY` with the path of a JSON payload (task, gate, last failing
output, rounds, spend) when the rounds, spend or time cap runs out.
