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

/// How a file came to name another module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportKind {
    /// An import statement: `use`, `import`, a re-export.
    #[default]
    Use,
    /// A Rust `mod x;` declaration. It wires a module into the tree; it does
    /// not use any of the module's items.
    ModDecl,
    /// A fully qualified path in an expression or type
    /// (`crate::domain::util::half()`). It names an item without importing it,
    /// so it links only when the path resolves to a file that defines the item.
    PathExpr,
    /// The identifiers a Go file names. Files of one directory that declare
    /// one package clause are one Go package and use each other's names
    /// without an import, so each links to the entity of that exact name
    /// another Go file of the same directory and package defines
    /// (ADR-2611221643: `hexa graph consumers` sees a grouped var's users).
    /// A method is never a target: a bare name does not denote one.
    SamePackage,
}

/// A raw import statement (paths not yet resolved to files).
#[derive(Debug, Clone)]
pub struct RawImport {
    /// The path as written (`./foo`, `crate::a::b`, `net/http`).
    pub raw_path: String,
    /// Imported symbol names (`*` for whole-module).
    pub names: Vec<String>,
    pub line: usize,
    pub kind: ImportKind,
}

/// Everything pulled out of a single source file.
#[derive(Debug, Clone, Default)]
pub struct FileExtract {
    pub entities: Vec<Entity>,
    pub imports: Vec<RawImport>,
    /// A Go file's package clause (`package p` → `p`).
    pub go_package: Option<String>,
    /// The methods a Go file declares and no function of the file shares a
    /// name with. They are entities, but no same-package link targets one.
    pub go_methods: Vec<String>,
    /// (type name, key) for each bare-identifier key of a Go composite
    /// literal whose type is a named type. It is a field name if the package
    /// declares that type a struct, else an expression that may name a
    /// same-package entity, so the graph links it unless the type is a struct
    /// of the package. A qualified type (`pkg.T`) has an empty name.
    pub go_literal_keys: Vec<(String, String)>,
}
