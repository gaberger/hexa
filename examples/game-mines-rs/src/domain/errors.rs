//! Every way the game can say "no". No error holds free text.

/// A board that cannot exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    ZeroWidth,
    ZeroHeight,
    TooLarge,
    TooManyMines,
}

/// A random source that cannot give a fair number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollError {
    ZeroBound,
    /// The draw cap was hit. A stuck generator fails loudly; it never hangs.
    Stuck,
}

/// Mines could not be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    TooManyMines,
    Roll(RollError),
    /// A position the domain built for itself did not fit. This must not happen.
    Internal,
}

/// A move the board refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveError {
    GameOver,
    OffBoard,
    AlreadyRevealed,
    CellIsFlagged,
    CannotFlagRevealed,
}

/// A promise the board makes about itself that did not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvariantError {
    LengthMismatch,
    MineTotal,
    AdjacentCount,
    RevealedSafe,
    HiddenMineRevealed,
    BothEndings,
}
