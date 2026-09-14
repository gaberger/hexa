//! Where puzzles come from.

// A port re-exports the domain types its callers need. That is the intended
// shape: an adapter imports the port, never the domain (CLAUDE.md, rule 4).
pub use crate::domain::level::{Level, LevelError, Tile};
pub use crate::domain::position::Pos;

/// The move count a level is meant to be beaten in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Par {
    pub moves: usize,
}

pub trait LevelSource {
    fn count(&self) -> usize;
    fn load(&self, index: usize) -> Option<(Level, Par)>;
}
