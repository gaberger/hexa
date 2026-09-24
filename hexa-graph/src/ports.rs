//! What hexa-graph exchanges across its boundary, as contracts.
//!
//! - The extraction contract: what a source extractor (the tree-sitter one in
//!   `extract::code`) hands the graph builder for one file.
//! - The value types hexa-graph's use cases take and return: a
//!   [`KnowledgeGraph`], and the [`NodeKind`] an extracted entity is tagged with.
//!
//! An adapter or a caller reaches these here, not in the domain model
//! (the convention hexa's own scaffold emits).

pub use crate::model::{KnowledgeGraph, NodeKind};

/// A declared entity (function, type, etc.).
#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    pub kind: NodeKind,
    pub line: usize,
}

/// A raw import statement (paths not yet resolved to files).
#[derive(Debug, Clone)]
pub struct RawImport {
    /// The path as written (`./foo`, `crate::a::b`, `net/http`).
    pub raw_path: String,
    /// Imported symbol names (`*` for whole-module).
    pub names: Vec<String>,
    pub line: usize,
}

/// Everything pulled out of a single source file.
#[derive(Debug, Clone, Default)]
pub struct FileExtract {
    pub entities: Vec<Entity>,
    pub imports: Vec<RawImport>,
}
