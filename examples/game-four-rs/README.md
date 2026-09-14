# Connect Four

A playable Connect Four for the terminal, written in Rust in ports-and-adapters
style.

## What it is

Two players take turns to drop a disc into one of seven columns. The disc falls
to the lowest free place. The first player to make a line of four in any
direction wins. If all forty-two places fill up with no line, the game is a
draw.

Think of a vending machine. You press a button for a column. A disc falls down
the slot and lands on top of the pile. The machine checks for a line of four.
Then it is the other player's turn. Nothing else happens.

The program holds forty-two places in memory, runs for less than a second, and
writes no files.

## How to run it

Play against the computer:

```sh
./run.sh
```

You are Red and you move first. Type a number from 1 to 7 and press Enter. Type
`q` to stop.

Watch a whole game play itself:

```sh
./run.sh --demo --seed 7
```

Check that everything still works:

```sh
./gate.sh
```

## The coordinate system

Get this wrong and every bug looks like a different bug. So it is written down
once, here, and the code obeys it everywhere.

- Column 0 is the left column. Column 6 is the right column.
- Row 0 is the floor. Row 5 is the top.
- Gravity pulls a disc toward row 0.
- The screen prints row 5 first and row 0 last.

That last line is a flip, and it happens in the renderer only. It is the single
flip in the whole program. A board that is flipped twice looks tidy and is
upside down, so `golden_frame` in `tests/output.rs` stands guard over it.

`BoardView` is a flat list of forty-two places, **bottom row first**. The place
at column `c` and row `r` sits at index `r * 7 + c`. Index 0 is the floor of the
left column.

## The output contract

Demo mode treats standard output as a contract that a machine reads. It prints
these bytes and no others.

1. One frame after each move. There is **no** empty frame at the start.
2. A frame is six lines. The top row comes first.
3. A line is exactly seven characters from `.`, `R` and `Y`. No spaces, no
   headers, no colours.
4. Every line ends with `\n`. Never `\r\n`.
5. The last line is `RED WINS`, `YELLOW WINS` or `DRAW`.
6. The total number of lines is `6 x moves + 1`.

Interactive mode is for a person, so it adds a blank line and a `1234567` ruler.
Prompts go to standard output, because that is the human's screen. Complaints go
to standard error in both modes, so `./run.sh 2>/dev/null` is still playable and
the demo contract stays exact.

## The exit codes

| Code | Meaning |
|---|---|
| 0 | The game finished, with any of the three results, or you quit. |
| 1 | A write failed. |
| 2 | The command line was wrong. |

The exit code does **not** follow the winner. A yellow win is still exit 0.

| Command line | Result |
|---|---|
| `--demo --seed 7` | Demo, seed 7. Exit 0. |
| `--seed 7` | Interactive, seed 7. Exit 0. |
| no arguments | Interactive, seed 1. Exit 0. |
| `--demo` | `--demo needs --seed <n>`. Exit 2. |
| `--seed` with no value | `--seed needs a value`. Exit 2. |
| `--seed abc`, `--seed -1`, `--seed 0x10` | `--seed needs a decimal number`. Exit 2. |
| anything else | `usage: connect-four [--demo] [--seed <n>]`. Exit 2. |

## The seed promise

The computer player is a small arithmetic generator called SplitMix64. It holds
its own number, and it reads no clock, no environment variable and no file. Give
it the same seed and it produces the same numbers, on any machine, in any order
of runs.

In **demo mode** one generator plays both colours, so the seed fixes the whole
game. Seed 7 today is seed 7 next year.

In **interactive mode** the generator only draws a number on its own turn, so
the game depends on your moves as well. The same seed will not repeat the same
game there. That is not a bug; it is what "the human is in the loop" means.

Here is the honest part. There are 2^64 seeds and far fewer Connect Four games,
so two seeds must play the same game somewhere. This is a **measurement**, not a
promise: 1000 seeds gave 1000 different games, with zero collisions. The test
`distinct_seeds_distinct_games` re-measures it on every run.

The computer player has no tactics. It does not try to win and it does not try
to block. A tactic would have to try out moves, which means reaching into the
rules, and an adapter is not allowed to do that. Tactics would also squeeze out
the variety, so many seeds would play one short game.

## How the code is laid out

```
src/domain/       the board and the rules; imports nothing else
src/ports/        the two plugs: a renderer and an input source
src/usecases/     one turn: ask, drop, draw
src/adapters/     the real screen, the real keyboard, the seeded chooser
src/lib.rs        the composition root; the only file that names an adapter
src/main.rs       reads the command line and sets the exit code
```

Only two plugs exist. Every adapter imports `ports` and nothing else, not even
another adapter, which is why the two renderers each build their own text.

## The tests

`cargo test` runs 23 tests. Two of them are **independent oracles**: they answer
the same question by a different road, so they cannot repeat one
misunderstanding.

- `oracle_agrees` plays 10 000 random games. The game walks out from the new
  disc; the oracle scans all 69 four-in-a-row windows with no cleverness. They
  must agree after every single move.
- `announcement_is_true` runs the real binary for 100 seeds, reads the last
  board off the screen, and works the winner out again from those forty-two
  characters. This is the test that catches a program that always prints `DRAW`.

The draw case gets a frozen 42-move transcript, found once by search, because
two random players almost never draw. `win_beats_draw_on_move_42` holds the line
that the last disc is still allowed to win.
