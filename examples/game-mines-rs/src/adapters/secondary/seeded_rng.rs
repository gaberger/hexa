//! SplitMix64, plus a fair way to cut a big number down to a small range.

use crate::ports::random::{RandomSource, RollFault};

/// How many draws the fair cut may take before it gives up.
pub const DRAW_CAP: usize = 128;

/// Turn raw 64 bit draws into a fair number below `bound`.
///
/// This throws away any draw that would make low numbers more likely. A board
/// with more mines on the left is not a board. If the cap is reached the
/// generator is stuck, and a stuck generator must fail loudly, not hang.
pub fn roll_below(bound: u32, draw: &mut dyn FnMut() -> u64) -> Result<u32, RollFault> {
    if bound == 0 {
        return Err(RollFault::ZeroBound);
    }
    let range = u64::from(bound);
    let span: u64 = 1u64 << 32;
    let limit = span - (span % range); // the largest whole number of ranges
    for _ in 0..DRAW_CAP {
        let v = draw() >> 32;
        if v < limit {
            let picked = v % range;
            return u32::try_from(picked).map_err(|_| RollFault::Stuck);
        }
    }
    Err(RollFault::Stuck)
}

/// A small, fast, seeded generator. Six lines of arithmetic, no dependencies.
#[derive(Debug, Clone)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    pub fn new(seed: u64) -> SeededRng {
        SeededRng { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl RandomSource for SeededRng {
    fn next_below(&mut self, bound: u32) -> Result<u32, RollFault> {
        roll_below(bound, &mut || self.next_u64())
    }
}
