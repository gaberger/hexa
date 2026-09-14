//! Where a run's moves are written.

pub use crate::domain::position::Dir;

pub trait MoveRecorder {
    fn record(&mut self, dir: Dir);
    fn history(&self) -> Vec<Dir>;
    fn rewind(&mut self) -> Option<Dir>;
}
