//! What one cell shows, and how the game ends.

/// One cell holds one of these. A cell cannot be revealed and flagged at the
/// same time, because an enum holds one value. The bad state does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    Hidden,
    Revealed,
    Flagged,
}

/// The state of the whole game. It is computed, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Won,
    Lost,
}
