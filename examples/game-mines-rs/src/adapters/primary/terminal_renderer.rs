//! Draws the grid with plain writes. It can print no text that you typed.

use std::io::{self, Write};

use crate::adapters::primary::text::{frame, notice_text};
use crate::ports::renderer::Renderer;
use crate::ports::view::{BoardView, Notice};

/// Writes the game to any sink. The program never changes terminal settings,
/// so it never has to put them back.
#[derive(Debug)]
pub struct TerminalRenderer<W: Write> {
    out: W,
}

impl<W: Write> TerminalRenderer<W> {
    pub fn new(out: W) -> TerminalRenderer<W> {
        TerminalRenderer { out }
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

impl<W: Write> Renderer for TerminalRenderer<W> {
    fn render(&mut self, view: &BoardView) -> io::Result<()> {
        let text = frame(view);
        self.out.write_all(text.as_bytes())?;
        self.out.flush()
    }

    fn notice(&mut self, notice: Notice) -> io::Result<()> {
        let text = notice_text(notice);
        self.out.write_all(text.as_bytes())?;
        self.out.flush()
    }
}
