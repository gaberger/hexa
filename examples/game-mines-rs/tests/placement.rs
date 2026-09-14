//! B. Placement.
//!
//! A generator that does nothing still puts mines somewhere. These tests make
//! it prove that the seed does work, that the count is exact, and that a stuck
//! generator fails fast instead of hanging.

use std::collections::HashSet;

use minesweeper::adapters::secondary::{roll_below, SeededRng, DRAW_CAP};
use minesweeper::domain::{ConfigError, Coord, Dims, Layout, PlacementError, RollError, MAX_CELLS};
use minesweeper::ports::{RandomSource, RollFault};
use minesweeper::usecases::{new_game, take_command, GameConfig, Step};
use minesweeper::ports::Command;
use minesweeper::usecases::EndReason;

/// The port has its own failure type. The domain has its own word for the same
/// trouble. A real use case does this translation, and so does a test.
fn as_domain(f: RollFault) -> RollError {
    match f {
        RollFault::ZeroBound => RollError::ZeroBound,
        RollFault::Stuck => RollError::Stuck,
    }
}

fn dims(w: u16, h: u16) -> Dims {
    Dims::new(w, h).expect("dims must build")
}

fn place(seed: u64, d: Dims, mines: usize, exclude: &[Coord]) -> Layout {
    let mut rng = SeededRng::new(seed);
    Layout::place(d, mines, exclude, &mut |b| rng.next_below(b).map_err(as_domain)).expect("place must work")
}

/// 7. Exactly the number of mines asked for, over 100 fixed seeds.
#[test]
fn the_mine_count_is_exact() {
    let d = dims(9, 9);
    for seed in 0..100u64 {
        for want in [0usize, 1, 10, 40, 72] {
            let layout = place(seed, d, want, &[]);
            assert_eq!(layout.mine_count(), want, "seed {seed} wanted {want}");
        }
    }
}

/// 8. The same seed and the same first click give the same board.
#[test]
fn the_same_seed_gives_the_same_board() {
    let d = dims(9, 9);
    let first = d.coord(3, 3).expect("cell");
    let mut exclude = d.neighbours(first);
    exclude.push(first);
    for seed in 0..25u64 {
        let a = place(seed, d, 10, &exclude);
        let b = place(seed, d, 10, &exclude);
        assert_eq!(a.fingerprint(), b.fingerprint(), "seed {seed}");
    }
}

/// 9. The seed must do work. A dead generator fails here.
#[test]
fn different_seeds_give_different_boards() {
    let d = dims(9, 9);
    let mut seen: HashSet<[u8; 8]> = HashSet::new();
    for seed in 0..100u64 {
        seen.insert(place(seed, d, 10, &[]).fingerprint());
    }
    assert!(seen.len() >= 95, "only {} different boards in 100", seen.len());
}

/// 10. Over 100 fixed seeds, every cell holds a mine at least one time.
///     The seeds are fixed, so this cannot flake.
#[test]
fn every_cell_can_hold_a_mine() {
    let d = dims(9, 9);
    let mut ever: Vec<bool> = vec![false; d.total()];
    for seed in 0..100u64 {
        let layout = place(seed, d, 10, &[]);
        for i in 0..d.total() {
            let c = d.from_index(i).expect("cell");
            if layout.is_mine(c) {
                if let Some(slot) = ever.get_mut(i) {
                    *slot = true;
                }
            }
        }
    }
    let never: Vec<usize> = ever
        .iter()
        .enumerate()
        .filter(|(_, v)| !**v)
        .map(|(i, _)| i)
        .collect();
    assert!(never.is_empty(), "these cells never held a mine: {never:?}");
}

/// 11. The first click and every cell around it are always safe.
#[test]
fn the_first_click_is_always_safe() {
    let d = dims(9, 9);
    for seed in 0..100u64 {
        for (x, y) in [(0u32, 0u32), (4, 4), (8, 0), (8, 8), (0, 8), (3, 7)] {
            let first = d.coord(x, y).expect("cell");
            let mut exclude = d.neighbours(first);
            exclude.push(first);
            let layout = place(seed, d, 10, &exclude);
            assert!(!layout.is_mine(first), "mine on the first click, seed {seed}");
            for n in d.neighbours(first) {
                assert!(!layout.is_mine(n), "mine beside the first click, seed {seed}");
            }
        }
    }
}

