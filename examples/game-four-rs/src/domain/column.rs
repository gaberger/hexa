//! A checked column number.

use super::errors::MoveError;

/// The board is seven columns wide.
pub const COLUMNS: usize = 7;
/// The board is six rows tall.
pub const ROWS: usize = 6;

/// A column that is known to exist. The inner number is private, so the only
/// way to get one is through [`Column::new`], which checks the range.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Column(u8);

impl Column {
    /// Build a column from a zero-based number.
    pub fn new(value: u8) -> Result<Column, MoveError> {
        if (value as usize) < COLUMNS {
            Ok(Column(value))
        } else {
            Err(MoveError::OutOfRange)
        }
    }

    /// The zero-based index, for reading the cells.
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The one-based number a human types.
    pub fn number(self) -> u8 {
        self.0 + 1
    }
}
