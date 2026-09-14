//! The keyboard plug.

use core::fmt;

use crate::domain::{BoardView, Column, Disc, LegalMoves};

/// The one way reading can fail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputError {
    Unreadable,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("could not read a move")
    }
}

/// What a player decided to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Play(Column),
    Quit,
}

/// Anything that can pick a column.
///
/// The chooser is handed the legal columns, so it never has to guess and never
/// has to retry. It is not given the board to change, only to look at.
pub trait InputSource {
    fn choose(
        &mut self,
        view: &BoardView,
        legal: &LegalMoves,
        to_move: Disc,
    ) -> Result<Choice, InputError>;
}