/// 12. Too many mines is refused. It does not loop for ever.
#[test]
fn too_many_mines_is_refused() {
    let cfg = GameConfig {
        width: 9,
        height: 9,
        mine_count: 73, // 81 - 9 leaves room for 72
    };
    assert_eq!(new_game(cfg).err(), Some(ConfigError::TooManyMines));

    let ok = GameConfig {
        width: 9,
        height: 9,
        mine_count: 72,
    };
    assert!(new_game(ok).is_ok());

    let d = dims(4, 4);
    let mut rng = SeededRng::new(1);
    let out = Layout::place(d, 17, &[], &mut |b| rng.next_below(b).map_err(as_domain));
    assert_eq!(out.err(), Some(PlacementError::TooManyMines));
}

/// 13. A stuck generator fails inside the draw cap. It never hangs.
#[test]
fn a_stuck_generator_fails_loudly() {
    // This draw is always thrown away by the fair cut, for a bound of 3.
    let mut draws = 0usize;
    let out = roll_below(3, &mut || {
        draws += 1;
        u64::MAX
    });
    assert_eq!(out, Err(RollFault::Stuck));
    assert_eq!(draws, DRAW_CAP, "the cap must stop the draws");

    assert_eq!(roll_below(0, &mut || 0), Err(RollFault::ZeroBound));

    // A source that answers with a number outside the range it was given is
    // also stuck, not something to fold back in silently.
    struct OutOfRange;
    impl RandomSource for OutOfRange {
        fn next_below(&mut self, bound: u32) -> Result<u32, RollFault> {
            Ok(bound)
        }
    }
    let mut bad = OutOfRange;
    let d = dims(9, 9);
    let out = Layout::place(d, 10, &[], &mut |b| bad.next_below(b).map_err(as_domain));
    assert_eq!(out.err(), Some(PlacementError::Roll(RollError::Stuck)));

    // A source that is stuck fails placement too.
    struct Stuck;
    impl RandomSource for Stuck {
        fn next_below(&mut self, _bound: u32) -> Result<u32, RollFault> {
            Err(RollFault::Stuck)
        }
    }
    let mut s = Stuck;
    let out = Layout::place(d, 10, &[], &mut |b| s.next_below(b).map_err(as_domain));
    assert_eq!(out.err(), Some(PlacementError::Roll(RollError::Stuck)));
}

/// The fair cut gives every value in the range, and never a value above it.
#[test]
fn the_fair_cut_stays_in_range() {
    let mut rng = SeededRng::new(7);
    let mut seen = [0usize; 5];
    for _ in 0..5000 {
        let v = rng.next_below(5).expect("roll");
        assert!(v < 5);
        if let Some(slot) = seen.get_mut(v as usize) {
            *slot += 1;
        }
    }
    for (v, hits) in seen.iter().enumerate() {
        assert!(*hits > 800, "value {v} came up only {hits} times in 5000");
    }
    assert_eq!(rng.next_below(1), Ok(0));
}

/// 14. A board with no mines is legal, and the first click wins it.
#[test]
fn a_board_with_no_mines_is_won_at_once() {
    let cfg = GameConfig {
        width: 9,
        height: 9,
        mine_count: 0,
    };
    let mut s = new_game(cfg).expect("no mines is legal");
    let mut rng = SeededRng::new(42);
    let step = take_command(&mut s, Command::Reveal { x: 4, y: 4 }, &mut rng);
    assert_eq!(step, Step::Ended(EndReason::Won));
}

/// 57. The board size cap holds, and it is the cap the help text names.
///
/// `checked_mul` alone is not enough. 65535 x 65535 fits in a 64 bit number
/// and then asks the machine for gigabytes. The cap refuses it first.
#[test]
fn the_board_size_cap_holds() {
    assert_eq!(Dims::new(65535, 65535).err(), Some(ConfigError::TooLarge));

    // Exactly the cap is accepted. 1000 x 1000 is MAX_CELLS cells.
    let at_cap = Dims::new(1000, 1000).expect("a board of exactly the cap must build");
    assert_eq!(at_cap.total(), MAX_CELLS);

    // One cell more is refused. 101 x 9901 is MAX_CELLS + 1 cells.
    assert_eq!(
        usize::from(101u16) * usize::from(9901u16),
        MAX_CELLS + 1,
        "this test must ask for exactly one cell above the cap"
    );
    assert_eq!(Dims::new(101, 9901).err(), Some(ConfigError::TooLarge));

    // A zero side is refused before the sum is ever done.
    assert_eq!(Dims::new(0, 10).err(), Some(ConfigError::ZeroWidth));
    assert_eq!(Dims::new(10, 0).err(), Some(ConfigError::ZeroHeight));
}
