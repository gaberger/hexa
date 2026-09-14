//! The rules. This folder imports nothing but itself.
//!
//! The sign convention, written once and obeyed everywhere:
//!
//! * Column 0 is the left column. Column 6 is the right column.
//! * Row 0 is the floor. Row 5 is the top.
//! * Gravity pulls a disc toward row 0.
//!
//! The screen prints row 5 first and row 0 last. The renderer does that flip,
//! and it is the only flip in the program.

pub mod board;
pub mod column;
pub mod disc;
pub mod errors;
pub mod moves;
pub mod outcome;
pub mod view;

pub use board::Game;
pub use column::{Column, COLUMNS, ROWS};
pub use disc::Disc;
pub use errors::MoveError;
pub use moves::MoveList;
pub use outcome::Outcome;
pub use view::{BoardView, LegalMoves, CELLS};
