//! The board and the rules of a drop.

use super::column::{Column, COLUMNS, ROWS};
use super::disc::Disc;
use super::errors::MoveError;
use super::moves::MoveList;
use super::outcome::Outcome;
use super::view::{BoardView, LegalMoves, CELLS};

/// The four direction pairs a line can run in: across, up, and the two
/// diagonals. Each is walked forward and backward from the new disc.
const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

/// One game of Connect Four.
///
/// The cells are the only truth. The height of a column, whose turn it is and
/// the list of legal moves are all worked out from them, so no second copy of
/// a fact can drift away from the first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Game {
    cells: [[Option<Disc>; ROWS]; COLUMNS],
    moves: MoveList,
    outcome: Outcome,
}

impl Game {
    /// An empty board, with Red to move.
    pub fn new() -> Game {
        Game {
            cells: [[None; ROWS]; COLUMNS],
            moves: MoveList::new(),
            outcome: Outcome::InProgress,
        }
    }

    /// Drop one disc into a column.
    ///
    /// The order of the checks is a rule. The win is tested before the draw,
    /// because the forty-second disc is allowed to win.
    pub fn drop(&mut self, column: Column) -> Result<Outcome, MoveError> {
        if self.outcome.is_final() {
            return Err(MoveError::GameOver);
        }
        let col = column.index();
        let row = self.height(col);
        if row >= ROWS {
            return Err(MoveError::ColumnFull);
        }

        let mover = self.to_move();
        self.cells[col][row] = Some(mover);
        self.moves.push(column);

        if self.makes_a_line(col, row, mover) {
            self.outcome = Outcome::Win(mover);
        } else if self.moves.len() == CELLS {
            self.outcome = Outcome::Draw;
        } else {
            self.outcome = Outcome::InProgress;
        }

        self.check_invariants();
        Ok(self.outcome)
    }

    /// A flat copy of the board, bottom row first.
    pub fn view(&self) -> BoardView {
        let mut cells = [None; CELLS];
        for (col, column) in self.cells.iter().enumerate() {
            for (row, cell) in column.iter().enumerate() {
                cells[row * COLUMNS + col] = *cell;
            }
        }
        BoardView { cells }
    }

    /// The columns that still have room, left to right.
    pub fn legal_moves(&self) -> LegalMoves {
        let mut open = [None; COLUMNS];
        let mut len = 0;
        for index in 0..COLUMNS {
            if self.height(index) < ROWS {
                if let Ok(column) = Column::new(index as u8) {
                    open[len] = Some(column);
                    len += 1;
                }
            }
        }
        LegalMoves::from_slots(open, len)
    }

    /// Whose turn it is. Red plays the even-numbered moves.
    pub fn to_move(&self) -> Disc {
        if self.moves.len().is_multiple_of(2) {
            Disc::Red
        } else {
            Disc::Yellow
        }
    }

    /// How the game stands.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// How many discs have been played.
    pub fn move_count(&self) -> usize {
        self.moves.len()
    }

    /// The columns played so far, oldest first.
    pub fn history(&self) -> impl Iterator<Item = Column> + '_ {
        self.moves.iterate()
    }

    /// How many discs sit in one column. Derived, never stored.
    pub fn height(&self, column: usize) -> usize {
        if column >= COLUMNS {
            return ROWS;
        }
        self.cells[column]
            .iter()
            .filter(|cell| cell.is_some())
            .count()
    }

    /// Does the new disc sit in a line of four or more?
    ///
    /// Only the new disc can make a new line, so the walk starts there. Four
    /// **or more** counts: one drop can join two groups and make five.
    fn makes_a_line(&self, col: usize, row: usize, disc: Disc) -> bool {
        for (dc, dr) in DIRECTIONS {
            let mut run = 1;
            for sign in [1_i32, -1_i32] {
                run += self.run_length(col, row, dc * sign, dr * sign, disc);
            }
            if run >= 4 {
                return true;
            }
        }
        false
    }

    /// Count the matching discs in one direction, not counting the start.
    ///
    /// The walk uses `i32`, because an unsigned step left from column 0 wraps
    /// to a huge number and then panics.
    fn run_length(&self, col: usize, row: usize, dc: i32, dr: i32, disc: Disc) -> usize {
        let mut count = 0;
        let mut c = col as i32 + dc;
        let mut r = row as i32 + dr;
        while c >= 0 && c < COLUMNS as i32 && r >= 0 && r < ROWS as i32 {
            if self.cells[c as usize][r as usize] != Some(disc) {
                break;
            }
            count += 1;
            c += dc;
            r += dr;
        }
        count
    }

    /// The four promises of section 5 of the spec. These are `assert!`, not
    /// `debug_assert!`, because `run.sh` builds in release mode and release
    /// throws every `debug_assert!` away.
    fn check_invariants(&self) {
        let mut total = 0;
        let mut red = 0;
        let mut yellow = 0;
        for (col, column) in self.cells.iter().enumerate() {
            let mut hole_below = false;
            for cell in column.iter() {
                match cell {
                    Some(disc) => {
                        assert!(!hole_below, "column {col} has a gap under a disc");
                        total += 1;
                        match disc {
                            Disc::Red => red += 1,
                            Disc::Yellow => yellow += 1,
                        }
                    }
                    None => hole_below = true,
                }
            }
        }
        assert!(total <= CELLS, "more than 42 discs on the board");
        assert!(
            total == self.moves.len(),
            "the disc count and the move count disagree"
        );
        assert!(
            red == yellow || red == yellow + 1,
            "the colours are out of turn: {red} red and {yellow} yellow"
        );
    }
}

impl Default for Game {
    fn default() -> Game {
        Game::new()
    }
}
