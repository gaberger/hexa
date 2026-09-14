//! The two colours.

/// A single disc. Red always moves first.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Disc {
    Red,
    Yellow,
}

impl Disc {
    /// The one character that stands for this disc on screen.
    pub fn glyph(self) -> char {
        match self {
            Disc::Red => 'R',
            Disc::Yellow => 'Y',
        }
    }

    /// The other colour.
    pub fn other(self) -> Disc {
        match self {
            Disc::Red => Disc::Yellow,
            Disc::Yellow => Disc::Red,
        }
    }
}
