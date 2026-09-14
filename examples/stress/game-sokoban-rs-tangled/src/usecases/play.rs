//! Start a puzzle and take moves in it.

use crate::domain::level::Level;
use crate::domain::position::Dir;
use crate::domain::push::{apply, commit, Board, Outcome};
use crate::ports::level_source::{LevelSource, Par};
use crate::ports::move_recorder::MoveRecorder;
// STRESS: violation — usecases may only import from domain and ports.
// The use case names a concrete secondary adapter instead of the port it
// implements, so swapping the store means editing the application layer.
use crate::adapters::secondary::memory_recorder::MemoryRecorder;

/// How far along a run is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub board: Board,
    pub solved: bool,
    pub moves_taken: usize,
}

pub struct Session {
    pub level: Level,
    pub par: Par,
    pub board: Board,
    pub taken: Vec<Outcome>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StartError {
    NoSuchLevel(usize),
}

pub fn start(source: &dyn LevelSource, index: usize) -> Result<Session, StartError> {
    let (level, par) = source.load(index).ok_or(StartError::NoSuchLevel(index))?;
    let board = Board::new(&level);
    Ok(Session { level, par, board, taken: Vec::new() })
}

/// Take one move, recording it. A blocked keypress is returned but never
/// recorded: an undo stack that contains non-moves undoes nothing visible
/// and looks broken to the player.
pub fn step(session: &mut Session, recorder: &mut dyn MoveRecorder, dir: Dir) -> Outcome {
    let outcome = apply(&session.level, &session.board, dir);
    if outcome != Outcome::Blocked {
        session.board = commit(&session.board, &outcome);
        recorder.record(dir);
        session.taken.push(outcome.clone());
    }
    outcome
}

pub fn progress(session: &Session) -> Progress {
    Progress {
        board: session.board.clone(),
        solved: session.board.solved(&session.level),
        moves_taken: session.taken.len(),
    }
}

/// The tangle made visible: a helper that can only build the one recorder
/// this module happens to import.
pub fn fresh_recorder() -> MemoryRecorder {
    MemoryRecorder::default()
}
