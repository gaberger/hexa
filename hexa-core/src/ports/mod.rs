// `inference.rs` declares the inference trait surface.
pub mod inference;
// What an editing adapter may not touch (the domain's critical-path rule).
pub mod edit;
// The spec an ADR compiles to, and its coverage rule (ADR-2610221540).
pub mod spec;
// Which build a binary is, and whether it is behind (ADR-2610270853).
pub mod project_floor;
pub mod provenance;
// The repository's forge: remote mapping, issue and PR texts, the secret scan (ADR-2611031914).
pub mod forge;
// Whether every ADR's gate passes, given what is declared unbuilt (ADR-2610271110).
pub mod gate_ledger;
// Which hexa verb a plain-English request names (ADR-2610271308).
pub mod guide;
pub mod intent;
// What hexa needs from the Claude CLI to install its mods (ADR-2610301129).
pub mod claude_mods;
// State port contract (IStatePort + focused sub-traits + DTOs). Relocated from
// hexa-nexus where it was an anomaly — port traits belong in hexa-core with the
// rest (ADR-2606071340 P1). Implemented by the STDB/SQLite adapters.
