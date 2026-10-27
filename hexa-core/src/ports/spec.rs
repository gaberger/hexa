//! What a spec checker reads and judges (ADR-2610221540).
//!
//! The coverage rule is the domain's (`domain::spec`). The adapters that check,
//! accept, seal and verify a spec are bound by it, so it is part of their
//! contract, and they reach it here rather than in the domain.

pub use crate::domain::spec::{
    coverage, numbered_decisions, test_ref, Acceptance, Exemption, Finding, Kind, Scenario, Spec,
};
