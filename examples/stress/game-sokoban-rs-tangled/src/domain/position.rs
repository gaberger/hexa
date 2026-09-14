//! A cell address on the grid. Pure; imports nothing.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    pub row: usize,
    pub col: usize,
}

/// The four moves a player can make. Sokoban has no diagonals: a diagonal
/// push is ambiguous about which box moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// The cell one step along this direction, or `None` at the edge.
    ///
    /// Returning `None` rather than saturating matters: a saturating step
    /// makes the top-left corner its own neighbour, and a box pushed into
    /// it would silently stay put while the move reported success.
    pub fn step(self, from: Pos, rows: usize, cols: usize) -> Option<Pos> {
        let (dr, dc): (isize, isize) = match self {
            Dir::Up => (-1, 0),
            Dir::Down => (1, 0),
            Dir::Left => (0, -1),
            Dir::Right => (0, 1),
        };
        let row = from.row.checked_add_signed(dr)?;
        let col = from.col.checked_add_signed(dc)?;
        if row >= rows || col >= cols {
            return None;
        }
        Some(Pos { row, col })
    }
}
