# ADR-2609140928: a run leaves a decision trail behind it

**Status:** Accepted
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** hexa records decisions between tasks and lessons after them. Inside one long run it records nothing, so a run that goes wrong cannot be traced to the step where it went wrong. A comparison against Cursor's `pstack`, whose `/show-me-your-work` logs decisions to a file you commit.

## Context

hexa's memory has a hole in the middle.

Before a task, an ADR records the decision. After a task, `hexa memory store
lesson:<topic>` records what was learned. Between them, inside a single
`hexa build`, `hexa harden`, or a twelve-step `hexa do run`, the agent makes
dozens of choices — which file to read, which hypothesis to test, which fix to
try, which one to abandon — and every one of them is gone when the process
exits.

That is fine while the run succeeds. When it fails, or worse, when it
succeeds for the wrong reason, there is nothing to read. The commit shows the
end state. The gate verdict shows pass or fail. Neither shows the step where
the run turned, and the operator's only recourse is to run it again and watch,
which is a different run.

`hexa spend` proves the appetite for this is already there: it exists because
"what did that cost" was a question nobody could answer after the fact, and
the answer was to write the fact down as it happened. This is the same move
applied to reasoning rather than tokens.

The same-model problem makes the reconstruction alternative worse than
useless. A summary written at the end of a run, by the model that made the
choices, encodes the model's account of its own reasoning. It is the
mirror-test failure applied to a post-mortem. The trail must be appended as
each decision is made, or it is not evidence.

## Decision

1. **Every agent loop appends a trail as it runs.** One row per decision,
   written when the decision is made. No end-of-run summarisation, ever.

2. **The trail is a file in the repository.** `docs/trails/<run-id>.tsv`, so
   it diffs, commits, greps, and survives the process. Tab-separated because
   the fields are short and the file is read by both people and `cut`.

3. **A row carries five fields: when, which step, what was chosen, what it
   was chosen over, and the evidence.** The fourth field is what makes it a
   decision rather than a log line. A step with no alternative was not a
   choice, and records none.

4. **A row with an empty evidence field is a defect, not a row.** The loop
   refuses to write it. A trail whose evidence column is empty throughout
   documents nothing and would let a run claim it was traced when it was not
   — ADR-2609122048's rule, applied to hexa's own reasoning.

5. **`hexa trail show <run-id>` reads one back. `hexa trail list` lists the
   runs.** A trail nothing can read is a file, not a record.

6. **The trail never gates anything.** It is evidence for a person. Making it
   a gate would give the loop a reason to write rows that pass the gate.

## Consequences

Every run now writes to the repository, which is a real change in behaviour
for a tool that has been careful about what it touches. `docs/trails/` is
therefore append-only, one file per run, and carries its own `.gitignore`
decision: committed when the operator wants the trail in history, ignored by
default is *not* offered, because a trail that is usually absent is a trail
nobody trusts.

Trail files accumulate. Pruning is the operator's, via a verb, and not
automatic — a tool that deletes its own evidence to save disk is the wrong
shape.

Decision 4 will make some loops harder to write. A step that cannot name its
evidence has to either find some or admit it was not a decision. That
friction is the feature; it is the same friction `hexa verify` now applies to
verdicts.

Decision 1 costs a file append per step. Against inference latency this is
free.

This ADR is what makes ADR-2609140929 possible: a playbook learned from
recorded runs needs runs that recorded something.

## Implementation

The gate, written before the code:

```
hexa do run "<a small task>" --file <f> --evidence '<cmd>'
test -f docs/trails/*.tsv                               # a trail exists
awk -F'\t' 'NF!=5 {exit 1}' docs/trails/*.tsv           # every row has five fields
awk -F'\t' '$5=="" {exit 1}' docs/trails/*.tsv          # no empty evidence
hexa trail list | grep -q .                             # and it reads back
```

The vacuity guard is the one that matters here, and it is easy to get wrong:
a run that wrote zero rows passes all four commands above. The test asserts
a row count at least equal to the number of loop steps the run reported, so
"traced nothing" cannot pass as "traced perfectly".

Phases:

1. The row type and the append, behind the loop's existing step boundary.
   Refuse the empty-evidence row here, at the write.
2. `hexa trail list` and `hexa trail show`.
3. The row-count test against reported loop steps.
4. Wire the remaining loops — `build`, `harden`, `scaffold` — to the same
   append. One shared writer, not four.
5. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

## References

- ADR-2609122048 — a tool that reports "nothing found" must prove it looked.
  Decision 4 is that rule turned on hexa's own reasoning.
- ADR-2609140929 — playbooks are learned from runs, not guessed. Depends on
  this; build this first.
- `hexa-cli/src/commands/spend_cmd.rs` — the same move, for cost.
- `hexa-exec/src/simple_agent.rs` — the ReAct loop whose steps are currently
  lost at process exit.
- CLAUDE.md, "Tests can mirror bugs" — why decision 1 forbids end-of-run
  summarisation.
- `cursor/plugins` `pstack`, `/show-me-your-work`, MIT —
  https://github.com/cursor/plugins/tree/main/pstack
