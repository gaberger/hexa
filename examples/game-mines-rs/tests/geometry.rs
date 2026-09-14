//! A. Geometry - the edge trap.
//!
//! A neighbour computed from a flat slot number wraps around the edge of the
//! board. Then every edge count is wrong, and a test written from the same
//! idea cannot see it. These tests come at the geometry from the outside.

use minesweeper::domain::{ConfigError, Dims, RollError};
use minesweeper::ports::RollFault;

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

/// 1. A corner has 3 neighbours, an edge has 5, an inside cell has 8.
#[test]
fn neighbour_counts_match_the_position() {
    let d = dims(9, 9);
    let corner = d.coord(0, 0).expect("corner");
    let edge = d.coord(0, 4).expect("edge");
    let inside = d.coord(4, 4).expect("inside");
    assert_eq!(d.neighbours(corner).len(), 3);
    assert_eq!(d.neighbours(edge).len(), 5);
    assert_eq!(d.neighbours(inside).len(), 8);

    for (x, y) in [(8u32, 0u32), (0, 8), (8, 8)] {
        let c = d.coord(x, y).expect("corner");
        assert_eq!(d.neighbours(c).len(), 3, "corner {x},{y}");
    }
    for (x, y) in [(4u32, 0u32), (8, 4), (4, 8)] {
        let c = d.coord(x, y).expect("edge");
        assert_eq!(d.neighbours(c).len(), 5, "edge {x},{y}");
    }

    let one = dims(1, 1);
    let only = one.coord(0, 0).expect("only cell");
    assert_eq!(one.neighbours(only).len(), 0);
}

/// 2. Nothing wraps around. A cell on the left edge has no neighbour on the right.
#[test]
fn the_left_edge_never_touches_the_right_edge() {
    // Boards narrower than 3 columns are skipped. There the right column is a
    // true neighbour of the left column, so a wrap cannot be told apart.
    for (w, h) in [(9u16, 9u16), (30, 1), (5, 7), (3, 3)] {
        let d = dims(w, h);
        let last = u32::from(w) - 1;
        for y in 0..u32::from(h) {
            let left = d.coord(0, y).expect("left cell");
            for n in d.neighbours(left) {
                assert_ne!(u32::from(n.x()), last, "wrap at row {y} on {w}x{h}");
                let dy = i64::from(n.y()) - i64::from(y);
                assert!(dy.abs() <= 1, "row jump at {y}");
            }
            let right = d.coord(last, y).expect("right cell");
            for n in d.neighbours(right) {
                assert_ne!(n.x(), 0, "wrap back at row {y} on {w}x{h}");
            }
        }
    }
}

/// 3. A position turns into a slot and back again, for every cell.
#[test]
fn index_and_from_index_round_trip() {
    for (w, h) in [(9u16, 9u16), (1, 30), (30, 1)] {
        let d = dims(w, h);
        for i in 0..d.total() {
            let c = d.from_index(i).expect("slot must map back");
            assert_eq!(d.index(c), Some(i), "round trip at slot {i} on {w}x{h}");
        }
        assert_eq!(d.from_index(d.total()), None);
    }
}

/// 4. A hand drawn 5x5 board. The counts below were written by a person, by
///    looking at the picture. They are the independent oracle.
///
/// ```text
///   M . . . .
///   . . . . .
///   . . M . .
///   . M . . .
///   . . . . M
/// ```
#[test]
fn hand_drawn_counts_match_the_code() {
    use minesweeper::domain::Layout;

    let d = dims(5, 5);
    let mines: Vec<_> = [(0u32, 0u32), (2, 2), (4, 4), (1, 3)]
        .iter()
        .map(|(x, y)| d.coord(*x, *y).expect("mine cell"))
        .collect();
    let layout = Layout::from_mines(d, &mines).expect("layout");

    // Written by hand, row by row, from the picture above.
    let by_hand: [[u8; 5]; 5] = [
        [0, 1, 0, 0, 0],
        [1, 2, 1, 1, 0],
        [1, 2, 1, 1, 0],
        [1, 1, 2, 2, 1],
        [1, 1, 1, 1, 0],
    ];

    for y in 0..5u32 {
        for x in 0..5u32 {
            let c = d.coord(x, y).expect("cell");
            let want = by_hand
                .get(y as usize)
                .and_then(|row| row.get(x as usize))
                .copied()
                .expect("hand table");
            assert_eq!(layout.adjacent(c), want, "count at col {x} row {y}");
        }
    }
}

/// 5. A property check. Recount every cell by brute force, over 200 layouts.
#[test]
fn every_count_equals_a_brute_force_recount() {
    use minesweeper::adapters::secondary::SeededRng;
    use minesweeper::domain::Layout;
    use minesweeper::ports::RandomSource;

    for seed in 0..200u64 {
        let d = dims(7, 11);
        let mut rng = SeededRng::new(seed);
        let layout = Layout::place(d, 13, &[], &mut |b| rng.next_below(b).map_err(as_domain)).expect("place");
        for i in 0..d.total() {
            let c = d.from_index(i).expect("cell");
            let brute = d
                .neighbours(c)
                .into_iter()
                .filter(|n| layout.is_mine(*n))
                .count();
            assert_eq!(usize::from(layout.adjacent(c)), brute, "seed {seed} slot {i}");
        }
    }
}

/// 6. A position off the board cannot be made. So `r 99 99` cannot crash.
#[test]
fn a_position_off_the_board_cannot_be_made() {
    let d = dims(9, 9);
    assert!(d.coord(9, 0).is_none());
    assert!(d.coord(0, 9).is_none());
    assert!(d.coord(99, 99).is_none());
    assert!(d.coord(u32::MAX, 0).is_none());
    assert!(d.coord(0, u32::MAX).is_none());
    assert!(d.coord(u32::MAX, u32::MAX).is_none());
    assert!(d.coord(8, 8).is_some());
}

/// A board with no width or no height is refused.
#[test]
fn an_empty_board_is_refused() {
    assert_eq!(Dims::new(0, 9), Err(ConfigError::ZeroWidth));
    assert_eq!(Dims::new(9, 0), Err(ConfigError::ZeroHeight));
    assert_eq!(Dims::new(2000, 2000), Err(ConfigError::TooLarge));
}
