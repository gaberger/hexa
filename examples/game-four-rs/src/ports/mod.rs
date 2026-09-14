//! The plugs. Plain data and traits only. No logic lives here.
//!
//! This folder imports the domain and nothing else. It also **re-exports** the
//! domain value types below. That re-export is what lets an adapter import
//! `ports` only and still speak about a `Column` or a `Disc`.

pub mod input;
pub mod renderer;
pub mod request;

pub use input::{Choice, InputError, InputSource};
pub use renderer::{RenderError, Renderer};
pub use request::Request;

pub use crate::domain::{BoardView, Column, Disc, LegalMoves, MoveError, Outcome};
