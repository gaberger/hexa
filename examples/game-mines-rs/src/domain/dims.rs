//! The size of the board, and every sum that turns a position into a slot.

use crate::domain::coord::Coord;
use crate::domain::errors::ConfigError;

/// The largest board the game accepts. It keeps one bad flag from asking for
/// gigabytes of memory.
///
/// This is public on purpose. `lib.rs` prints it in the help text and in the
/// refusal message, so a player reads the rule instead of meeting it by
/// surprise.
pub const MAX_CELLS: usize = 1_000_000;

/// Width and height. Copy, because it is two small numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dims {
    width: u16,
    height: u16,
}

impl Dims {
    pub fn new(width: u16, height: u16) -> Result<Dims, ConfigError> {
        if width == 0 {
            return Err(ConfigError::ZeroWidth);
        }
        if height == 0 {
            return Err(ConfigError::ZeroHeight);
        }
        let total = usize::from(width)
            .checked_mul(usize::from(height))
            .ok_or(ConfigError::TooLarge)?;
        if total > MAX_CELLS {
            return Err(ConfigError::TooLarge);
        }
        Ok(Dims { width, height })
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn total(&self) -> usize {
        usize::from(self.width).saturating_mul(usize::from(self.height))
    }

    /// The only maker of a `Coord`. It refuses a position off the board.
    pub fn coord(&self, x: u32, y: u32) -> Option<Coord> {
        if x >= u32::from(self.width) || y >= u32::from(self.height) {
            return None;
        }
        let cx = u16::try_from(x).ok()?;
        let cy = u16::try_from(y).ok()?;
        Some(Coord::new(cx, cy))
    }

    /// The one place that turns a position into a flat slot number.
    pub fn index(&self, c: Coord) -> Option<usize> {
        if c.x() >= self.width || c.y() >= self.height {
            return None;
        }
        usize::from(c.y())
            .checked_mul(usize::from(self.width))?
            .checked_add(usize::from(c.x()))
    }

    /// The one place that turns a flat slot number back into a position.
    pub fn from_index(&self, i: usize) -> Option<Coord> {
        if i >= self.total() {
            return None;
        }
        let w = usize::from(self.width);
        let y = i.checked_div(w)?;
        let x = i.checked_rem(w)?;
        self.coord(u32::try_from(x).ok()?, u32::try_from(y).ok()?)
    }

    /// The cells that touch `c`: 3 at a corner, 5 on an edge, 8 inside.
    ///
    /// Every sum happens in `i32` on the column and the row. A neighbour is
    /// never computed from a flat slot number, because slot 26 on a 9 wide
    /// board is the far right of the row above, not the cell to the left.
    pub fn neighbours(&self, c: Coord) -> Vec<Coord> {
        let mut out = Vec::with_capacity(8);
        let cx = i32::from(c.x());
        let cy = i32::from(c.y());
        for dy in -1i32..=1i32 {
            for dx in -1i32..=1i32 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = match cx.checked_add(dx) {
                    Some(v) => v,
                    None => continue,
                };
                let ny = match cy.checked_add(dy) {
                    Some(v) => v,
                    None => continue,
                };
                let ux = match u32::try_from(nx) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let uy = match u32::try_from(ny) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(n) = self.coord(ux, uy) {
                    out.push(n);
                }
            }
        }
        out
    }
}
