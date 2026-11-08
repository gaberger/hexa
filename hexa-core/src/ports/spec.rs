//! What a spec checker reads and judges (ADR-2610221540).
//!
//! The coverage rule is the domain's (`domain::spec`). The adapters that check,
//! accept, seal and verify a spec are bound by it, so it is part of their
//! contract, and they reach it here rather than in the domain.

pub use crate::domain::spec::{
    coverage, judge_seal, numbered_decisions, rebind, test_fns, test_ref, without_tests, Acceptance, Amendment, Exemption, Finding,
    Kind, Scenario, SealVerdict, Spec,
};
