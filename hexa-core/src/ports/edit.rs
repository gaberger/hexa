//! What an adapter that writes files for an agent must refuse to touch.
//!
//! The rule is the domain's (`domain::validation`). Every editing adapter —
//! the `code_patch` tool, a safe writer — is bound by it, so it is part of
//! their contract, and they reach it here rather than in the domain.

pub use crate::domain::validation::is_critical_path;
