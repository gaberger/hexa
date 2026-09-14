# ADR-2609140926: the arena decides by gate, not by taste

**Status:** Accepted
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** A comparison against Cursor's `pstack`, whose `/arena` runs N parallel attempts and takes the best parts of each. `hexa build` diverges on the design and then builds exactly once, so the step with the most variance in it is sampled a single time.

## Context

`hexa build` proposes N designs, red-teams each, synthesizes one, and builds
to the gate. The divergence stops at the design boundary. Everything after it
— the part that produces the code the gate actually judges — happens once.

That is backwards relative to where the variance is. A design is a paragraph;
two competent designs for the same challenge differ less than two
implementations of the same design. The build step is where a model's run-to-
run spread shows up, and it is the step hexa samples once.

`pstack` runs N attempts in parallel and then takes the best parts of each.
That works for them because a human reads the attempts. It is the wrong
ending for hexa, and for a specific reason: a composite assembled from three
attempts is a fourth artefact that no gate ever ran. The parts were each
proven inside a whole that no longer exists. That is the mirror-test failure
wearing a third hat — proof transferred from the thing that was measured to a
thing that was not.

hexa does not need taste here. It has two gates. "Best" is not a judgment
call when a command exits 0 or does not, and a graph property falls out as a
number.

The isolation already exists. `hexa dev worktree` gives each entrant its own
tree. Nothing fans out into them.

## Decision

1. **`hexa arena '<challenge>' --target <dir> --gate '<cmd>' --entrants N`
   builds N implementations of the same challenge, in parallel, each in its
   own worktree.** The challenge and the gate are identical for every
   entrant. Only the run differs.

2. **The gate eliminates.** An entrant whose gate does not exit 0 is out.
   There is no partial credit, no "closest attempt", and no repair pass that
   promotes a loser. An entrant that needed help did not win.

3. **Among survivors, the grade ranks.** `hexa analyze` scores each surviving
   worktree. Ties break on the smallest diff — the change that solves the
   problem with least added surface wins, which is the rule hexa already
   applies everywhere else.

4. **One winner is merged into the target. Whole.** No cherry-picking across
   entrants in this version. A composite is an artefact the gate never
   judged, and shipping it would mean the arena's own output is the one thing
   in the run with no evidence behind it.

5. **Zero survivors is a reported result, not a rescue.** The command says
   that no entrant held the gate, merges nothing, and exits non-zero. A gate
   that eliminated everybody is information about the gate or the challenge,
   and the operator needs to see it, not a best-effort merge.

6. **Losing worktrees are kept until the operator prunes them.** The
   comparison must be auditable after the fact. `hexa dev worktree cleanup`
   already removes them on request.

## Consequences

Cost multiplies by N. The arena is for the change that matters, not the
default path, and `hexa build` stays the default. The ADR does not make
`hexa build` call the arena.

Decision 4 gives up something real. `pstack`'s composite can be better than
any single entrant, and hexa will not produce it. That is the trade the two
gates buy: everything hexa ships has run, whole, against the command that
judges it. If cherry-picking is wanted later, it needs its own ADR and its own
answer to "what gate ran on the composite".

Wall-clock does not multiply, because the entrants are parallel and isolated.
Disk does. N worktrees of a large repository is the practical ceiling on N,
not the inference budget.

A tie broken on diff size will sometimes pick the less elegant entrant. That
is acceptable and deliberate: elegance is exactly the judgment call this ADR
refuses to make.

No new port. One verb, the existing worktree machinery, and the analyzer.

## Implementation

The gate, written before the code:

```
hexa arena 'a function that returns the nth Fibonacci number, with tests' \
  --target "$(mktemp -d)" --gate 'cargo test' --entrants 2
```

It must exit 0, name a winner, and leave the target holding code that passes
the gate on its own.

A second gate covers decision 5, and it is the one that matters: an arena run
whose gate can never pass — `--gate 'exit 1'` — must exit non-zero, merge
nothing, and leave the target unchanged. A vacuous arena (`--entrants 0`, or
a gate that runs zero tests) is refused up front by `evidence_is_vacuous`.

Phases:

1. Refuse the invalid run first: entrants below 2, a vacuous gate, a target
   that is not empty and not a repository.
2. Fan out into worktrees; collect gate verdicts. No ranking yet — prove the
   elimination works.
3. Rank survivors by grade, then by diff size.
4. Merge the winner whole; report every entrant's verdict.
5. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

Order this after ADR-2609140927: the swarm's fan-out and worktree lifecycle is
the same machinery, and building it twice is how the two drift apart.

## References

- ADR-2609121400 — hexa is a scaffolding system with two gates. Decisions 2
  and 3 are those two gates used as a ranking function.
- ADR-2609140927 — swarm fans out by file boundary or refuses. Shares the
  fan-out; build that first.
- `hexa-cli/src/commands/build.rs` — the diverge/red-team/synthesize path this
  extends past the design boundary.
- `hexa-cli/src/commands/worktree.rs` — the isolation the arena needs.
- CLAUDE.md, "Tests can mirror bugs" — decision 4 is the same lesson applied
  to a composite artefact.
- `cursor/plugins` `pstack`, `/arena`, MIT —
  https://github.com/cursor/plugins/tree/main/pstack
