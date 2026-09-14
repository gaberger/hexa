# ADR-2609140020: A grade is a claim about files that were read

**Status:** Accepted
**Date:** 2026-09-14
**Drivers:** Pointed at a twenty-seven file Python project, hexa 26.9.9 scanned it, parsed none of it, and awarded **A+ — 100/100**. Pointed eight times at one unchanged tangled tree, it reported 2, 3, 4 and 5 cycles.

## Context

Three defects, one root: the grade was computed and printed without carrying
what it was computed from.

### Nothing parsed, top marks

hexa has grammars for Rust, Go and TypeScript. Given a tree in any other
language the scan finds files, the parser reads none, and the import graph is
empty. Every boundary claim is then true of nothing, and the release says so in
the shape of evidence:

```
    ✓ Analysed 0 files, 0 import edges
    ✓ 0 boundary violations
  ⬡ Architecture grade: A+ — score 100/100
```

`--grade A` passes on that. A CI job that gates on the architecture grade of a
Python, Java or Ruby repository has been green since the flag existed, and green
means the parser never opened the files.

### The cycle count was a random variable

`detect_cycles` started a depth-first walk from `HashMap::keys()` and took
neighbours out of a `HashSet`. Rust seeds both randomly per process, so the
answer depended on which node the walk happened to start from. Eight consecutive
runs of the release over one unchanged fixture:

| cycles reported | runs |
|---|---|
| 2 | 3 |
| 3 | 1 |
| 4 | 1 |
| 5 | 3 |

The score subtracts 15 points per cycle, so the same tree scored anywhere across
a 45-point span. **A grade that is a random variable is not a gate.**

It was wrong in kind as well as in order. `a → b → a` and `b → c → b` was
reported as two cycles, and which two depended on where the walk began. All
three modules reach each other; breaking one edge does not separate them. That
is one strongly connected component, not two cycles.

### The most expensive deduction named nothing

Boundary violations, dead exports and unused ports were each printed as a list a
reader can go and look at. Cycles — at 15 points, the costliest single item —
were printed as a bare number, under a footer promising that every item names
its fix. In `--json`, `circular_deps` was not serialized at all, and
`boundary_violations` came back as a count from a different pass than the
authoritative one: the import scan found 4 where the graded pass found 5, and a
consumer had no way to reconcile them.

## Decision

1. **Cycles are the strongly connected components of the module graph**, each
   returned as a sorted list of module keys, the components themselves sorted.
   The answer comes from the graph, not from the order the edges arrived in.
2. **A tree that parses to nothing is not graded.** When files were scanned and
   none were parsed, hexa prints `NOT GRADED — N files scanned, 0 parsed` and
   says why, withholds the grade line, withholds `0 boundary violations`, and
   leaves the score unset so `--grade` fails rather than comparing a floor
   against a number nothing produced. The scan count is marked with a warning
   glyph rather than a tick.
3. **Every input to the grade carries its items, not a count.** Cycles are
   printed like the other three, up to eight with a tail count, and `--json`
   carries `circular_deps` and a `boundary_violations` array taken from the
   graded pass.

## Consequences

- A repository in a language hexa cannot parse loses a grade it should never
  have had. Any CI gating `--grade` on such a tree turns red, correctly, and the
  message says the parser read nothing.
- Grades on trees that do parse are unchanged, except where cycles were
  miscounted. hexa's own grade is A+ 99/100 on four consecutive runs before and
  after; blacksheep's is A+ 100/100 over 82 files and 658 edges, 0 cycles.
- The tangled fixture goes from "2, 3, 4 or 5 cycles" to one, stable over eight
  runs — and a component of three, which is what it is.
- `hexa analyze --json` gains two arrays. A consumer reading only the counts is
  unaffected.
- Withholding a grade is a refusal, and a refusal can be wrong: a tree whose
  files all fail to parse for some *other* reason — an unreadable directory, a
  grammar that broke — is now told it was not graded rather than told it scored
  100. That is the right way round, and it is still a message about parsing
  rather than about the cause.

## Implementation

- `hexa-analysis/src/cycle_detector.rs` — strongly connected components, sorted
  output, and the module-key mapping unchanged.
- `hexa-cli/src/commands/analyze.rs` — `ScoreItems` carries cycles as items;
  `parsed_files` drives the withheld grade; `run_json` serializes
  `circular_deps` and `boundary_violations`.
- `hexa-cli/tests/a_grade_needs_evidence.rs` — the gate below.

## Gate

`cargo test -p hexa-cli --test a_grade_needs_evidence && cargo test -p hexa-analysis cycle_detector`

Three tests over a Python tree and a Rust tree built in a scratch directory: the
Python tree is not graded, `--grade F` fails on it, and the Rust tree is still
graded so a total failure to analyse cannot make the first two pass. Verified
red against the previous `analyze.rs` — two failed, the control passed. Beside
them, the detector's own tests pin that interlocking loops are one component,
that every rotation and the reversal of an edge list give an identical answer,
and that separate knots come back sorted.

## References

- ADR-2609132122 — a recorded gate is run, or it is prose; this ADR records one.
- ADR-2609121400 — module keys per language, which decides what a cycle is
  between.
- ADR-2609132158 — where the gates run, which is how this reaches `main` proven.
