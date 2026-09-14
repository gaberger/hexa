//! How a game stands.

use super::disc::Disc;

/// The state of the game after a move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Nobody has won and the board has room.
    InProgress,
    /// This colour made a line of four or more.
    Win(Disc),
    /// The board is full and nobody made a line.
    Draw,
}

impl Outcome {
    /// True once no further move is allowed.
    pub fn is_final(self) -> bool {
        !matches!(self, Outcome::InProgress)
    }
}
