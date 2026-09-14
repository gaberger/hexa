# Minesweeper

A playable Minesweeper for the terminal, in Rust, in ports-and-adapters style.
No dependencies. No save file. No raw terminal mode.

## Start it

```bash
./run.sh                       # play
./run.sh --demo --seed 42      # watch it play itself
```

`run.sh` always builds first. Cargo skips the work when nothing changed.

## How to play

The board is a grid of hidden cells. Some of them hold a mine. Open a cell and
it shows how many mines touch it. Open every cell that holds no mine and you
win. Open a mine and the game is over.

Type one command per line:

| Command | What it does |
|---|---|
| `r <col> <row>` | Open that cell |
| `f <col> <row>` | Put a flag on, or take a flag off |
| `q` | Stop playing |

Columns and rows start at `0`, and `0 0` is the top left cell. The column
numbers are printed above the grid and the row number beside each row.

`f` is a toggle. Type it once to put a flag down, and again to take it off. A
flag only stops you from opening a cell by mistake. Flags never decide a win.

Your first click is always safe. The mines are placed after you click, around
the cell you chose and around its neighbours.

The grid uses plain ASCII:

| Sign | Meaning |
|---|---|
| `.` | hidden |
| `F` | your flag |
| (space) | open, no mine touches it |
| `1`-`8` | open, that many mines touch it |
| `*` | a mine, shown after you lose |
| `X` | the mine you stepped on |

## Options

| Flag | Meaning | Default |
|---|---|---|
| `--demo` | Play a scripted game and print only the result | off |
| `--seed <n>` | Set the seed. Works with and without `--demo` | the clock |
| `--policy <deduce\|reckless>` | The demo player to use | `deduce` |
| `--width <n>` | Board width | 9 |
| `--height <n>` | Board height | 9 |
| `--mines <n>` | Number of mines | 10 |
| `-h`, `--help` | Show the options | |

Two limits apply, and `--help` prints both. A board may hold at most
**1,000,000** cells. The number of mines may be at most `cells - 9`, so the
first click and its eight neighbours always have room to be safe. Ask for more
than either and the program says so and leaves with code 2.

`deduce` plays by two safe rules and guesses when neither one fires. It sees
only what you see, so it can lose. `reckless` opens cells in order, so with one
mine or more it always loses. The demo needs no terminal, so a script can run it.

## What a run prints

The last three lines of any run are always these three, in this order:

```
BOARD 1f3a9c04d7b2e185
STATS revealed=63 flags=10
YOU WIN
```

* `BOARD` is a short fingerprint of the mine positions. The same seed and the
  same first click always give the same line, and a different seed gives a
  different one.
* `STATS` counts the safe cells you opened and the flags you put down.
* The last line is exactly `YOU WIN`, `GAME OVER` or `QUIT`. Closed input in
  interactive play prints `QUIT`.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | The run finished. A loss is not an error |
| 2 | A bad option, or a board size that cannot exist |
| 3 | An internal fault: a broken promise, a stuck generator, or a demo player that ran too long |

## How it is put together

```
src/
  domain/    the board and the rules; imports nothing else
  ports/     plain data and traits; no logic
  usecases/  one turn, and the one game loop
  adapters/
    primary/    the terminal screen, the keyboard, the demo player
    secondary/  the seeded generator
  lib.rs     the composition root; the only file that names an adapter
  main.rs    calls lib.rs and does nothing else
```

Two small choices carry the whole design.

1. **The game counts safe cells, not open cells.** A mine can never add to the
   win counter, so a win is always real.
2. **The demo player is handed the same window you look through.** It receives
   a `BoardView`, and that type does not hold the mines. So a passing gate means
   the game plays, not that the program stopped.

Three more things are worth knowing.

* A message is an enum, not a string. So there is no path for text you typed to
  reach your terminal. The compiler holds that rule, not a note in a document.
* The spread over an empty region uses a stack, never recursion. A large empty
  board cannot run the program out of call stack.
* The game is single threaded on purpose. One board, one writer, no locks.

## Honest notes

* The game keeps **no save file**. Close it and the game is gone. It is still
  fully repeatable from the seed, because the board is a function of the seed
  and your first click.
* The game **never uses raw terminal mode**. It reads whole lines. So it never
  has to put your terminal back the way it was.
* `Step` has a fourth case, `Fault`, that the build spec does not list. A stuck
  generator has to fail loudly with exit code 3, and a refusal shown to the
  player would have hidden it.
* The random port has its own failure type, `RollFault`. The domain has its own
  word for the same trouble, `RollError`. They look alike on purpose: an adapter
  may name the port and may not name the domain, so the two have to be separate.
  The use case translates between them, because it is the one layer allowed to
  see both.

## Tests

```bash
cargo test     # 60 tests
./gate.sh      # the gate: the tests, both endings, the seed, and the grade
```

The suite covers geometry and the edge trap, mine placement, the spread, the
win and lose rules, the window, the demo player, the screen, the keyboard, the
flags you place before the first click, the replay, and the real binary.

`gate.sh` is the harder check. It runs the tests and counts them, so deleting
them fails it. It pins **both** endings by name, so a program that prints four
words cannot pass. It proves two seeds give two different boards. It then asks
`hexa analyze .` for the architecture grade, and accepts only `A+`.
