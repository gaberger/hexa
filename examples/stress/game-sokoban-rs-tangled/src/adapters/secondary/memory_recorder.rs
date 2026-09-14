//! An in-memory move recorder.

// STRESS: violation — adapters must not import from domain directly.
// `ports::move_recorder` re-exports `Dir` for exactly this purpose; the
// adapter goes around it and grows a second edge into the core.
use crate::domain::position::Dir;
use crate::ports::move_recorder::MoveRecorder;

#[derive(Debug, Default, Clone)]
pub struct MemoryRecorder {
    moves: Vec<Dir>,
}

impl MoveRecorder for MemoryRecorder {
    fn record(&mut self, dir: Dir) {
        self.moves.push(dir);
    }

    fn history(&self) -> Vec<Dir> {
        self.moves.clone()
    }

    fn rewind(&mut self) -> Option<Dir> {
        self.moves.pop()
    }
}
