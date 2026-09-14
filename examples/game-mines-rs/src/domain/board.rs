//! The part of the game that changes: what is open, what is flagged.

use crate::domain::coord::Coord;
use crate::domain::dims::Dims;
use crate::domain::errors::{InvariantError, MoveError};
use crate::domain::layout::Layout;
use crate::domain::status::{CellState, Status};

/// The board in play. The layout is the truth and never changes. The state
/// vector is the play and does change.
#[derive(Debug, Clone)]
pub struct Board {
    layout: Layout,
    state: Vec<CellState>,
    revealed_safe: usize,
    blast: Option<Coord>,
}

impl Board {
    pub fn new(layout: Layout) -> Board {
        let total = layout.dims().total();
        Board {
            layout,
            state: vec![CellState::Hidden; total],
            revealed_safe: 0,
            blast: None,
        }
    }

    pub fn dims(&self) -> Dims {
        self.layout.dims()
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Open a cell, and spread over an empty region.
    ///
    /// The spread uses an explicit stack. Recursion would overflow the call
    /// stack on a large empty region and kill the program.
    pub fn reveal(&mut self, c: Coord) -> Result<(), MoveError> {
        if self.status() != Status::Playing {
            return Err(MoveError::GameOver);
        }
        let dims = self.layout.dims();
        let i = dims.index(c).ok_or(MoveError::OffBoard)?;
        match self.state.get(i) {
            Some(CellState::Flagged) => return Err(MoveError::CellIsFlagged),
            Some(CellState::Revealed) => return Err(MoveError::AlreadyRevealed),
            Some(CellState::Hidden) => {}
            None => return Err(MoveError::OffBoard),
        }

        if self.layout.is_mine(c) {
            if let Some(s) = self.state.get_mut(i) {
                *s = CellState::Revealed;
            }
            self.blast = Some(c);
            // A mine never adds to the win counter.
            return Ok(());
        }

        let mut stack: Vec<Coord> = Vec::new();
        if let Some(s) = self.state.get_mut(i) {
            *s = CellState::Revealed;
        }
        self.revealed_safe = self.revealed_safe.saturating_add(1);
        stack.push(c);

        while let Some(cur) = stack.pop() {
            if self.layout.adjacent(cur) != 0 {
                continue;
            }
            for nb in dims.neighbours(cur) {
                let ni = match dims.index(nb) {
                    Some(v) => v,
                    None => continue,
                };
                if self.state.get(ni) != Some(&CellState::Hidden) {
                    continue;
                }
                // A cell with a count of zero has no mine beside it. The check
                // stays anyway, so a flood can never open a mine.
                if self.layout.is_mine(nb) {
                    continue;
                }
                if let Some(s) = self.state.get_mut(ni) {
                    *s = CellState::Revealed;
                }
                self.revealed_safe = self.revealed_safe.saturating_add(1);
                stack.push(nb);
            }
        }
        Ok(())
    }

    /// Put a flag on, or take a flag off. It is a toggle, both ways.
    pub fn toggle_flag(&mut self, c: Coord) -> Result<(), MoveError> {
        if self.status() != Status::Playing {
            return Err(MoveError::GameOver);
        }
        let i = self.layout.dims().index(c).ok_or(MoveError::OffBoard)?;
        match self.state.get(i) {
            Some(CellState::Revealed) => Err(MoveError::CannotFlagRevealed),
            Some(CellState::Hidden) => {
                if let Some(s) = self.state.get_mut(i) {
                    *s = CellState::Flagged;
                }
                Ok(())
            }
            Some(CellState::Flagged) => {
                if let Some(s) = self.state.get_mut(i) {
                    *s = CellState::Hidden;
                }
                Ok(())
            }
            None => Err(MoveError::OffBoard),
        }
    }

    /// Computed every time. It counts revealed **safe** cells, never revealed
    /// cells. A mine must never add to the win counter.
    pub fn status(&self) -> Status {
        if self.blast.is_some() {
            return Status::Lost;
        }
        let safe_total = self
            .layout
            .dims()
            .total()
            .saturating_sub(self.layout.mine_count());
        if self.revealed_safe >= safe_total {
            Status::Won
        } else {
            Status::Playing
        }
    }

    pub fn cell_state(&self, c: Coord) -> CellState {
        self.layout
            .dims()
            .index(c)
            .and_then(|i| self.state.get(i).copied())
            .unwrap_or(CellState::Hidden)
    }

    pub fn adjacent(&self, c: Coord) -> u8 {
        self.layout.adjacent(c)
    }

    pub fn is_mine(&self, c: Coord) -> bool {
        self.layout.is_mine(c)
    }

    pub fn mine_count(&self) -> usize {
        self.layout.mine_count()
    }

    pub fn flags_placed(&self) -> usize {
        self.state
            .iter()
            .filter(|s| **s == CellState::Flagged)
            .count()
    }

    pub fn revealed_safe(&self) -> usize {
        self.revealed_safe
    }

    pub fn blast(&self) -> Option<Coord> {
        self.blast
    }

    pub fn fingerprint(&self) -> [u8; 8] {
        self.layout.fingerprint()
    }

    /// Every promise the board makes about itself, checked from scratch.
    pub fn check_invariants(&self) -> Result<(), InvariantError> {
        let dims = self.layout.dims();
        let total = dims.total();
        if self.state.len() != total {
            return Err(InvariantError::LengthMismatch);
        }

        let mut mines: usize = 0;
        let mut safe_open: usize = 0;
        let mut mine_open: usize = 0;
        for i in 0..total {
            let c = dims.from_index(i).ok_or(InvariantError::LengthMismatch)?;
            let is_mine = self.layout.is_mine(c);
            if is_mine {
                mines = mines.checked_add(1).ok_or(InvariantError::MineTotal)?;
            }

            let count = self.layout.adjacent(c);
            if count > 8 {
                return Err(InvariantError::AdjacentCount);
            }
            let fresh = dims
                .neighbours(c)
                .into_iter()
                .filter(|n| self.layout.is_mine(*n))
                .count();
            if usize::from(count) != fresh {
                return Err(InvariantError::AdjacentCount);
            }

            if self.state.get(i) == Some(&CellState::Revealed) {
                if is_mine {
                    mine_open = mine_open
                        .checked_add(1)
                        .ok_or(InvariantError::HiddenMineRevealed)?;
                } else {
                    safe_open = safe_open
                        .checked_add(1)
                        .ok_or(InvariantError::RevealedSafe)?;
                }
            }
        }

        if mines != self.layout.mine_count() {
            return Err(InvariantError::MineTotal);
        }
        if safe_open != self.revealed_safe {
            return Err(InvariantError::RevealedSafe);
        }
        if self.blast.is_none() && mine_open != 0 {
            return Err(InvariantError::HiddenMineRevealed);
        }

        let status = self.status();
        if status == Status::Won && self.blast.is_some() {
            return Err(InvariantError::BothEndings);
        }
        Ok(())
    }
}
