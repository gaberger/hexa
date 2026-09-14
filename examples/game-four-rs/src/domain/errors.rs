//! Every way a move can fail.

use core::fmt;

/// A move that the rules refuse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveError {
    /// The column number is not 0..=6.
    OutOfRange,
    /// The column already holds six discs.
    ColumnFull,
    /// Somebody won, or the board is full.
    GameOver,
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            MoveError::OutOfRange => "that column does not exist",
            MoveError::ColumnFull => "that column is full",
            MoveError::GameOver => "the game is over",
        };
        f.write_str(text)
    }
}
