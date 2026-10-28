//! The project floor's contract (ADR-2610281552): which ledger a citation
//! names, and what a new project ignores. Re-exported for the adapters and
//! the build, which may reach the domain only through a port.

pub use crate::domain::project_floor::{adr_ids, cited_ledger, gitignore_lines, merged_gitignore, CitedLedger};
