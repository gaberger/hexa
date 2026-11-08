//! Value objects shared across the workspace. Pure data, no I/O.

pub mod api_optimization;
// Which steps install hexa's Claude Code mods (ADR-2610301129).
pub mod claude_mods;
pub mod forge;
pub mod gate_ledger;
pub mod guide;
pub mod intent;
pub mod design;
// What a gate's run proved, read by one fail-closed reader (ADR-2611081151).
pub mod evidence;
pub mod messages;
pub mod project_floor;
pub mod provenance;
pub mod spec;
pub mod tools;
pub mod workflow;
pub mod validation;
