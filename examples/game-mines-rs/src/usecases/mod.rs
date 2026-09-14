//! What the game does. It uses the domain and the ports, never an adapter.

pub mod new_game;
pub mod project_view;
pub mod run_game;
pub mod take_command;

pub use new_game::{new_game, EndReason, Fault, GameConfig, GameError, Session};
pub use project_view::project_view;
pub use run_game::run_game;
pub use take_command::{take_command, Step};
