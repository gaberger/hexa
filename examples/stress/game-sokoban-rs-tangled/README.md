# game-sokoban-rs-tangled

A working Sokoban with a deliberately tangled architecture.

Everything here compiles. All 19 behaviour tests pass. The rules are real:
boxes do not push other boxes, a step off the grid is `None` rather than a
clamp, blocked keypresses are not recorded and do not count against par, and
a rendered board parses back into the level it came from.

The only thing wrong with it is the shape.

That is the point. This fixture exists so that **"it builds and the tests are
green" can be told apart from "the architecture is sound"**. A grader with no
failing fixture has never been tested in the direction that matters.

## The five violations

Each is marked `STRESS: violation` at the import line that causes it.

| File | Edge | Rule broken |
|---|---|---|
| `src/domain/score.rs` | → `ports/level_source` | domain must not import from ports |
| `src/ports/replay.rs` | → `usecases/play` | ports must not import from usecases |
| `src/usecases/play.rs` | → `adapters/secondary/memory_recorder` | usecases may only import from domain and ports |
| `src/adapters/primary/console.rs` | → `adapters/secondary/memory_recorder` | adapters must not import from other adapters |
| `src/adapters/secondary/memory_recorder.rs` | → `domain/position` | adapters must not import from domain directly |

There is also a deliberate module cycle between `domain/level.rs` and
`domain/push.rs`, marked `STRESS: cycle`. Rust permits it; a hexagonal
grader should still charge for it.

The last row is the one worth dwelling on. `ports/move_recorder` re-exports
`Dir` precisely so an adapter never needs to reach into the domain. Going
around it is the mistake CLAUDE.md warns about: every adapter grows a second
edge into the core.

## Gates

Two commands, and they are meant to disagree:

```sh
cargo test                                      # exits 0 — the game is correct
hexa analyze . --grade A+ --exit-code           # exits 1 — the shape is not
```

A change that makes the second one exit 0 without deleting the `STRESS`
markers has broken the fixture, not fixed the code.

## What hexa actually reports

Captured 2026-09-13 against hexa v26.9.9, `hexa analyze examples/stress/game-sokoban-rs-tangled --grade A+ --exit-code`:

```text
  Boundary analysis:
    ‣ 19 source files scanned
    ⚠ 4 boundary violation(s) (import scan)
      ✗ src/usecases/play.rs → src/adapters/secondary/memory_recorder/MemoryRecorder (usecases/ may only import from domain/ and ports/)
      ✗ src/ports/replay.rs → src/usecases/play/Progress (ports/ may only import from domain/)
      ✗ src/domain/score.rs → src/ports/level_source/Par (domain/ must only import from domain/)
      ✗ src/adapters/primary/console.rs → src/adapters/secondary/memory_recorder/MemoryRecorder (adapters must never import other adapters)
    ⚠ Boundary violations: 5
        src/adapters/primary/console.rs → src/adapters/secondary/memory_recorder (adapters must not import from other adapters)
        src/adapters/secondary/memory_recorder.rs → src/domain/position (adapters must not import from domain directly)
        src/domain/score.rs → src/ports/level_source (domain must not import from ports (use domain/value-objects))
        src/ports/replay.rs → src/usecases/play (ports must not import from usecases)
        src/usecases/play.rs → src/adapters/secondary/memory_recorder (usecases may only import from domain and ports)
    ✓ Analysed 19 files, 64 import edges

  ⬡ Architecture grade: F — score 4/100
    violations 5 · cycles 3 · dead exports 1 · unused ports 0
      dead export   src/usecases/play.rs:62 fresh_recorder

  Architectural health:
    • orphans            1
        orphan_port Replay src/ports/replay.rs:9
```

Exit code 1. All five violations found, each named with its file, its edge
and the rule it breaks. The cycle is charged. The unused `fresh_recorder`
helper is called out as a dead export, and `Replay` — a port nothing
implements — as an orphan. The grader does its job.

## What this fixture found in hexa

Two defects, on the first run.

**The two violation lists disagree.** The import-scan list says 4; the list
below it says 5. The missing one is
`src/adapters/secondary/memory_recorder.rs → src/domain/position`. Two
detectors reach different answers and the display prints both without
reconciling them. The grade uses 5, so the score is right, but a reader who
stops at the first list is one violation short. (ADR-2609140020)

**Cycles are charged but never named.** `cycles 3` costs 45 of the 96 points
lost here, and nothing says which three. Violations name their file and
edge; dead exports name their line; the footer promises "each item above
names its fix". Cycles are the one deduction a reader cannot act on. Only
one cycle was written deliberately, so two of the three are unexplained even
to the author of the fixture. (ADR-2609140020)
