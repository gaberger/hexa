# game-wordle-py — a Python stress fixture for `hexa analyze`

A working game of Wordle, written in Python, laid out in the hexagonal
structure hexa expects. **This is a stress fixture, not a showcase.** It exists
to show what `hexa analyze` reports on a tree it cannot read.

## The game

Five-letter answer, six attempts. Each guess is scored position by position:
green for the right letter in the right place, yellow for a letter that is in
the answer somewhere else, grey for absent. Duplicate letters follow the real
Wordle rule — a guess letter earns yellow only if an *unmatched* copy of it
still remains in the answer after every green has claimed its own copy. That
rule lives in `src/domain/scoring.py` and is tested directly in
`tests/test_scoring.py`, including against an independent oracle over all 6561
four-letter pairs of a three-letter alphabet.

Run it:

```
cd examples/stress/game-wordle-py
python3 -m src.composition_root        # optional integer argument seeds the answer
```

Test it:

```
python3 -m pytest -q                   # 59 passed
python3 -m unittest discover -s tests -t .   # same 59, no third-party dependency
```

There are no third-party runtime dependencies. pytest is optional; the suite is
written as `unittest.TestCase` classes so it runs under either runner.

## Layout

```
src/domain/                  scoring, word validity, the Game entity
src/ports/                   typing.Protocol contracts: WordSource, GameStore
src/usecases/                start_game, submit_guess
src/adapters/primary/        console front end
src/adapters/secondary/      in-memory store, wordlist word source
src/composition_root.py      the single wiring point
```

## The two deliberate violations

This tree contains **exactly two** real hexagonal architecture violations. Both
are marked in the source with a `# STRESS: violation` comment.

| # | File | Line-level marker | Rule broken |
|---|---|---|---|
| 1 | `src/domain/game.py` | `from src.ports.ids import GameId` | **domain/ imports only from domain/.** The domain reaches outward into the port package for its identity type. |
| 2 | `src/usecases/start_game.py` | `from src.adapters.secondary.memory_store import InMemoryGameStore` | **usecases/ imports from domain/ and ports/ only.** The use case is bound to a concrete secondary adapter, not to the `GameStore` port. |

Both are live code, not comments: `GameId` really is the domain entity's id
type, and `InMemoryGameStore` really is the fallback store `start_game` uses
when no store is passed. The game runs and all 59 tests pass with the
violations in place. Removing them would change behaviour.

A tree of this shape written in TypeScript, Go or Rust would be reported by
`hexa analyze` as two boundary violations and graded down accordingly
(−10 points each, so 80/100, a B).

## What hexa actually reports

Run on 2026-09-13 from the hexa repository root, against
`hexa-analysis` as built in `target/release`:

```
$ ./target/release/hexa analyze examples/stress/game-wordle-py --grade A+ --exit-code
⬡ Architecture analysis: /home/gary/projects/hexa/examples/stress/game-wordle-py

  Project:
    language:     unsupported (Python; hexa grades Rust, Go and TypeScript)
    ✗ .hexa/ config
    docs/adrs/:   none

  Hex layers:
    ✓ Domain (4 files)
    ✓ Ports (4 files)
    ✓ Use Cases (3 files)
    ✓ Primary Adapters (2 files)
    ✓ Secondary Adapters (3 files)
    ✓ Composition root (detected)

  Boundary analysis:
    ‣ 19 source files scanned
    ✓ Analysed 0 files, 0 import edges
    ✓ 0 boundary violations

  ⬡ Architecture grade: A+ — score 100/100
    violations 0 · cycles 0 · dead exports 0 · unused ports 0
    score = 100 − 10·violations − 15·cycles − dead exports (max 20) − unused ports (max 10)
    A+ 95–100 · A 90–94 · B 80–89 · C 70–79 · D 60–69 · F below 60

  Architectural health:
    read next to the grade; none of these move the score
    ✓ cohesion           0
    ✓ duplication        0
    ✓ god types          0
    ✓ dead layers        0
    ✓ orphans            0

  ADR compliance:
    ○ ADR rules NOT CHECKED — no .hexa/ADR-rules.toml — run `hexa init` to write the shipped rule set
exit=0
```

## The finding

**A+ with `--exit-code 0` is the failure, not the pass.**

Two real violations sit in this tree and hexa reports zero. The reason is in
`hexa-analysis/src/domain.rs`: `Language::from_path` knows `.ts/.tsx/.js/.jsx`,
`.go` and `.rs`, and maps everything else to `Language::Unknown`. The
tree-sitter adapter then returns no imports for an unknown language, so the
boundary checker in `hexa-analysis/src/boundary_checker.rs` has no edges to
check — and "no edges checked" is printed in the same shape as "no violations
found".

The report is honest about two of the three facts and silent about the third:

- It **does** say `language: unsupported (Python; hexa grades Rust, Go and TypeScript)`.
- It **does** say `Analysed 0 files, 0 import edges`, next to `19 source files scanned`.
- It **still** awards `A+ — score 100/100`, still prints `✓ 0 boundary violations`
  with a green tick, and still **exits 0 against `--grade A+`**.

So anything that consumes the exit code — a CI gate, a pre-merge hook, an agent
checking its own work — reads a pass. The grade is computed from an empty edge
set and presented identically to a grade computed from a complete one. That is
silent degradation: *a gate that degrades silently is worse than no gate.* The
claim on offer is "no boundary violations"; the claim actually earned is "no
files were parsed".

The layer counts make it worse, not better: `✓ Domain (4 files)`,
`✓ Ports (4 files)`, `✓ Composition root (detected)` all pass, because they are
directory-shape checks that need no parser. A reader skimming the report sees
six green layer ticks and a green boundary line and concludes the architecture
was verified. Only the `0 import edges` line says otherwise, and it is not the
line the exit code comes from.

### What would fix it

Any of these would turn the silent pass into a loud failure:

1. Refuse to grade a tree whose files are all `Language::Unknown` — exit
   non-zero against `--grade`, the way an unreadable input should.
2. Report the boundary line as *not checked* rather than *0 violations* when
   `analysed files == 0` while `scanned files > 0`, and never mark it `✓`.
3. Add a Python parser, at which point this fixture should report exactly two
   violations and a B. That is this fixture's regression assertion.

Until one of those lands, this directory is the reproduction.
