//! Which hexa verb a plain-English request names (ADR-2610271308).
//!
//! The routing table is the domain's (`domain::intent`). `hexa hey` is bound
//! by it, so it is part of that adapter's contract, reached here.

pub use crate::domain::intent::{hexa_commands, route, Route, Routed};
