//! The window the player looks through.

use std::io;

use crate::ports::view::{BoardView, Notice};

/// Draws the board and shows messages. No method takes free text.
///
/// The trait carries no `Send` and no `Sync`. The game is single threaded, and
/// the contract stays boring.
pub trait Renderer {
    fn render(&mut self, view: &BoardView) -> io::Result<()>;
    fn notice(&mut self, notice: Notice) -> io::Result<()>;
}
