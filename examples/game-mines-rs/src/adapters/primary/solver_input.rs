//! The demo player. It sees the view, and nothing else.

use std::io;

use crate::ports::input::InputSource;
use crate::ports::view::{BoardView, Command, Glyph};

/// How the demo player chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Plays by the two safe rules, and guesses when neither one fires.
    Deduce,
    /// Opens cells in order. With one mine or more, it must hit one.
    Reckless,
}

/// A player made of rules. It reads a `BoardView`, so it cannot read the mines.
#[derive(Debug)]
pub struct SolverInput {
    policy: Policy,
    rounds: usize,
    cursor: usize,
}

/// The neighbours of a slot, worked out on the column and the row.
/// A neighbour is never computed from a flat slot number.
fn neighbours(width: u16, height: u16, i: usize) -> Vec<usize> {
    let w = usize::from(width);
    if w == 0 {
        return Vec::new();
    }
    let x = match i.checked_rem(w) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let y = match i.checked_div(w) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(8);
    for dy in -1i64..=1i64 {
        for dx in -1i64..=1i64 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = match i64::try_from(x).ok().and_then(|v| v.checked_add(dx)) {
                Some(v) => v,
                None => continue,
            };
            let ny = match i64::try_from(y).ok().and_then(|v| v.checked_add(dy)) {
                Some(v) => v,
                None => continue,
            };
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= i64::from(height) {
                continue;
            }
            let ni = match usize::try_from(ny)
                .ok()
                .and_then(|v| v.checked_mul(w))
                .and_then(|v| usize::try_from(nx).ok().and_then(|x| v.checked_add(x)))
            {
                Some(v) => v,
                None => continue,
            };
            out.push(ni);
        }
    }
    out
}

fn to_command(view: &BoardView, i: usize, flag: bool) -> Option<Command> {
    let w = usize::from(view.width);
    if w == 0 {
        return None;
    }
    let x = u32::try_from(i.checked_rem(w)?).ok()?;
    let y = u32::try_from(i.checked_div(w)?).ok()?;
    if flag {
        Some(Command::Flag { x, y })
    } else {
        Some(Command::Reveal { x, y })
    }
}

impl SolverInput {
    pub fn new(policy: Policy) -> SolverInput {
        SolverInput {
            policy,
            rounds: 0,
            cursor: 0,
        }
    }

    pub fn rounds(&self) -> usize {
        self.rounds
    }

    /// Rule A, then Rule B, then a guess. Every branch changes a cell, so the
    /// game always moves forward.
    fn deduce(&self, view: &BoardView) -> Option<Command> {
        let total = view.glyphs.len();

        // Rule A: the number of mines still to find equals the number of cells
        // still hidden beside this cell. Then all of them are mines.
        for i in 0..total {
            let count = match view.glyphs.get(i) {
                Some(Glyph::Count(n)) => usize::from(*n),
                _ => continue,
            };
            let nbrs = neighbours(view.width, view.height, i);
            let flagged = nbrs
                .iter()
                .filter(|n| view.glyphs.get(**n) == Some(&Glyph::Flag))
                .count();
            let hidden: Vec<usize> = nbrs
                .iter()
                .copied()
                .filter(|n| view.glyphs.get(*n) == Some(&Glyph::Hidden))
                .collect();
            let need = match count.checked_sub(flagged) {
                Some(v) => v,
                None => continue,
            };
            if need > 0 && need == hidden.len() {
                if let Some(first) = hidden.first() {
                    return to_command(view, *first, true);
                }
            }
        }

        // Rule B: every mine beside this cell already has a flag. Then every
        // other hidden neighbour is safe.
        for i in 0..total {
            let count = match view.glyphs.get(i) {
                Some(Glyph::Count(n)) => usize::from(*n),
                _ => continue,
            };
            let nbrs = neighbours(view.width, view.height, i);
            let flagged = nbrs
                .iter()
                .filter(|n| view.glyphs.get(**n) == Some(&Glyph::Flag))
                .count();
            if flagged != count {
                continue;
            }
            let hidden = nbrs
                .iter()
                .copied()
                .find(|n| view.glyphs.get(*n) == Some(&Glyph::Hidden));
            if let Some(first) = hidden {
                return to_command(view, first, false);
            }
        }

        // Neither rule fired. Open the lowest numbered hidden cell. A guess.
        for i in 0..total {
            if view.glyphs.get(i) == Some(&Glyph::Hidden) {
                return to_command(view, i, false);
            }
        }
        None
    }

    fn reckless(&mut self, view: &BoardView) -> Option<Command> {
        let total = view.glyphs.len();
        while self.cursor < total {
            let i = self.cursor;
            self.cursor = self.cursor.saturating_add(1);
            if view.glyphs.get(i) == Some(&Glyph::Hidden) {
                return to_command(view, i, false);
            }
        }
        None
    }
}

impl InputSource for SolverInput {
    fn next(&mut self, view: &BoardView) -> io::Result<Option<Command>> {
        self.rounds = self.rounds.saturating_add(1);
        let cap = view.glyphs.len().saturating_mul(4).saturating_add(8);
        if self.rounds > cap {
            // A gate that hangs is worse than a gate that fails.
            return Err(io::Error::other("solver round cap reached"));
        }
        match self.policy {
            Policy::Deduce => Ok(self.deduce(view)),
            Policy::Reckless => Ok(self.reckless(view)),
        }
    }
}
