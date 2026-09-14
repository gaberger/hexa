//! The work of the game. This folder imports the domain and the ports only.

pub mod play_turn;

pub use play_turn::{play_turn, TurnError, TurnResult};
