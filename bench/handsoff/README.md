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

Point `HEXA_BIN` at another hexa binary to run against it. Escalations call
`HANDSOFF_NOTIFY` with the path of a JSON payload (task, gate, last failing
output, rounds, spend) when the rounds, spend or time cap runs out.
