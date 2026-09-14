//! Reads one line at a time. The adapter never prints anything.

use std::io::{self, BufRead};

use crate::ports::input::InputSource;
use crate::ports::view::{BoardView, Command};

/// Turn one typed line into a command. A line it does not understand becomes
/// `Command::Unknown`, so the loop can say so. The adapter itself is silent.
pub fn parse_line(line: &str) -> Command {
    let mut parts = line.split_whitespace();
    let head = match parts.next() {
        Some(h) => h,
        None => return Command::Unknown,
    };
    let verb = head.to_ascii_lowercase();
    match verb.as_str() {
        "q" | "quit" | "exit" => Command::Quit,
        "r" | "reveal" | "f" | "flag" => {
            let x = match parts.next().and_then(|t| t.parse::<u32>().ok()) {
                Some(v) => v,
                None => return Command::Unknown,
            };
            let y = match parts.next().and_then(|t| t.parse::<u32>().ok()) {
                Some(v) => v,
                None => return Command::Unknown,
            };
            if parts.next().is_some() {
                return Command::Unknown;
            }
            if verb == "r" || verb == "reveal" {
                Command::Reveal { x, y }
            } else {
                Command::Flag { x, y }
            }
        }
        _ => Command::Unknown,
    }
}

/// Reads commands from any line source. Line input only: the program never
/// puts the terminal into raw mode.
#[derive(Debug)]
pub struct StdinInput<R: BufRead> {
    reader: R,
}

impl<R: BufRead> StdinInput<R> {
    pub fn new(reader: R) -> StdinInput<R> {
        StdinInput { reader }
    }
}

impl<R: BufRead> InputSource for StdinInput<R> {
    fn next(&mut self, _view: &BoardView) -> io::Result<Option<Command>> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(None); // the input ended
        }
        Ok(Some(parse_line(&line)))
    }
}
