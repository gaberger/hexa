//! Conway's Game of Life, playable.
//!
//! The library had every rule and no way to watch them run. `cargo test` was
//! green and there was nothing to start, which is the difference between
//! "it compiles" and "it works".
//!
//! `--demo` runs a fixed number of generations and exits, so a gate can prove
//! the thing actually ran. With no flags it steps on Enter.

use std::io::{self, BufRead, Write};

use game_life_rs::Grid;

const WIDTH: usize = 24;
const HEIGHT: usize = 14;

/// A glider, at the top left. It walks diagonally forever on an open field,
/// which makes it the clearest proof that the rules are being applied.
fn seed(g: &mut Grid) {
    for (x, y) in [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
        g.set(x, y, true);
    }
}

fn render(g: &Grid) -> String {
    let mut s = format!("generation {}\n", g.generation());
    for y in 0..g.height() {
        for x in 0..g.width() {
            s.push(if g.get(x, y) { '#' } else { '.' });
        }
        s.push('\n');
    }
    s
}

fn live_cells(g: &Grid) -> usize {
    (0..g.height()).map(|y| (0..g.width()).filter(|&x| g.get(x, y)).count()).sum()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let demo = args.iter().any(|a| a == "--demo");
    let generations: u64 = args
        .iter()
        .position(|a| a == "--generations")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(20);

    let mut grid = Grid::new(WIDTH, HEIGHT);
    seed(&mut grid);

    if demo {
        println!("{}", render(&grid));
        for _ in 0..generations {
            grid.step();
        }
        print!("{}", render(&grid));
        // The last line is what a gate reads.
        println!("DONE {} generations, {} live cells", grid.generation(), live_cells(&grid));
        return;
    }

    println!("Conway's Game of Life — press Enter to step, Ctrl-D to quit.");
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("{}", render(&grid));
        print!("[enter] ");
        let _ = io::stdout().flush();
        match lines.next() {
            Some(Ok(_)) => grid.step(),
            _ => {
                println!("\nDONE {} generations, {} live cells", grid.generation(), live_cells(&grid));
                return;
            }
        }
    }
}
