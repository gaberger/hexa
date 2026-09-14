//! One turn: ask, drop, draw.

use core::fmt;

use crate::domain::{Game, MoveError, Outcome};
use crate::ports::{Choice, InputError, InputSource, RenderError, Renderer};

/// What one turn left behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnResult {
    /// The game goes on.
    Continued,
    /// The game ended on this move.
    Finished(Outcome),
    /// The player asked to stop.
    Quit,
}

/// What can go wrong in one turn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnError {
    Input(InputError),
    Render(RenderError),
    Move(MoveError),
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TurnError::Input(error) => write!(f, "{error}"),
            TurnError::Render(error) => write!(f, "{error}"),
            TurnError::Move(error) => write!(f, "{error}"),
        }
    }
}

/// Play exactly one turn.
///
/// The order is fixed: ask the player, drop the disc, then draw one frame. The
/// frame is drawn after the drop, so the picture always shows the move that
/// was just made.
pub fn play_turn(
    game: &mut Game,
    input: &mut dyn InputSource,
    out: &mut dyn Renderer,
) -> Result<TurnResult, TurnError> {
    let view = game.view();
    let legal = game.legal_moves();
    let to_move = game.to_move();

    let choice = input
        .choose(&view, &legal, to_move)
        .map_err(TurnError::Input)?;

    let column = match choice {
        Choice::Quit => return Ok(TurnResult::Quit),
        Choice::Play(column) => column,
    };

    let outcome = game.drop(column).map_err(TurnError::Move)?;
    out.frame(&game.view()).map_err(TurnError::Render)?;

    if outcome.is_final() {
        Ok(TurnResult::Finished(outcome))
    } else {
        Ok(TurnResult::Continued)
    }
}
