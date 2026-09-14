//! Draws nothing per turn. Used by the demo, which has no terminal to fill.

use std::io::{self, Write};

use crate::adapters::primary::text::notice_text;
use crate::ports::renderer::Renderer;
use crate::ports::view::{BoardView, Notice};

/// Shows only the three closing messages: the board line, the stats line and
/// the ending line.
#[derive(Debug)]
pub struct QuietRenderer<W: Write> {
    out: W,
}

impl<W: Write> QuietRenderer<W> {
    pub fn new(out: W) -> QuietRenderer<W> {
        QuietRenderer { out }
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

impl<W: Write> Renderer for QuietRenderer<W> {
    fn render(&mut self, _view: &BoardView) -> io::Result<()> {
        Ok(())
    }

    fn notice(&mut self, notice: Notice) -> io::Result<()> {
        let keep = matches!(
            notice,
            Notice::Fingerprint(_)
                | Notice::Stats { .. }
                | Notice::GameOver
                | Notice::YouWin
                | Notice::Quit
        );
        if !keep {
            return Ok(());
        }
        let text = notice_text(notice);
        self.out.write_all(text.as_bytes())?;
        self.out.flush()
    }
}
