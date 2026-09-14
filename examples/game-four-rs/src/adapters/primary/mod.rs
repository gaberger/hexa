//! Things that drive the game from outside.

pub mod cli;
pub mod human_renderer;
pub mod stdin_input;
pub mod strict_renderer;

pub use cli::{parse, UsageError, USAGE};
pub use human_renderer::HumanRenderer;
pub use stdin_input::StdinInput;
pub use strict_renderer::StrictRenderer;
