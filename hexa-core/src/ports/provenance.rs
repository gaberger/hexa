//! Which build a binary is and whether it is behind (ADR-2610270853).
//!
//! The judgement is the domain's (`domain::provenance`). The adapters that
//! read git and the release feed are bound by it, so it is part of their
//! contract, and they reach it here rather than in the domain.

pub use crate::domain::provenance::{advice, staleness, Build, Checkout, Staleness};
