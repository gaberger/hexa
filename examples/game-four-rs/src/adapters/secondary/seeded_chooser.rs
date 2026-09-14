//! A player made of arithmetic.
//!
//! The chooser has no tactics. It does not try to win and it does not try to
//! block. Three reasons, in order of weight:
//!
//! 1. A tactic needs to try moves, which means calling the domain's `drop`.
//!    An adapter may not do that, and the architecture grade would fall.
//! 2. Tactics squeeze the variety out. Many seeds would then play one game.
//! 3. The challenge asks for a seeded chooser, not a strong opponent.

use crate::ports::{BoardView, Choice, Disc, InputError, InputSource, LegalMoves};

/// The odd number SplitMix64 walks by.
const STEP: u64 = 0x9E37_79B9_7F4A_7C15;

/// A player that picks a legal column from its own stream of numbers.
///
/// Determinism has hard bans here, and every one of them is a way the seed
/// could stop controlling the game: no clock, no environment variable, no
/// `HashMap` order, no global generator, no check for a terminal, and no
/// outside crate.
pub struct SeededChooser {
    state: u64,
}

impl SeededChooser {
    /// Start the stream at a seed.
    pub fn new(seed: u64) -> SeededChooser {
        SeededChooser { state: seed }
    }

    /// The next number. This is SplitMix64, which mixes the seed itself, so
    /// seed 41 and seed 42 differ from the very first number, and seed 0 is
    /// as good as any other.
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(STEP);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Pick one of `n` things with no bias.
    ///
    /// The rejection happens on the **raw number**, never on the column. A
    /// chooser that picks `number % 7` and asks again when that column is full
    /// can loop for ever once a column fills up. This one always answers with
    /// a legal column on its first try.
    fn below(&mut self, n: u64) -> u64 {
        let floor = (u64::MAX % n + 1) % n;
        loop {
            let x = self.next();
            if x >= floor {
                return x % n;
            }
        }
    }
}

impl InputSource for SeededChooser {
    fn choose(
        &mut self,
        _view: &BoardView,
        legal: &LegalMoves,
        _to_move: Disc,
    ) -> Result<Choice, InputError> {
        let n = legal.len() as u64;
        if n == 0 {
            // The board is full, so the game already ended. Stop rather than
            // invent a move.
            return Ok(Choice::Quit);
        }
        let pick = self.below(n) as usize;
        match legal.iterate().nth(pick) {
            Some(column) => Ok(Choice::Play(column)),
            None => Err(InputError::Unreadable),
        }
    }
}
