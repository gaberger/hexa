//! The rules of the game. This module imports nothing outside `domain/`.
//!
//! The lints below are denied for the whole domain tree. The game must never
//! stop because of a panic, an index out of range, or an overflow.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod board;
pub mod coord;
pub mod dims;
pub mod errors;
pub mod layout;
pub mod status;

pub use board::Board;
pub use coord::Coord;
pub use dims::{Dims, MAX_CELLS};
pub use errors::{ConfigError, InvariantError, MoveError, PlacementError, RollError};
pub use layout::Layout;
pub use status::{CellState, Status};
