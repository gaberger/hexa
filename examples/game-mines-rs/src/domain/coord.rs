//! A position on the board that is known to be on the board.

/// A column and a row. The fields are private, and only `Dims::coord` builds one.
/// So a position off the board cannot exist inside the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Coord {
    x: u16,
    y: u16,
}

impl Coord {
    /// Only the domain builds a `Coord`, and only after a bounds check.
    pub(in crate::domain) fn new(x: u16, y: u16) -> Coord {
        Coord { x, y }
    }

    pub fn x(&self) -> u16 {
        self.x
    }

    pub fn y(&self) -> u16 {
        self.y
    }
}
