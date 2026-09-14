//! C. Reveal.
//!
//! Opening an empty cell spreads. The spread must never open a mine, must stop
//! at a flag, must leave no permanent hole, and must not fall over on a large
//! empty board.

use minesweeper::adapters::secondary::SeededRng;
use minesweeper::domain::{Board, CellState, Coord, Dims, Layout, MoveError, RollError};
use minesweeper::ports::{RandomSource, RollFault};

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

/// The same hand drawn 5x5 board used by the geometry tests.
///
/// ```text
///   M . . . .
///   . . . . .
///   . . M . .
///   . M . . .
///   . . . . M
/// ```
fn fixture() -> (Dims, Board) {
    let d = dims(5, 5);
    let mines: Vec<Coord> = [(0u32, 0u32), (2, 2), (4, 4), (1, 3)]
        .iter()
        .map(|(x, y)| d.coord(*x, *y).expect("mine cell"))
        .collect();
    let layout = Layout::from_mines(d, &mines).expect("layout");
    (d, Board::new(layout))
}

fn open_set(d: Dims, b: &Board) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for i in 0..d.total() {
        let c = d.from_index(i).expect("cell");
        if b.cell_state(c) == CellState::Revealed {
            out.push((u32::from(c.x()), u32::from(c.y())));
        }
    }
    out.sort_unstable();
    out
}

/// A picture of the whole board, so two boards can be compared byte for byte.
fn snapshot(d: Dims, b: &Board) -> String {
    let mut s = String::new();
    for i in 0..d.total() {
        let c = d.from_index(i).expect("cell");
        s.push(match b.cell_state(c) {
            CellState::Hidden => 'h',
            CellState::Revealed => 'r',
            CellState::Flagged => 'f',
        });
    }
    s.push_str(&format!("|safe={}|blast={:?}", b.revealed_safe(), b.blast()));
    s
}

/// 15. The set of cells opened by one click, written out by a person who
///     followed the spread by hand on the picture above.
#[test]
fn the_spread_opens_the_set_a_person_traced() {
    let (d, mut b) = fixture();
    let click = d.coord(4, 0).expect("cell");
    b.reveal(click).expect("reveal");

    // Traced by hand from the top right corner, which shows nothing.
    let by_hand = vec![
        (1u32, 0u32),
        (2, 0),
        (3, 0),
        (4, 0),
        (1, 1),
        (2, 1),
        (3, 1),
        (4, 1),
        (3, 2),
        (4, 2),
        (3, 3),
        (4, 3),
    ];
    let mut want = by_hand.clone();
    want.sort_unstable();

    assert_eq!(open_set(d, &b), want);
    assert_eq!(b.revealed_safe(), 12);
    assert_eq!(b.blast(), None);
}

/// 16. Over 200 boards, a spread never opens a mine.
#[test]
fn the_spread_never_opens_a_mine() {
    let d = dims(12, 9);
    for seed in 0..200u64 {
        let mut rng = SeededRng::new(seed);
        let layout = Layout::place(d, 12, &[], &mut |bd| rng.next_below(bd).map_err(as_domain)).expect("place");
        let mut b = Board::new(layout);
        // Open every safe cell in turn. Each spread must stay off the mines.
        for i in 0..d.total() {
            let c = d.from_index(i).expect("cell");
            if b.is_mine(c) {
                continue;
            }
            let _ = b.reveal(c);
            for j in 0..d.total() {
                let k = d.from_index(j).expect("cell");
                if b.is_mine(k) {
                    assert_eq!(
                        b.cell_state(k),
                        CellState::Hidden,
                        "a mine was opened, seed {seed}"
                    );
                }
            }
        }
    }
}

/// 17. A spread stops at a flag, and the flag stays on.
#[test]
fn the_spread_stops_at_a_flag() {
    let (d, mut b) = fixture();
    let flagged = d.coord(2, 0).expect("cell");
    b.toggle_flag(flagged).expect("flag");
    b.reveal(d.coord(4, 0).expect("cell")).expect("reveal");

    assert_eq!(b.cell_state(flagged), CellState::Flagged);
    let mut want = vec![
        (4u32, 0u32),
        (3, 0),
        (3, 1),
        (4, 1),
        (3, 2),
        (4, 2),
        (3, 3),
        (4, 3),
        (2, 1),
    ];
    want.sort_unstable();
    assert_eq!(open_set(d, &b), want);
    // The cells behind the flag stayed shut.
    assert_eq!(b.cell_state(d.coord(1, 0).expect("cell")), CellState::Hidden);
    assert_eq!(b.cell_state(d.coord(1, 1).expect("cell")), CellState::Hidden);
}

/// 18. Take the flag off and click again. The cell opens. No permanent hole.
#[test]
fn a_cell_unflagged_inside_an_open_region_still_opens() {
    let (d, mut b) = fixture();
    let flagged = d.coord(2, 0).expect("cell");
    b.toggle_flag(flagged).expect("flag on");
    b.reveal(d.coord(4, 0).expect("cell")).expect("reveal");
    b.toggle_flag(flagged).expect("flag off");
    assert_eq!(b.cell_state(flagged), CellState::Hidden);

    b.reveal(flagged).expect("reveal the freed cell");
    assert_eq!(b.cell_state(flagged), CellState::Revealed);
    // The spread carries on from there and opens the rest of the top left.
    assert_eq!(b.cell_state(d.coord(1, 0).expect("cell")), CellState::Revealed);
    assert_eq!(b.cell_state(d.coord(1, 1).expect("cell")), CellState::Revealed);
    assert_eq!(b.revealed_safe(), 12);
}

/// 19. A very large empty board floods in full, and the program stays up.
///     A spread written with recursion would run out of call stack here.
#[test]
fn a_large_empty_board_floods_without_falling_over() {
    let d = dims(300, 300);
    let layout = Layout::from_mines(d, &[]).expect("layout");
    let mut b = Board::new(layout);
    b.reveal(d.coord(0, 0).expect("cell")).expect("reveal");
    assert_eq!(b.revealed_safe(), 90_000);
    assert_eq!(b.check_invariants(), Ok(()));
}

/// 20. A refused click changes nothing at all.
#[test]
fn a_refused_click_leaves_the_board_untouched() {
    let (d, mut b) = fixture();
    b.reveal(d.coord(4, 0).expect("cell")).expect("reveal");
    let flagged = d.coord(0, 2).expect("cell");
    b.toggle_flag(flagged).expect("flag");

    let before = snapshot(d, &b);

    let open = d.coord(4, 0).expect("cell");
    assert_eq!(b.reveal(open), Err(MoveError::AlreadyRevealed));
    assert_eq!(snapshot(d, &b), before);

    assert_eq!(b.reveal(flagged), Err(MoveError::CellIsFlagged));
    assert_eq!(snapshot(d, &b), before);

    assert_eq!(b.toggle_flag(open), Err(MoveError::CannotFlagRevealed));
    assert_eq!(snapshot(d, &b), before);
}
