//! Start a game, and hold the things one game needs.

use std::io;

use crate::domain::board::Board;
use crate::domain::dims::Dims;
use crate::domain::errors::{ConfigError, InvariantError, PlacementError};

/// The board the player asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameConfig {
    pub width: u16,
    pub height: u16,
    pub mine_count: usize,
}

impl Default for GameConfig {
    fn default() -> GameConfig {
        GameConfig {
            width: 9,
            height: 9,
            mine_count: 10,
        }
    }
}

/// How a game ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Won,
    Lost,
    Quit,
}

/// Something inside the program broke. This is never shown as a game message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Placement(PlacementError),
    Invariant(InvariantError),
}

/// A game that could not finish.
#[derive(Debug)]
pub enum GameError {
    Io(io::Error),
    Fault(Fault),
}

impl From<io::Error> for GameError {
    fn from(e: io::Error) -> GameError {
        GameError::Io(e)
    }
}

/// One game in progress.
///
/// The board is absent until the first reveal. That is what makes the first
/// click always safe: the mines are placed around it, and nothing is repaired
/// afterwards.
#[derive(Debug)]
pub struct Session {
    dims: Dims,
    mine_count: usize,
    board: Option<Board>,
    /// Flags put down before the first reveal, when no board exists yet.
    pre_flags: Vec<bool>,
}

impl Session {
    pub fn dims(&self) -> Dims {
        self.dims
    }

    pub fn mine_count(&self) -> usize {
        self.mine_count
    }

    pub fn board(&self) -> Option<&Board> {
        self.board.as_ref()
    }

    pub(crate) fn board_mut(&mut self) -> Option<&mut Board> {
        self.board.as_mut()
    }

    pub(crate) fn set_board(&mut self, b: Board) {
        self.board = Some(b);
    }

    pub(crate) fn pre_flags(&self) -> &[bool] {
        &self.pre_flags
    }

    pub(crate) fn toggle_pre_flag(&mut self, i: usize) -> bool {
        match self.pre_flags.get_mut(i) {
            Some(slot) => {
                *slot = !*slot;
                true
            }
            None => false,
        }
    }

    pub fn fingerprint(&self) -> [u8; 8] {
        match &self.board {
            Some(b) => b.fingerprint(),
            None => [0u8; 8],
        }
    }
}

/// Build a game, or say why the board cannot exist.
///
/// A board needs nine free cells for the first click and its neighbours. So
/// `mine_count` may never be larger than `total - 9`.
pub fn new_game(cfg: GameConfig) -> Result<Session, ConfigError> {
    let dims = Dims::new(cfg.width, cfg.height)?;
    let total = dims.total();
    let room = total.saturating_sub(9);
    if cfg.mine_count > room {
        return Err(ConfigError::TooManyMines);
    }
    Ok(Session {
        dims,
        mine_count: cfg.mine_count,
        board: None,
        pre_flags: vec![false; total],
    })
}
