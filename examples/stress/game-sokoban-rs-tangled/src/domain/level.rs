//! The static part of a puzzle: walls, floors and goals.

use crate::domain::position::Pos;
// STRESS: cycle — level and push import each other. Rust allows it; a
// hexagonal grader should still report the cycle.
use crate::domain::push::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Wall,
    Floor,
    Goal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    tiles: Vec<Vec<Tile>>,
    pub start: Pos,
    pub boxes: Vec<Pos>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LevelError {
    Ragged,
    NoPlayer,
    /// A puzzle with more boxes than goals can never be solved, and one
    /// with more goals than boxes is solved by a subset — both are
    /// authoring mistakes, not states to play from.
    BoxesAndGoalsDiffer { boxes: usize, goals: usize },
}

impl Level {
    /// Parse the conventional Sokoban charset:
    /// `#` wall, ` ` floor, `.` goal, `@` player, `$` box, `*` box on goal,
    /// `+` player on goal.
    pub fn parse(text: &str) -> Result<Level, LevelError> {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let width = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        let mut tiles = Vec::new();
        let mut start = None;
        let mut boxes = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            // Trailing spaces are routinely trimmed by editors, so a short
            // line is padded with floor rather than rejected as ragged.
            let mut cells = Vec::with_capacity(width);
            for col in 0..width {
                let ch = line.as_bytes().get(col).copied().unwrap_or(b' ') as char;
                let tile = match ch {
                    '#' => Tile::Wall,
                    '.' | '*' | '+' => Tile::Goal,
                    ' ' | '@' | '$' => Tile::Floor,
                    _ => return Err(LevelError::Ragged),
                };
                if ch == '@' || ch == '+' {
                    start = Some(Pos { row, col });
                }
                if ch == '$' || ch == '*' {
                    boxes.push(Pos { row, col });
                }
                cells.push(tile);
            }
            tiles.push(cells);
        }
        let start = start.ok_or(LevelError::NoPlayer)?;
        let goals = tiles.iter().flatten().filter(|t| **t == Tile::Goal).count();
        if goals != boxes.len() {
            return Err(LevelError::BoxesAndGoalsDiffer { boxes: boxes.len(), goals });
        }
        Ok(Level { tiles, start, boxes })
    }

    pub fn rows(&self) -> usize {
        self.tiles.len()
    }

    pub fn cols(&self) -> usize {
        self.tiles.first().map_or(0, Vec::len)
    }

    pub fn tile(&self, at: Pos) -> Tile {
        self.tiles
            .get(at.row)
            .and_then(|r| r.get(at.col))
            .copied()
            // Off-grid reads as wall. A puzzle with no border would
            // otherwise let the player walk off the edge of the vector.
            .unwrap_or(Tile::Wall)
    }

    pub fn goals(&self) -> Vec<Pos> {
        let mut out = Vec::new();
        for (row, line) in self.tiles.iter().enumerate() {
            for (col, t) in line.iter().enumerate() {
                if *t == Tile::Goal {
                    out.push(Pos { row, col });
                }
            }
        }
        out
    }

    /// Part of the cycle: a convenience the push rules hand back to the
    /// level so a caller can ask "did that land a box home?".
    pub fn landed_home(&self, outcome: &Outcome) -> bool {
        match outcome {
            Outcome::Pushed { box_to, .. } => self.tile(*box_to) == Tile::Goal,
            _ => false,
        }
    }
}
