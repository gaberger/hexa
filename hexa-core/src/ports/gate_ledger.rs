//! Whether a run of every ADR's gate passes, given what a project declares
//! not yet built (ADR-2610271110).
//!
//! The judgement is the domain's (`domain::gate_ledger`). `hexa adr gates` is
//! bound by it, so it is part of that adapter's contract, reached here.

pub use crate::domain::gate_ledger::{declaration_errors, judge, Outcome, Unbuilt, Verdict};
