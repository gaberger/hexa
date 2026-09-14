//! The human at the keyboard.
//!
//! The prompt goes to the same place as the board, because that is the human's
//! screen and no gate reads it. Complaints go to stderr, so
//! `./run.sh 2>/dev/null` is still playable.

use std::io::{BufRead, Write};

use crate::ports::{BoardView, Choice, Column, Disc, InputError, InputSource, LegalMoves};

/// A player who types a number from 1 to 7.
pub struct StdinInput<R: BufRead, W: Write> {
    reader: R,
    prompt: W,
}

impl<R: BufRead, W: Write> StdinInput<R, W> {
    /// Wrap a place to read from and a place to prompt on.
    pub fn new(reader: R, prompt: W) -> StdinInput<R, W> {
        StdinInput { reader, prompt }
    }

    fn say(&mut self, text: &str) {
        let _ = self.prompt.write_all(text.as_bytes());
        let _ = self.prompt.flush();
    }
}

impl<R: BufRead, W: Write> InputSource for StdinInput<R, W> {
    fn choose(
        &mut self,
        _view: &BoardView,
        legal: &LegalMoves,
        to_move: Disc,
    ) -> Result<Choice, InputError> {
        loop {
            let colour = match to_move {
                Disc::Red => "Red",
                Disc::Yellow => "Yellow",
            };
            self.say(&format!("{colour}, pick a column 1-7 (q to quit): "));

            let mut line = String::new();
            let read = self
                .reader
                .read_line(&mut line)
                .map_err(|_| InputError::Unreadable)?;
            // Zero bytes means end of input. Quit at once. A loop here is the
            // classic hang for this program.
            if read == 0 {
                self.say("\n");
                return Ok(Choice::Quit);
            }

            let word = line.trim();
            if word.eq_ignore_ascii_case("q") || word.eq_ignore_ascii_case("quit") {
                return Ok(Choice::Quit);
            }

            let number = match word.parse::<u8>() {
                Ok(number) if (1..=7).contains(&number) => number,
                _ => {
                    eprintln!("type a number from 1 to 7, or q to quit");
                    continue;
                }
            };

            let column = match Column::new(number - 1) {
                Ok(column) => column,
                Err(_) => {
                    eprintln!("type a number from 1 to 7, or q to quit");
                    continue;
                }
            };

            // A full column costs the player nothing. Ask again.
            if !legal.contains(column) {
                eprintln!("column {number} is full, pick another");
                continue;
            }
            return Ok(Choice::Play(column));
        }
    }
}
