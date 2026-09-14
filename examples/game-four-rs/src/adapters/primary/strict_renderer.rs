//! The machine contract writer.
//!
//! Demo mode prints these bytes and nothing else:
//!
//! * one frame after each move, and no empty frame at the start;
//! * a frame is six lines, top row first;
//! * a line is exactly seven characters from `.`, `R` and `Y`;
//! * every line ends with `\n`, never `\r\n`;
//! * the last line is `RED WINS`, `YELLOW WINS` or `DRAW`.

use std::io::Write;

use crate::ports::{BoardView, Disc, Outcome, RenderError, Renderer};

const COLUMNS: usize = 7;
const ROWS: usize = 6;

/// Turn one board into its six lines, top row first.
///
/// This is the only flip in the program. The board is bottom row first, and
/// the screen wants the top row first, so the rows are walked downward once.
fn frame_text(view: &BoardView) -> String {
    let mut text = String::with_capacity(ROWS * (COLUMNS + 1));
    for row in (0..ROWS).rev() {
        for column in 0..COLUMNS {
            text.push(match view.at(column, row) {
                Some(disc) => disc.glyph(),
                None => '.',
            });
        }
        text.push('\n');
    }
    text
}

/// The three words the gate reads.
fn result_text(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Win(Disc::Red) => "RED WINS",
        Outcome::Win(Disc::Yellow) => "YELLOW WINS",
        Outcome::Draw => "DRAW",
        Outcome::InProgress => "DRAW",
    }
}

/// A writer that prints the contract and nothing else.
pub struct StrictRenderer<W: Write> {
    out: W,
}

impl<W: Write> StrictRenderer<W> {
    /// Wrap a place to write.
    pub fn new(out: W) -> StrictRenderer<W> {
        StrictRenderer { out }
    }

    /// Give the writer back, for a test that wants the bytes.
    pub fn into_inner(self) -> W {
        self.out
    }

    /// The one and only writer.
    ///
    /// Every byte the program prints in demo mode goes through this method.
    /// It builds the whole text first, then writes it once.
    fn emit(&mut self, text: &str) -> Result<(), RenderError> {
        self.out
            .write_all(text.as_bytes())
            .map_err(|_| RenderError::Unwritable)?;
        self.out.flush().map_err(|_| RenderError::Unwritable)
    }
}

impl<W: Write> Renderer for StrictRenderer<W> {
    fn frame(&mut self, view: &BoardView) -> Result<(), RenderError> {
        let text = frame_text(view);
        self.emit(&text)
    }

    fn announce(&mut self, outcome: Outcome) -> Result<(), RenderError> {
        let mut text = String::from(result_text(outcome));
        text.push('\n');
        self.emit(&text)
    }
}
