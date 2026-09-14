//! The screen plug.

use core::fmt;

use crate::domain::{BoardView, Outcome};

/// The one way drawing can fail.
///
/// This error is named here, in the ports. It is not `std::io::Error`, and it
/// does not carry a disk word into the game. "Unwritable" is true of a screen,
/// a pipe and a file alike.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderError {
    Unwritable,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("could not write to the output")
    }
}

/// Anything that can show a board.
///
/// Both methods take `&mut self`. A `&self` port would force a `RefCell`
/// inside every writer, which is a lock added by a rule meant to remove locks.
pub trait Renderer {
    /// Show one board.
    fn frame(&mut self, view: &BoardView) -> Result<(), RenderError>;
    /// Show the result line, once, at the end.
    fn announce(&mut self, outcome: Outcome) -> Result<(), RenderError>;
}
