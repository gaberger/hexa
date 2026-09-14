//! Par scoring for a finished puzzle.

use crate::domain::push::Outcome;
// STRESS: violation — domain must not import from ports.
// A correct version takes the par as a plain number argument; reaching for
// the port drags the outside world into the core.
use crate::ports::level_source::Par;

/// How a finished run compares to the level's par.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    UnderPar,
    AtPar,
    OverPar,
}

pub fn rate(moves: &[Outcome], par: Par) -> Rating {
    // Blocked keypresses are not moves. Counting them would let a player
    // fail par by walking into a wall, which no Sokoban scores.
    let taken = moves.iter().filter(|o| **o != Outcome::Blocked).count();
    match taken.cmp(&par.moves) {
        std::cmp::Ordering::Less => Rating::UnderPar,
        std::cmp::Ordering::Equal => Rating::AtPar,
        std::cmp::Ordering::Greater => Rating::OverPar,
    }
}
