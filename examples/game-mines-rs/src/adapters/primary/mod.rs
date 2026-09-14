//! Adapters that drive the game: what the player sees and what the player types.

pub mod quiet_renderer;
pub mod solver_input;
pub mod stdin_input;
pub mod text;
pub mod terminal_renderer;

pub use quiet_renderer::QuietRenderer;
pub use solver_input::{Policy, SolverInput};
pub use stdin_input::StdinInput;
pub use terminal_renderer::TerminalRenderer;
