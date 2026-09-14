//! One turn of the game.

use crate::domain::board::Board;
use crate::domain::coord::Coord;
use crate::domain::errors::{MoveError, RollError};
use crate::domain::layout::Layout;
use crate::domain::status::Status;
use crate::ports::random::{RandomSource, RollFault};
use crate::ports::view::{Command, Notice};
use crate::usecases::new_game::{EndReason, Fault, Session};

/// What one turn did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Applied,
    Refused(Notice),
    Ended(EndReason),
    /// The program itself broke. It is not a refusal, and it is not an ending.
    Fault(Fault),
}

fn refuse(e: MoveError) -> Step {
    match e {
        MoveError::GameOver => Step::Refused(Notice::GameOver),
        MoveError::OffBoard => Step::Refused(Notice::OffBoard),
        MoveError::AlreadyRevealed => Step::Refused(Notice::AlreadyRevealed),
        MoveError::CellIsFlagged => Step::Refused(Notice::CellIsFlagged),
        MoveError::CannotFlagRevealed => Step::Refused(Notice::CannotFlagRevealed),
    }
}

fn after_move(s: &Session) -> Step {
    if cfg!(debug_assertions) {
        if let Some(b) = s.board() {
            if let Err(e) = b.check_invariants() {
                return Step::Fault(Fault::Invariant(e));
            }
        }
    }
    match s.board().map(|b| b.status()) {
        Some(Status::Won) => Step::Ended(EndReason::Won),
        Some(Status::Lost) => Step::Ended(EndReason::Lost),
        _ => Step::Applied,
    }
}

/// Build the mines around the first click, then keep them for ever.
fn build_layout(s: &mut Session, first: Coord, rng: &mut dyn RandomSource) -> Result<(), Fault> {
    let dims = s.dims();
    let mut exclude = dims.neighbours(first);
    exclude.push(first);
    // The port has its own failure type, so no adapter has to name the domain.
    // This closure is the one place the two words for the same trouble meet.
    let mut roll = |bound: u32| {
        rng.next_below(bound).map_err(|f| match f {
            RollFault::ZeroBound => RollError::ZeroBound,
            RollFault::Stuck => RollError::Stuck,
        })
    };
    let layout = Layout::place(dims, s.mine_count(), &exclude, &mut roll)
        .map_err(Fault::Placement)?;
    let mut board = Board::new(layout);
    // Carry over any flags placed before the first reveal.
    for i in 0..dims.total() {
        if s.pre_flags().get(i).copied() != Some(true) {
            continue;
        }
        if let Some(c) = dims.from_index(i) {
            let _ = board.toggle_flag(c);
        }
    }
    s.set_board(board);
    Ok(())
}

/// Apply one command to the game.
pub fn take_command(s: &mut Session, cmd: Command, rng: &mut dyn RandomSource) -> Step {
    match cmd {
        Command::Quit => Step::Ended(EndReason::Quit),
        Command::Unknown => Step::Refused(Notice::BadCommand),
        Command::Reveal { x, y } => {
            let dims = s.dims();
            let c = match dims.coord(x, y) {
                Some(c) => c,
                None => return Step::Refused(Notice::OffBoard),
            };
            if s.board().is_none() {
                let i = match dims.index(c) {
                    Some(i) => i,
                    None => return Step::Refused(Notice::OffBoard),
                };
                if s.pre_flags().get(i).copied() == Some(true) {
                    return Step::Refused(Notice::CellIsFlagged);
                }
                if let Err(f) = build_layout(s, c, rng) {
                    return Step::Fault(f);
                }
            }
            let outcome = match s.board_mut() {
                Some(b) => b.reveal(c),
                None => return Step::Fault(Fault::Invariant(
                    crate::domain::errors::InvariantError::LengthMismatch,
                )),
            };
            match outcome {
                Ok(()) => after_move(s),
                Err(e) => refuse(e),
            }
        }
        Command::Flag { x, y } => {
            let dims = s.dims();
            let c = match dims.coord(x, y) {
                Some(c) => c,
                None => return Step::Refused(Notice::OffBoard),
            };
            match s.board_mut() {
                Some(b) => match b.toggle_flag(c) {
                    Ok(()) => after_move(s),
                    Err(e) => refuse(e),
                },
                None => {
                    let i = match dims.index(c) {
                        Some(i) => i,
                        None => return Step::Refused(Notice::OffBoard),
                    };
                    if s.toggle_pre_flag(i) {
                        Step::Applied
                    } else {
                        Step::Refused(Notice::OffBoard)
                    }
                }
            }
        }
    }
}
