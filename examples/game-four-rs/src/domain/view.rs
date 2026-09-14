//! What the board looks like from outside, and what may be played next.

use super::column::{Column, COLUMNS, ROWS};
use super::disc::Disc;

/// Forty-two cells.
pub const CELLS: usize = COLUMNS * ROWS;

/// A flat, read-only copy of the board.
///
/// The cells are **bottom row first**: index `row * 7 + col`. Index 0 is the
/// floor of the left column. Index 41 is the top of the right column. The
/// renderer turns this the other way up for the screen; nothing else does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BoardView {
    pub cells: [Option<Disc>; CELLS],
}

impl BoardView {
    /// The disc at a place, or `None` if the place is empty.
    pub fn at(&self, column: usize, row: usize) -> Option<Disc> {
        if column >= COLUMNS || row >= ROWS {
            return None;
        }
        self.cells[row * COLUMNS + column]
    }

    /// How many discs are on the board.
    pub fn disc_count(&self) -> usize {
        self.cells.iter().filter(|cell| cell.is_some()).count()
    }
}

/// The columns that still have room, in rising order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegalMoves {
    items: [Option<Column>; COLUMNS],
    len: usize,
}

impl LegalMoves {
    /// Build the list from the first `len` filled slots.
    pub(super) fn from_slots(items: [Option<Column>; COLUMNS], len: usize) -> LegalMoves {
        assert!(len <= COLUMNS, "more open columns than the board has");
        LegalMoves { items, len }
    }

    /// How many columns have room. Zero only when the board is full.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the board is full.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Walk the playable columns, left to right.
    pub fn iterate(&self) -> impl Iterator<Item = Column> + '_ {
        self.items[..self.len].iter().flatten().copied()
    }

    /// True when this column still has room.
    pub fn contains(&self, column: Column) -> bool {
        self.iterate().any(|item| item == column)
    }
}
