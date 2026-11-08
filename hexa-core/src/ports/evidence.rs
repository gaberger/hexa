//! What a gate's run proved (ADR-2611081151 §4–§6).
//!
//! The reader is the domain's (`domain::evidence`). Every adapter that runs a
//! gate (`do`, `adr gates`, `loop stage done`, `gate coverage`, `scaffold`) is
//! bound by it, so it is part of their contract, reached here.

pub use crate::domain::evidence::{read_evidence, Evidence, KNOWN_RUNNERS};
