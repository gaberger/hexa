# ADR-2609140927: swarm fans out by file boundary, or refuses

**Status:** Proposed
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** CLAUDE.md has carried the rule "parallelize by file boundary, serialize by file overlap" since the adversarial review that produced it. Nothing in the binary implements it, checks it, or can even compute it. A lesson with no verb behind it is a lesson you will break.

## Context

hexa has the isolation and none of the fan-out. `hexa dev worktree` creates
and cleans worktrees. `hexa do run` drives one task to one gate. Between them
there is no verb that says "this task, across these twenty files, N workers,
one report" — so the work is done in series, or it is done in parallel by
hand, which is where the overlap rule gets broken.

The rule matters because of how it fails. Two agents editing disjoint files
finish and merge cleanly. Two agents editing the same file both succeed
against their own gate, and then one overwrites the other, and the gate that
proved each of them was measuring a state that no longer exists. Nothing
errors. The evidence survives the change it was evidence for. That failure is
silent, which is the class of failure this project exists to close.

The rule is also mechanically checkable, which is why leaving it as prose is
the wrong choice. A slice is a set of files. Two slices overlap or they do
not. It is a set intersection, computed before any worker starts.

`pstack`'s `/swarm` runs N workers across slices or races and returns one
aggregated report. It does not compute overlap, because it cannot: nothing in
that system knows what a worker will touch until it touches it. hexa's graph
does know, and `hexa graph consumers` already answers the harder version of
the same question.

## Decision

1. **`hexa swarm '<task>' --over <slices> --gate '<cmd>'` runs one worker per
   slice, each in its own worktree.** A slice is a path, a glob, or a line of
   a file list. The task and the gate are the same for every worker; the
   slice is what differs.

2. **Overlap is computed before any worker starts, and overlap refuses the
   run.** Slices are expanded to file sets and intersected. A non-empty
   intersection names the shared files and exits non-zero, having started
   nothing. This is the rule from CLAUDE.md, made mechanical.

3. **A worker's result is its gate verdict, and nothing else.** Not the
   agent's self-report, not "looks done". The gate runs inside the worker's
   worktree, on that worker's slice.

4. **A failed worker does not block the others, and is never dropped.** The
   report has one line per slice, including the ones that failed and the ones
   that produced no change. A slice missing from the report is a defect in
   the report.

5. **Nothing merges automatically.** The swarm reports branches. The operator
   merges, or runs `hexa harden` over the result first. Twenty parallel
   agents landing on a branch unattended is not a feature.

6. **A swarm of one slice is a `hexa do run` with extra machinery, and says
   so.** It runs, and it tells the operator the simpler verb exists.

## Consequences

Decision 2 will refuse runs that would have worked. Two slices can share a
file that neither worker would have edited, and the overlap check cannot know
that, so it refuses. That is the correct direction to be wrong in: the cost is
a rejected run the operator can re-slice, against a silent overwrite that
destroys evidence.

Glob expansion becomes load-bearing. A glob that expands differently on two
platforms produces a different overlap verdict, so expansion happens inside
hexa, over the git file list, and never in the shell.

N worktrees of a large repository is the disk ceiling, the same as the arena's.

The report is the product. A swarm whose report is incomplete is worse than
no swarm, because the operator will believe the slices it does not mention
were fine. Decision 4 is therefore a test, not a convention.

No new port. One verb over the existing worktree machinery, the git file
list, and the agent loop.

## Implementation

The gate, written before the code, in two halves that must both hold:

```
# fans out and reports every slice
hexa swarm 'add a module doc comment' --over 'hexa-git/src' 'hexa-parser/src' \
  --gate 'cargo check -p hexa-git -p hexa-parser'

# and refuses overlap, before starting anything
hexa swarm 'add a module doc comment' --over 'hexa-cli/src' 'hexa-cli/src/commands' \
  --gate 'cargo check -p hexa-cli'   # must exit non-zero, name the shared files,
                                     # and create no worktree
```

The refusal half is the real gate. A swarm that fans out is easy; a swarm
that refuses to is the decision. The test asserts no worktree was created,
not merely that the exit code was non-zero.

Vacuity guard: a swarm over zero slices, or over slices that expand to zero
files, is refused rather than reported as "0 workers, all passed".

Phases:

1. Slice expansion over the git file list, and the overlap check. Ship this
   alone — it is useful on its own and it is the decision.
2. Fan out into worktrees, one worker per slice, gate each.
3. The aggregated report, with the completeness test from decision 4.
4. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

This lands before ADR-2609140926. The arena reuses phases 2 and 3 wholesale.

## References

- CLAUDE.md, "Parallelize by file boundary, serialize by file overlap" — the
  lesson this ADR gives a verb to.
- ADR-2609140926 — the arena decides by gate, not by taste. Reuses the
  fan-out built here.
- `hexa-cli/src/commands/worktree.rs` — the isolation.
- `hexa-cli/src/commands/direct.rs` — `hexa do run`, which one worker is.
- `hexa graph consumers` — the harder version of the same overlap question,
  already answered for deletion safety.
- `cursor/plugins` `pstack`, `/swarm`, MIT —
  https://github.com/cursor/plugins/tree/main/pstack
