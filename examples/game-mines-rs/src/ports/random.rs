//! Where the mines come from.

/// A random source that cannot give a fair number.
///
/// The port owns this type. It does **not** re-export the domain's
/// `RollError`. An adapter may import ports, and may not import the domain, so
/// the promise the outside world makes must be written here. The use case
/// translates between the two, because the use case is allowed to see both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollFault {
    /// A number below zero was asked for. There is no such number.
    ZeroBound,
    /// The draw cap was hit. A stuck generator fails loudly; it never hangs.
    Stuck,
}

/// Gives a whole number `0 <= v < bound`, with every value equally likely.
pub trait RandomSource {
    fn next_below(&mut self, bound: u32) -> Result<u32, RollFault>;
}
