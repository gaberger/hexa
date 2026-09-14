//! Step a run backwards.

use crate::domain::level::Level;
use crate::domain::position::Dir;
use crate::domain::push::{commit, Board, Outcome};
use crate::ports::move_recorder::MoveRecorder;

/// Rebuild the board by replaying every move but the last.
///
/// Replaying from the start rather than inverting the last move is
/// deliberate: inverting a push has to know whether the player pulled the
/// box back or stepped off it, and getting that wrong corrupts the board
/// in a way that only shows up several moves later.
pub fn undo(level: &Level, recorder: &mut dyn MoveRecorder) -> Board {
    recorder.rewind();
    replay(level, &recorder.history())
}

pub fn replay(level: &Level, moves: &[Dir]) -> Board {
    let mut board = Board::new(level);
    for dir in moves {
        let outcome = crate::domain::push::apply(level, &board, *dir);
        if outcome != Outcome::Blocked {
            board = commit(&board, &outcome);
        }
    }
    board
}
