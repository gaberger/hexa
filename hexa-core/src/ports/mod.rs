// `inference.rs` declares the inference trait surface.
pub mod inference;
// State port contract (IStatePort + focused sub-traits + DTOs). Relocated from
// hexa-nexus where it was an anomaly — port traits belong in hexa-core with the
// rest (ADR-2606071340 P1). Implemented by the STDB/SQLite adapters.
