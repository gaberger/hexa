# ADR-2609141030: the grade counts a copy as a caller

**Status:** Proposed
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** While building `hexa arena`, this repository graded A+ 100/100 with two entrant worktrees present and A+ 96/100 without them, on byte-identical `hexa-infer` code. The analyser walked into the nested checkouts, and four real dead exports stopped being reported.

## Context

`hexa analyze` is the second of the two gates. Its number is the one
`hexa scaffold --grade A` refuses a build below, and the one this project
quotes about itself. It can be raised by putting a copy of the repository
inside the repository.

The measurement, taken minutes apart on the same working tree:

| tree | files analysed | import edges | dead exports | grade |
|---|---|---|---|---|
| with two entrant worktrees under `.hexa/arena/` | 448 | 2808 | 0 | A+ 100 |
| the same tree, entrants absent | 149 | 934 | 4 | A+ 96 |

`hexa-infer/src/registry.rs` is identical in both. Its four exported functions
— `load_from`, `upsert_in`, `remove_in`, `save_to` — are called only from
inside their own module, which is what makes them dead exports. With the
entrant worktrees present, the copies of `registry.rs` inside them are read as
source, and the copies' internal calls are counted as callers of the original.
The finding disappears.

The direction is what makes this serious. A gate that degrades silently is
worse than no gate; a gate that degrades *upward* is worse still, because
nobody investigates a better score. The only reason this was caught is that the
same tree was measured twice within an hour for an unrelated reason.

`hexa do` and `hexa bench` already fork their isolated worktrees to
`../.hexa-autoruns/`, outside the repository, so neither has ever triggered
this. `hexa arena` forked into `.hexa/arena/` and did. That is now fixed in
`arena.rs`, and it is a fix to the caller, not to the defect: any nested
checkout, vendored copy, or `git worktree add` inside the tree does the same
thing to the number.

`.hexa/*` is in `.gitignore`, and the analyser walked into it anyway. Whatever
it uses to decide what counts as source, it is not the ignore rules.

## Decision

1. **The analyser never descends into a nested git worktree or repository.** A
   directory containing `.git`, or listed by `git worktree list`, is not part
   of the tree being graded. It is a different tree that happens to be stored
   here.

2. **The analyser respects `.gitignore`.** If git would not track it, it is not
   this project's source. The ignore rules are already the project's own
   statement about what its code is.

3. **The scan reports its own size, and a change in it is visible.** Files
   scanned and import edges are already printed. They become part of the JSON
   output too, so a run can be compared with the previous one and a threefold
   jump is something a person or a test can notice.

4. **A grade is reported alongside what it was computed over.** A number that
   can move by 4 points depending on what is lying about in the working tree
   must say how many files it read. "A+ 96 over 149 files" is a claim. "A+ 96"
   is a rumour.

## Consequences

Grades will go down in trees that currently contain nested checkouts. That is
the correction, and the first such drop is this repository's own.

Decision 2 changes what gets analysed in projects that gitignore generated
source. That is the right answer — generated code the project does not track is
not the project's architecture — but it is a behaviour change for anyone whose
grade currently includes it.

Decision 1 costs a check per directory. Decision 2 costs reading the ignore
rules, which the graph builder may already do; if it does, the two should share
one implementation rather than disagreeing.

This does not touch the boundary checker, which reads imports rather than
walking the tree. It should be checked against the same defect, because if it
also walks, a copy inside the tree can hide a violation the same way it hid a
dead export.

## Implementation

The gate, written before the code, and it is a differential:

```
# the same tree, graded with and without a nested checkout, must score the same
git worktree add --detach /tmp/nested HEAD && mv /tmp/nested ./nested-copy
hexa analyze . --json | jq .score      # must equal the score without it
rm -rf ./nested-copy
```

A test builds that pair in a temporary repository: grade it, add a nested
worktree, grade it again, and assert both the score and the file count are
unchanged. The file count is the half that fails loudly when decision 1
regresses; the score can coincide.

Vacuity guard: the fixture must contain at least one real dead export, or both
runs score 100 and the test proves nothing.

Phases:

1. The differential test, failing. It is the decision.
2. Skip nested worktrees and repositories during the walk.
3. Apply the ignore rules.
4. Put `files_analysed` and `import_edges` in the JSON, and print them next to
   the grade.
5. Check the boundary checker for the same defect, and say either way.
6. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

## References

- ADR-2609121400 — hexa is a scaffolding system with two gates. This is one of
  the two gates, measuring the wrong tree.
- ADR-2609122048 — a tool that reports "nothing found" must prove it looked.
  The same family: four findings vanished and the report said nothing had.
- ADR-2609140926 — the arena decides by gate, not by taste. Where this was
  found, and whose worktree location is fixed as its caller-side half.
- `hexa-cli/src/commands/arena.rs` — `arena_root`, and the test that entrant
  worktrees never live inside the repository.
- CLAUDE.md, "A gate that degrades silently is worse than no gate."
