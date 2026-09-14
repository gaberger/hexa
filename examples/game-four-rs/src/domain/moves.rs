//! The move history: forty-two small numbers and a length. No heap.

use super::column::{Column, COLUMNS, ROWS};

/// The list of columns played so far, oldest first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveList {
    slots: [u8; COLUMNS * ROWS],
    len: usize,
}

impl MoveList {
    /// An empty history.
    pub fn new() -> MoveList {
        MoveList {
            slots: [0; COLUMNS * ROWS],
            len: 0,
        }
    }

    /// How many discs have been played.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True before the first move.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Add one column to the end. The board never allows a 43rd move.
    pub fn push(&mut self, column: Column) {
        assert!(self.len < self.slots.len(), "more than 42 moves");
        self.slots[self.len] = column.index() as u8;
        self.len += 1;
    }

    /// Walk the history, oldest first.
    pub fn iterate(&self) -> impl Iterator<Item = Column> + '_ {
        self.slots[..self.len]
            .iter()
            .filter_map(|raw| Column::new(*raw).ok())
    }
}

impl Default for MoveList {
    fn default() -> MoveList {
        MoveList::new()
    }
}
