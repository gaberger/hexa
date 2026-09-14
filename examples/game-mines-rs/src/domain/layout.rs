//! Where the mines are. Fixed from the moment it is made.

use crate::domain::coord::Coord;
use crate::domain::dims::Dims;
use crate::domain::errors::{PlacementError, RollError};

/// The truth of the board. It never changes after it is made, so no count is
/// ever patched and no mine ever moves.
#[derive(Debug, Clone)]
pub struct Layout {
    dims: Dims,
    mine: Vec<bool>,
    adjacent: Vec<u8>,
}

impl Layout {
    /// Place `mine_count` mines, never on a cell in `exclude`.
    ///
    /// The domain never imports the random port. It takes a closure, and the
    /// use case builds that closure from the port.
    pub fn place(
        dims: Dims,
        mine_count: usize,
        exclude: &[Coord],
        roll: &mut dyn FnMut(u32) -> Result<u32, RollError>,
    ) -> Result<Layout, PlacementError> {
        let total = dims.total();
        let mut blocked = vec![false; total];
        for c in exclude {
            if let Some(i) = dims.index(*c) {
                if let Some(slot) = blocked.get_mut(i) {
                    *slot = true;
                }
            }
        }

        let mut slots: Vec<usize> = Vec::with_capacity(total);
        for i in 0..total {
            if blocked.get(i).copied() == Some(false) {
                slots.push(i);
            }
        }

        let n = slots.len();
        if mine_count > n {
            return Err(PlacementError::TooManyMines);
        }

        // A partial Fisher-Yates shuffle. It draws exactly `mine_count`
        // numbers and always stops. A "pick again if it repeats" loop has no
        // upper bound.
        for i in 0..mine_count {
            let remaining = n.checked_sub(i).ok_or(PlacementError::Internal)?;
            let bound = u32::try_from(remaining).map_err(|_| PlacementError::Internal)?;
            let r = roll(bound).map_err(PlacementError::Roll)?;
            let step = usize::try_from(r).map_err(|_| PlacementError::Internal)?;
            if step >= remaining {
                // The generator gave a number outside the range it was asked
                // for. Fail loudly rather than fold it back in silently.
                return Err(PlacementError::Roll(RollError::Stuck));
            }
            let j = i.checked_add(step).ok_or(PlacementError::Internal)?;
            if j >= n {
                return Err(PlacementError::Internal);
            }
            slots.swap(i, j);
        }

        let mut mines: Vec<Coord> = Vec::with_capacity(mine_count);
        for k in 0..mine_count {
            let idx = *slots.get(k).ok_or(PlacementError::Internal)?;
            mines.push(dims.from_index(idx).ok_or(PlacementError::Internal)?);
        }
        Layout::from_mines(dims, &mines)
    }

    /// Build a layout from a known mine set. Used by the placer above and by
    /// hand drawn fixtures in the tests.
    pub fn from_mines(dims: Dims, mines: &[Coord]) -> Result<Layout, PlacementError> {
        let total = dims.total();
        let mut mine = vec![false; total];
        let mut placed: usize = 0;
        for c in mines {
            let i = dims.index(*c).ok_or(PlacementError::Internal)?;
            let slot = mine.get_mut(i).ok_or(PlacementError::Internal)?;
            if !*slot {
                *slot = true;
                placed = placed.checked_add(1).ok_or(PlacementError::Internal)?;
            }
        }
        if placed > total {
            return Err(PlacementError::TooManyMines);
        }

        // The counts come from the final mine set. Walk each mine once and add
        // one to each of its neighbours.
        let mut adjacent = vec![0u8; total];
        for i in 0..total {
            if mine.get(i).copied() != Some(true) {
                continue;
            }
            let c = dims.from_index(i).ok_or(PlacementError::Internal)?;
            for nb in dims.neighbours(c) {
                let ni = dims.index(nb).ok_or(PlacementError::Internal)?;
                let slot = adjacent.get_mut(ni).ok_or(PlacementError::Internal)?;
                *slot = slot.checked_add(1).ok_or(PlacementError::Internal)?;
            }
        }

        Ok(Layout {
            dims,
            mine,
            adjacent,
        })
    }

    pub fn dims(&self) -> Dims {
        self.dims
    }

    pub fn is_mine(&self, c: Coord) -> bool {
        self.dims
            .index(c)
            .and_then(|i| self.mine.get(i).copied())
            .unwrap_or(false)
    }

    pub fn adjacent(&self, c: Coord) -> u8 {
        self.dims
            .index(c)
            .and_then(|i| self.adjacent.get(i).copied())
            .unwrap_or(0)
    }

    pub fn mine_count(&self) -> usize {
        self.mine.iter().filter(|m| **m).count()
    }

    /// FNV-1a over the size and the mine bits. Two different boards give two
    /// different lines on the screen, so the gate can see that the seed works.
    pub fn fingerprint(&self) -> [u8; 8] {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |byte: u8| {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for b in self.dims.width().to_be_bytes() {
            eat(b);
        }
        for b in self.dims.height().to_be_bytes() {
            eat(b);
        }
        for m in &self.mine {
            eat(if *m { 1u8 } else { 0u8 });
        }
        hash.to_be_bytes()
    }
}
