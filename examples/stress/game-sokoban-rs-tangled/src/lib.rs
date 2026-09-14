//! game-sokoban-rs-tangled — a working Sokoban with a deliberately tangled
//! architecture.
//!
//! Everything here compiles and every test passes. The only thing wrong
//! with it is the shape, which is the point: this fixture exists to prove
//! that "it builds and the tests are green" is a different claim from "the
//! architecture is sound", and that `hexa analyze` can tell them apart.
//!
//! The five violations are marked `STRESS: violation` at their import
//! lines. See README.md for the list and for what hexa actually reported.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

use adapters::secondary::builtin_levels::BuiltinLevels;
use adapters::secondary::memory_recorder::MemoryRecorder;
use ports::level_source::LevelSource;
use ports::move_recorder::MoveRecorder;

/// The composition root: the one place allowed to name concrete adapters
/// and hand them to the application layer as ports.
pub struct Game {
    pub source: Box<dyn LevelSource>,
    pub recorder: Box<dyn MoveRecorder>,
}

impl Game {
    pub fn new() -> Game {
        Game {
            source: Box::new(BuiltinLevels),
            recorder: Box::new(MemoryRecorder::default()),
        }
    }
}

impl Default for Game {
    fn default() -> Self {
        Game::new()
    }
}
