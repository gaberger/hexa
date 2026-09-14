# game-2048-ts

2048, in TypeScript, in ports-and-adapters style.

## Play it

```bash
./run.sh                    # w/a/s/d to move, Ctrl-D to quit
./run.sh --demo             # play a fixed game and exit
```

`--demo` cycles left, up, right, down until the board is stuck. The moves are
fixed rather than random, because a demo that plays differently each run cannot
be part of a gate.

## What is here

| Directory | Holds |
|---|---|
| `src/domain/` | the grid, the slide, the merge, the score. No I/O. |
| `src/ports/` | what the game needs from outside: a board store, a random source. |
| `src/adapters/` | the implementations of those ports. |
| `src/usecases/` | start a game, play a move, read a game. |
| `src/composition-root.ts` | the only file that names an adapter. |
| `src/main.ts` | the only file that reads a key or prints a board. |

The luck lives in the game state as a seed, so a replay is exact.

## Test it

```bash
bun test        # or: npm test
```
