//! The plugs. Plain data and traits only. No logic lives here.

pub mod input;
pub mod random;
pub mod renderer;
pub mod view;

pub use input::InputSource;
pub use random::{RandomSource, RollFault};
pub use renderer::Renderer;
pub use view::{BoardView, Command, Glyph, Notice, Phase};
