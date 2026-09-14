//! The move rules. This is the whole game: everything else is plumbing.

use crate::domain::level::{Level, Tile};
use crate::domain::position::{Dir, Pos};

/// What a single keypress did.
///
/// `Blocked` is a distinct answer from `Walked`, not an error: a player
/// walking into a wall is ordinary play, and collapsing the two would make
/// an undo stack record moves that never happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Walked { to: Pos },
    Pushed { to: Pos, box_from: Pos, box_to: Pos },
    Blocked,
}

/// The live state of a puzzle: where the player is and where the boxes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub player: Pos,
    pub boxes: Vec<Pos>,
}

impl Board {
    pub fn new(level: &Level) -> Board {
        let mut boxes = level.boxes.clone();
        boxes.sort();
        Board { player: level.start, boxes }
    }

    pub fn has_box(&self, at: Pos) -> bool {
        self.boxes.binary_search(&at).is_ok()
    }

    /// Every box stands on a goal.
    pub fn solved(&self, level: &Level) -> bool {
        // A level whose goal count differs from its box count cannot reach
        // here: `Level::parse` rejects it.
        self.boxes.iter().all(|b| level.tile(*b) == Tile::Goal)
    }
}

/// Apply one move. Pure: takes the world, returns what happened.
pub fn apply(level: &Level, board: &Board, dir: Dir) -> Outcome {
    let (rows, cols) = (level.rows(), level.cols());
    let Some(ahead) = dir.step(board.player, rows, cols) else {
        return Outcome::Blocked;
    };
    if level.tile(ahead) == Tile::Wall {
        return Outcome::Blocked;
    }
    if !board.has_box(ahead) {
        return Outcome::Walked { to: ahead };
    }
    // A box is in the way. It moves only if the cell beyond it is free,
    // and a box never pushes a second box: two-box pushes are the classic
    // Sokoban bug and make otherwise-dead puzzles solvable.
    let Some(beyond) = dir.step(ahead, rows, cols) else {
        return Outcome::Blocked;
    };
    if level.tile(beyond) == Tile::Wall || board.has_box(beyond) {
        return Outcome::Blocked;
    }
    Outcome::Pushed { to: ahead, box_from: ahead, box_to: beyond }
}

/// Fold an outcome back into a board. Separate from `apply` so a caller can
/// look at a move before taking it.
pub fn commit(board: &Board, outcome: &Outcome) -> Board {
    match outcome {
        Outcome::Blocked => board.clone(),
        Outcome::Walked { to } => Board { player: *to, boxes: board.boxes.clone() },
        Outcome::Pushed { to, box_from, box_to } => {
            let mut boxes = board.boxes.clone();
            if let Ok(i) = boxes.binary_search(box_from) {
                boxes.remove(i);
            }
            let at = boxes.binary_search(box_to).unwrap_or_else(|i| i);
            boxes.insert(at, *box_to);
            Board { player: *to, boxes }
        }
    }
}
