//! Plain data that crosses the boundary. An adapter reads this and never
//! touches the domain.

/// What one cell shows. `Mine` and `Blast` appear only after a loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Hidden,
    Flag,
    Empty,
    Count(u8),
    Mine,
    Blast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Playing,
    Won,
    Lost,
}

/// One frame of the game, copied out of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardView {
    pub width: u16,
    pub height: u16,
    pub glyphs: Vec<Glyph>,
    pub mines_total: usize,
    pub flags_placed: usize,
    pub revealed_safe: usize,
    pub phase: Phase,
}

/// Every message the game can show.
///
/// A `Notice` holds no string. So the renderer has no way to print your bytes,
/// and an escape sequence in your typing cannot reach your terminal. There is
/// no path for it. The compiler holds this rule, not a promise in a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Welcome,
    Prompt,
    BadCommand,
    OffBoard,
    AlreadyRevealed,
    CellIsFlagged,
    CannotFlagRevealed,
    GameOver,
    YouWin,
    Quit,
    Fingerprint([u8; 8]),
    Stats { revealed: usize, flags: usize },
}

/// What the player asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Reveal { x: u32, y: u32 },
    Flag { x: u32, y: u32 },
    Quit,
    Unknown,
}
