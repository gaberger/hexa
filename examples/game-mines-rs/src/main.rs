//! The program. It holds no wiring, and names no adapter.
//!
//! Everything the binary does lives in `minesweeper::run`, inside `lib.rs`.
//! That is the composition root, and it is the only file that plugs an adapter
//! into a use case.

fn main() {
    std::process::exit(minesweeper::run(std::env::args().skip(1)));
}
