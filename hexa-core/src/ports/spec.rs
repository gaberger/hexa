//! What a spec checker reads and judges (ADR-2610221540).
//!
//! The coverage rule is the domain's (`domain::spec`). The adapters that check,
//! accept, seal and verify a spec are bound by it, so it is part of their
//! contract, and they reach it here rather than in the domain.

pub use crate::domain::spec::{
    binding_anchor, coverage, judge_binding, judge_seal, numbered_decisions, rebind, test_fns, test_ref, without_tests, Acceptance,
    Amendment, BindingVerdict, Exemption, Finding, Kind, Move, Scenario, SealVerdict, Spec,
};
