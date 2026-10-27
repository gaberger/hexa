// `inference.rs` declares the inference trait surface.
pub mod inference;
// What an editing adapter may not touch (the domain's critical-path rule).
pub mod edit;
// The spec an ADR compiles to, and its coverage rule (ADR-2610221540).
pub mod spec;
// Which build a binary is, and whether it is behind (ADR-2610270853).
pub mod provenance;
// State port contract (IStatePort + focused sub-traits + DTOs). Relocated from
// hexa-nexus where it was an anomaly — port traits belong in hexa-core with the
// rest (ADR-2606071340 P1). Implemented by the STDB/SQLite adapters.
