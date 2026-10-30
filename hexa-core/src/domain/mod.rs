//! Value objects shared across the workspace. Pure data, no I/O.

pub mod api_optimization;
// Which steps install hexa's Claude Code mods (ADR-2610301129).
pub mod claude_mods;
pub mod gate_ledger;
pub mod intent;
pub mod messages;
pub mod provenance;
pub mod spec;
pub mod tools;
pub mod validation;
