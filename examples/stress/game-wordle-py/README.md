# game-wordle-py — a Python stress fixture for `hexa analyze`

A working game of Wordle, written in Python, laid out in the hexagonal
structure hexa expects. **This is a stress fixture, not a showcase.** It exists
to show what `hexa analyze` reports on a Python tree with two planted violations.

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

`hexa analyze` reports them as two boundary violations, −10 points each, and
the cycle the first one closes, −15 (see below).

## What hexa reports

hexa reads Python since ADR-2611211324. Run on 2026-11-21 from this directory:

```
$ hexa analyze .
  Project:
    language:     python (pyproject.toml)

  Boundary analysis:
    ‣ 19 source files scanned
    ⚠ Boundary violations: 2
        src/domain/game.py → src/ports/ids.py (domain must not import from ports (use domain/value-objects))
        src/usecases/start_game.py → src/adapters/secondary/memory_store.py (usecases may only import from domain and ports)
    ✓ Analysed 19 files, 28 import edges

  ⬡ Architecture grade: D — score 65/100
    violations 2 · cycles 1 · dead exports 0 · unused ports 0
    coverage 18/19 files in a layer — score capped at 94
      no layer      src/__init__.py
      cycle         src/domain → src/ports
```

The two violations are exactly the two planted ones, and nothing else is
reported as one. The cycle is the first planted violation seen from the other
side: `src/ports` imports the domain, as a port should, and the domain now
imports `src/ports/ids.py` back. `src/__init__.py` is the package marker; it
has no layer of its own, so the score is capped at the share of files that have
one. Removing the two violations, and declaring `src/__init__.py` in
`.hexa/project.json → analyze.layers`, is what an A+ would take.

`hexa-cli/tests/adr_2611211324_spec.rs` (S43) holds this: the example is
graded, and its violations are exactly the two planted ones.

## What hexa used to report

Before ADR-2611211324, the same run printed `language: unsupported (Python;
hexa grades Rust, Go and TypeScript)` and `Analysed 0 files, 0 import edges`,
then awarded `A+ — score 100/100` and exited 0 against `--grade A+`: a grade
computed from an empty edge set, presented like one computed from a complete
one. A later change withheld the grade (`NOT GRADED — 19 files scanned, 0
parsed`); this one reads the files.
