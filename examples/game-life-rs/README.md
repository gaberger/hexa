# game-life-rs

Conway's Game of Life, in Rust.

## Play it

```bash
./run.sh                    # step with Enter, Ctrl-D to quit
./run.sh --demo             # run 20 generations and exit
./run.sh --demo --generations 100
```

A glider starts at the top left and walks diagonally. After 20 generations it
is still five cells, which is what `--demo` prints on its last line.

## What is here

`src/lib.rs` is the whole rule set: a `Grid` with `get`, `set`, `step` and a
generation count. Neighbours wrap at the edges, so the field is a torus.

`src/main.rs` is the only part that reads a key or writes a character. The
rules never touch the terminal, which is why they are easy to test.

## Test it

```bash
cargo test
```
