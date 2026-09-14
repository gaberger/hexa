//! The writer for a person at a terminal.
//!
//! This one adds the column numbers and a blank line. No gate reads it, so it
//! is free to be friendly. It is never used in demo mode.
//!
//! It builds its own text. It does **not** borrow the strict renderer's text,
//! because an adapter may not import another adapter. Twenty lines of
//! duplication is the price of a boundary that holds.

use std::io::Write;

use crate::ports::{BoardView, Disc, Outcome, RenderError, Renderer};

const COLUMNS: usize = 7;
const ROWS: usize = 6;

/// A writer that shows the board with a ruler under it.
pub struct HumanRenderer<W: Write> {
    out: W,
}

impl<W: Write> HumanRenderer<W> {
    /// Wrap a place to write.
    pub fn new(out: W) -> HumanRenderer<W> {
        HumanRenderer { out }
    }

    fn emit(&mut self, text: &str) -> Result<(), RenderError> {
        self.out
            .write_all(text.as_bytes())
            .map_err(|_| RenderError::Unwritable)?;
        self.out.flush().map_err(|_| RenderError::Unwritable)
    }
}

impl<W: Write> Renderer for HumanRenderer<W> {
    fn frame(&mut self, view: &BoardView) -> Result<(), RenderError> {
        let mut text = String::from("\n");
        for row in (0..ROWS).rev() {
            for column in 0..COLUMNS {
                text.push(match view.at(column, row) {
                    Some(disc) => disc.glyph(),
                    None => '.',
                });
            }
            text.push('\n');
        }
        text.push_str("1234567\n");
        self.emit(&text)
    }

    fn announce(&mut self, outcome: Outcome) -> Result<(), RenderError> {
        let words = match outcome {
            Outcome::Win(Disc::Red) => "RED WINS",
            Outcome::Win(Disc::Yellow) => "YELLOW WINS",
            Outcome::Draw => "DRAW",
            Outcome::InProgress => "DRAW",
        };
        let mut text = String::from("\n");
        text.push_str(words);
        text.push('\n');
        self.emit(&text)
    }
}
