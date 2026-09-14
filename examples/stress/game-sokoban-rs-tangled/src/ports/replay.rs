//! Replaying a recorded run.

use crate::ports::move_recorder::Dir;
// STRESS: violation — ports must not import from usecases.
// The port reaches forward into the application layer for the result type
// it should have declared itself.
use crate::usecases::play::Progress;

pub trait Replay {
    fn replay(&self, moves: &[Dir]) -> Progress;
}
