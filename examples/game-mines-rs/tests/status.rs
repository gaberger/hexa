//! D. Status - the group that matters most.
//!
//! One word decides whether a win is real: the game counts revealed **safe**
//! cells, never revealed cells. If a mine could add to the win counter, the
//! game would announce a win at the exact moment you lose.

use minesweeper::adapters::secondary::SeededRng;
use minesweeper::domain::{Board, Coord, Dims, Layout, MoveError, RollError, Status};
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

/// A 3x3 board with one mine in the middle. Every other cell shows 1.
fn tiny() -> (Dims, Coord, Board) {
    let d = dims(3, 3);
    let mine = d.coord(1, 1).expect("cell");
    let layout = Layout::from_mines(d, &[mine]).expect("layout");
    (d, mine, Board::new(layout))
}

fn safe_cells(d: Dims, mine: Coord) -> Vec<Coord> {
    (0..d.total())
        .filter_map(|i| d.from_index(i))
        .filter(|c| *c != mine)
        .collect()
}

/// 21. Open all 8 safe cells and you win. Open 7 and then step on the mine and
///     you lose, and you stay lost.
#[test]
fn a_mine_never_adds_to_the_win_counter() {
    let (d, mine, mut won) = tiny();
    for c in safe_cells(d, mine) {
        won.reveal(c).expect("reveal a safe cell");
    }
    assert_eq!(won.revealed_safe(), 8);
    assert_eq!(won.status(), Status::Won);

    let (d, mine, mut lost) = tiny();
    let safe = safe_cells(d, mine);
    for c in safe.iter().take(7) {
        lost.reveal(*c).expect("reveal a safe cell");
    }
    assert_eq!(lost.status(), Status::Playing);
    lost.reveal(mine).expect("step on the mine");

    // Eight cells are now open, but only seven of them are safe.
    assert_eq!(lost.revealed_safe(), 7, "a mine must not count as progress");
    assert_eq!(lost.status(), Status::Lost);
    assert_eq!(lost.blast(), Some(mine));

    // It stays lost. It never turns into a win afterwards.
    assert_eq!(lost.status(), Status::Lost);
    assert_eq!(lost.check_invariants(), Ok(()));
}

/// 22. Flags do not decide a win.
///
/// A safe cell that keeps a flag can never be opened, so it can never be part
/// of a win. That is why the wrong-flag case is tested as flags that were put
/// down in the wrong place during play. The win is the same either way.
#[test]
fn flags_do_not_decide_a_win() {
    // (a) A win with no flags at all.
    let (d, mine, mut a) = tiny();
    for c in safe_cells(d, mine) {
        a.reveal(c).expect("reveal");
    }
    assert_eq!(a.status(), Status::Won);
    assert_eq!(a.flags_placed(), 0);

    // (b) A win with a flag on the mine.
    let (d, mine, mut b) = tiny();
    b.toggle_flag(mine).expect("flag the mine");
    for c in safe_cells(d, mine) {
        b.reveal(c).expect("reveal");
    }
    assert_eq!(b.status(), Status::Won);
    assert_eq!(b.flags_placed(), 1);

    // (c) A win after flags were put in the wrong place and taken off again.
    let (d, mine, mut c) = tiny();
    let safe = safe_cells(d, mine);
    for cell in safe.iter() {
        c.toggle_flag(*cell).expect("flag a safe cell");
    }
    assert_eq!(c.flags_placed(), 8);
    assert_eq!(c.status(), Status::Playing, "flags alone never win");
    for cell in safe.iter() {
        c.toggle_flag(*cell).expect("unflag");
        c.reveal(*cell).expect("reveal");
    }
    assert_eq!(c.status(), Status::Won);
    assert_eq!(c.flags_placed(), 0);
}

/// 23. Once the game is lost, every further command is refused.
#[test]
fn a_lost_game_refuses_everything() {
    let (d, mine, mut b) = tiny();
    b.reveal(mine).expect("step on the mine");
    assert_eq!(b.status(), Status::Lost);

    for c in safe_cells(d, mine) {
        assert_eq!(b.reveal(c), Err(MoveError::GameOver));
        assert_eq!(b.toggle_flag(c), Err(MoveError::GameOver));
    }
    assert_eq!(b.reveal(mine), Err(MoveError::GameOver));
    assert_eq!(b.revealed_safe(), 0);
    assert_eq!(b.status(), Status::Lost);
}

/// A won game refuses everything too.
#[test]
fn a_won_game_refuses_everything() {
    let (d, mine, mut b) = tiny();
    for c in safe_cells(d, mine) {
        b.reveal(c).expect("reveal");
    }
    assert_eq!(b.status(), Status::Won);
    assert_eq!(b.reveal(mine), Err(MoveError::GameOver));
    assert_eq!(b.toggle_flag(mine), Err(MoveError::GameOver));
    assert_eq!(b.status(), Status::Won);
}

/// 24. Ten thousand moves picked at random. Every promise holds after each one.
#[test]
fn a_long_random_walk_keeps_every_promise() {
    let d = dims(9, 9);
    let mut rng = SeededRng::new(20260914);
    let mut games = 0usize;
    let mut board = {
        let layout = Layout::place(d, 10, &[], &mut |b| rng.next_below(b).map_err(as_domain)).expect("place");
        Board::new(layout)
    };

    for move_no in 0..10_000u32 {
        if board.status() != Status::Playing {
            games += 1;
            let layout = Layout::place(d, 10, &[], &mut |b| rng.next_below(b).map_err(as_domain)).expect("place");
            board = Board::new(layout);
        }
        let slot = rng.next_below(81).expect("roll") as usize;
        let c = d.from_index(slot).expect("cell");
        let flag = rng.next_below(3).expect("roll") == 0;
        let _ = if flag {
            board.toggle_flag(c)
        } else {
            board.reveal(c)
        };
        assert_eq!(
            board.check_invariants(),
            Ok(()),
            "a promise broke after move {move_no}"
        );
        assert!(
            !(board.status() == Status::Won && board.blast().is_some()),
            "won and lost at the same time after move {move_no}"
        );
    }
    assert!(games > 0, "the walk never finished a game");
}
