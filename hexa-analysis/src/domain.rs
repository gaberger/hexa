//! Domain types for architecture analysis.
//!
//! Pure value objects with no external dependencies — these represent the
//! vocabulary of hexagonal architecture analysis (layers, edges, violations).

use serde::{Deserialize, Serialize};
use std::fmt;

// ── Hex Layers ───────────────────────────────────────────

/// The six canonical hexagonal architecture layers, plus special file roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexLayer {
    Domain,
    Ports,
    Usecases,
    AdaptersPrimary,
    AdaptersSecondary,
    Infrastructure,
    CompositionRoot,
    EntryPoint,
    Unknown,
}

impl fmt::Display for HexLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain => write!(f, "domain"),
            Self::Ports => write!(f, "ports"),
            Self::Usecases => write!(f, "usecases"),
            Self::AdaptersPrimary => write!(f, "adapters/primary"),
            Self::AdaptersSecondary => write!(f, "adapters/secondary"),
            Self::Infrastructure => write!(f, "infrastructure"),
            Self::CompositionRoot => write!(f, "composition-root"),
            Self::EntryPoint => write!(f, "entry-point"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Supported Languages ──────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    TypeScript,
    Go,
    Rust,
    Unknown,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_path(path: &str) -> Self {
        if path.ends_with(".ts") || path.ends_with(".tsx")
            || path.ends_with(".js") || path.ends_with(".jsx")
        {
            Self::TypeScript
        } else if path.ends_with(".go") {
            Self::Go
        } else if path.ends_with(".rs") {
            Self::Rust
        } else {
            Self::Unknown
        }
    }
}

// ── Import/Export Primitives ─────────────────────────────

/// A single import statement extracted from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStatement {
    /// Project-relative path of the importing file.
    pub from_file: String,
    /// Raw import path as written in source (e.g. `../ports/index.js`, `crate::domain`).
    pub raw_path: String,
    /// Resolved project-relative path of the imported module.
    pub resolved_path: String,
    /// Individual names imported (e.g. `["Foo", "Bar"]`). Empty for `import *` or namespace imports.
    pub names: Vec<String>,
    /// Source line number (1-based).
    pub line: usize,
}

/// A single exported symbol from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDeclaration {
    /// Project-relative path of the exporting file.
    pub file: String,
    /// Exported symbol name (function, type, const, etc.).
    pub name: String,
    /// Source line number (1-based).
    pub line: usize,
    /// Whether this export is annotated with `@hexa:public`.
    pub hexa_public: bool,
    /// What kind of item this is. The dead-export finder reads it.
    pub kind: ExportKind,
}

/// What kind of item an export is.
///
/// The dead-export finder treats a type differently from a value. A type its
/// own file names again (the return type of a live function, the receiver of
/// a method) is the file's API surface and is not dead. A value used only in
/// its own file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportKind {
    Function,
    Type,
    Value,
    /// A Go method. It is consumed through a receiver and never imported by name.
    Method,
    /// A Rust `impl` block. Not an export. Recorded so `unused_ports` keeps
    /// seeing adapter methods for structural matching.
    Impl,
    /// `export default`.
    Default,
}

// ── Analysis Graph ───────────────────────────────────────

/// A directed edge in the import dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEdge {
    pub from_file: String,
    pub to_file: String,
    pub from_layer: HexLayer,
    pub to_layer: HexLayer,
    /// Raw import path as written in source.
    pub import_path: String,
    /// Source line number of the import statement.
    pub line: usize,
}

// ── Violation & Dead-Export Types ─────────────────────────

/// A hexagonal boundary violation: an import that crosses layers illegally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyViolation {
    pub edge: ImportEdge,
    /// Human-readable rule description (e.g. "adapters must not import from domain directly").
    pub rule: String,
}

/// An export that no other file in the project imports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadExport {
    pub file: String,
    pub export_name: String,
    pub line: usize,
}

// ── Full Analysis Result ─────────────────────────────────

/// Complete architecture analysis output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchAnalysisResult {
    pub violations: Vec<DependencyViolation>,
    pub dead_exports: Vec<DeadExport>,
    pub circular_deps: Vec<Vec<String>>,
    pub orphan_files: Vec<String>,
    pub unused_ports: Vec<String>,
    pub health_score: u8,
    pub file_count: usize,
    pub edge_count: usize,
    /// ADR-056: Frontend hexagonal architecture check results (None if no frontend found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontend: Option<FrontendCheckResult>,
    /// How much of what was read could be placed in a layer (ADR-2609241707).
    #[serde(default)]
    pub coverage: Coverage,
    /// The API contract's errors and unserved ports (ADR-2610092245 §6).
    #[serde(default)]
    pub api: ApiFindings,
}

/// The share of the graded files that have a layer. An import touching an
/// unclassified file is never checked, so the grade says nothing about it —
/// and hexa graded itself A+ with 118 of 144 files in that state, while
/// nothing said so (ADR-2609241707).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub classified: usize,
    pub total: usize,
    /// Every file with no layer, by path — each one a fix.
    pub unclassified: Vec<String>,
}

impl Coverage {
    /// The highest score this coverage can support: the classified share,
    /// rounded down, and never A+ (95) while any file is unclassified.
    pub fn ceiling(&self) -> u8 {
        if self.unclassified.is_empty() || self.total == 0 {
            return 100;
        }
        let pct = self.classified * 100 / self.total;
        u8::try_from(pct.min(94)).unwrap_or(94)
    }
}

impl ArchAnalysisResult {
    /// Compute health score from analysis findings.
    ///
    /// Scoring (matches TypeScript implementation):
    /// - Violations: -10 points each
    /// - Rule errors: -10 points each
    /// - Circular deps: -15 points each
    /// - Dead exports: -1 point each (capped at -20)
    /// - Unused ports: -1 point each (capped at -10)
    ///
    /// `rule_errors` is the count of error-severity findings from the
    /// project's own rules file (ADR-2609211430 §1). It costs what a boundary
    /// violation costs, because it is one: a rule at `severity = "error"` is a
    /// statement that the tree is wrong, and until this term existed
    /// `--exit-code` failed on such a tree while `--grade A` passed and the
    /// tool printed A+. Warnings stay out of the score — that is what
    /// `--strict` is for.
    ///
    /// This crate analyses structure and does not read the rules file, so its
    /// own caller passes 0. The count arrives from whoever owns the rules
    /// file, which keeps the formula in one place rather than letting each
    /// surface subtract its own idea of a penalty.
    pub fn compute_health_score(
        violations: usize,
        circular_deps: usize,
        dead_exports: usize,
        unused_ports: usize,
        rule_errors: usize,
    ) -> u8 {
        let penalty = (violations * 10)
            + (rule_errors * 10)
            + (circular_deps * 15)
            + dead_exports.min(20)
            + unused_ports.min(10);
        // Saturate in `usize`, then narrow. `penalty as u8` wrapped: 26
        // violations is a penalty of 260, which is 4 in a `u8`, so the worst
        // code in the repository scored 96/100 and the score climbed back
        // towards 100 the more violations you added. `saturating_sub` cannot
        // help — the truncation happens before it is called.
        //
        // This is the number hexa prints for every project. It was found by
        // hexa's own narrowing-cast rule.
        let penalty = u8::try_from(penalty.min(100)).unwrap_or(100);
        100u8.saturating_sub(penalty)
    }
}

#[cfg(test)]
mod health_score_tests {
    use super::ArchAnalysisResult as R;

    #[test]
    fn a_clean_project_scores_100() {
        assert_eq!(R::compute_health_score(0, 0, 0, 0, 0), 100);
    }

    #[test]
    fn the_score_never_climbs_as_violations_are_added() {
        let mut previous = 100u8;
        for violations in 0..60 {
            let score = R::compute_health_score(violations, 0, 0, 0, 0);
            assert!(
                score <= previous,
                "score rose from {previous} to {score} at {violations} violations"
            );
            previous = score;
        }
    }

    /// The exact regression: 26 violations is a penalty of 260, which used to
    /// truncate to 4 and report 96/100.
    #[test]
    fn twenty_six_violations_do_not_report_ninety_six() {
        assert_eq!(R::compute_health_score(26, 0, 0, 0, 0), 0);
    }

    #[test]
    fn the_worst_case_floors_at_zero_rather_than_wrapping() {
        assert_eq!(R::compute_health_score(usize::MAX / 100, 0, 0, 0, 0), 0);
    }

    /// ADR-2609211430 §1. A rule the project marked `error` costs what a
    /// boundary violation costs; the two are the same claim about the tree.
    #[test]
    fn a_rule_error_costs_what_a_boundary_violation_costs() {
        assert_eq!(
            R::compute_health_score(0, 0, 0, 0, 1),
            R::compute_health_score(1, 0, 0, 0, 0),
            "ten points either way"
        );
        assert_eq!(R::compute_health_score(0, 0, 0, 0, 1), 90);
        assert_eq!(R::compute_health_score(1, 0, 0, 0, 1), 80, "they add");
    }

    #[test]
    fn rule_errors_saturate_like_every_other_term() {
        assert_eq!(R::compute_health_score(0, 0, 0, 0, 26), 0);
        assert_eq!(R::compute_health_score(0, 0, 0, 0, usize::MAX / 100), 0);
    }
}

/// What one file, or one (language, layer) cell, declares: interfaces,
/// types, implementations, functions (the layer inventory).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ItemCounts {
    pub interfaces: usize,
    pub types: usize,
    /// `None` where the language has no syntax for it (Go).
    pub implementations: Option<usize>,
    pub functions: usize,
}

impl ItemCounts {
    pub(crate) fn add(&mut self, o: &ItemCounts) {
        self.interfaces += o.interfaces;
        self.types += o.types;
        self.implementations = match (self.implementations, o.implementations) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        self.functions += o.functions;
    }
}

// ── Frontend check (ADR-056) ─────────────────────────────

/// Result of a single frontend architecture rule check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendRuleResult {
    /// Rule identifier: "F1", "F2", etc.
    pub id: String,
    /// Human-readable rule name.
    pub name: String,
    /// Whether the rule passed (no violations found).
    pub passed: bool,
    /// Specific violations found for this rule.
    pub violations: Vec<FrontendViolation>,
}

/// A single frontend architecture violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendViolation {
    /// Project-relative file path.
    pub file: String,
    /// Line number (1-based) where the violation was found.
    pub line: usize,
    /// Human-readable description of what was found.
    pub message: String,
}

/// Complete result of all frontend architecture checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendCheckResult {
    /// Per-rule results.
    pub rules: Vec<FrontendRuleResult>,
    /// Overall frontend health score (0–100).
    pub score: u32,
}

// ── Module references (the domain import policy) ─────────

/// A place where source code names a module without an import line.
///
/// ADR-2609211600. `use std::fs;` and `std::fs::read("x")` are the same claim
/// about what a file depends on, and a policy that judged only the first let
/// the second walk past a `deny` that named it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleReference {
    /// The path as written, with any leading `::` removed.
    pub raw_path: String,
    /// 1-based line.
    pub line: usize,
    /// What the reference is, for the caller's own rules.
    pub kind: ReferenceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A path naming a module: `std::fs::read`, `sqlx::PgPool`, `#[tokio::main]`.
    Path,
    /// `extern crate x;`
    ExternCrate,
    /// A module specifier in an expression: `require("x")`, `import("x")`.
    Specifier,
    /// A load whose name is not a literal, so nothing can be judged about it.
    /// `raw_path` is the expression as written.
    ComputedLoad,
}

// ── API contract (ADR-2610092245) ────────────────────────

/// A type as a signature or a field writes it, lowered out of its language.
///
/// `Named` is a reference the contract builder resolves against the
/// project's declarations. `Unsupported` carries the type as written: it is
/// an error wherever it reaches the wire, never an empty schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeRef {
    String,
    Integer,
    Number,
    Boolean,
    Unit,
    Array(Box<TypeRef>),
    Optional(Box<TypeRef>),
    /// String-keyed map.
    Map(Box<TypeRef>),
    Named(String),
    Unsupported(String),
}

/// How a port method reports failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorChannel {
    /// It cannot fail.
    None,
    /// It can fail, with nothing typed to say how: `error`, a rejected promise.
    Opaque,
    /// It fails with a declared type, whose variants may carry statuses.
    Typed(TypeRef),
}

/// One method of a trait or interface, as the parser read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiMethodDecl {
    pub name: String,
    pub line: usize,
    /// Doc text with every `@hexa:` line removed.
    pub doc: String,
    /// The text after `@hexa:api`, when the method is tagged.
    pub tag: Option<String>,
    /// The text after each `@hexa:status`.
    pub statuses: Vec<String>,
    /// Parameters the API sees: no receiver, no context.
    pub params: Vec<(String, TypeRef)>,
    pub returns: TypeRef,
    pub error: ErrorChannel,
}

/// A trait or interface carrying `@hexa:api`, or holding a method that does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiPortDecl {
    pub name: String,
    pub line: usize,
    /// The text after `@hexa:api` on the interface; `None` when only its
    /// methods are tagged, which is an error the builder reports.
    pub tag: Option<String>,
    pub statuses: Vec<String>,
    pub methods: Vec<ApiMethodDecl>,
}

/// A field as it goes over the wire: its wire name is already decided by the
/// language's own rules (serde renames, Go `json:` tags).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecl {
    pub wire_name: String,
    pub ty: TypeRef,
    /// Absent from the wire when empty: `omitempty`, `?:`.
    pub optional: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantDecl {
    pub name: String,
    /// The text after `@hexa:status`, if the variant has one.
    pub status: Option<String>,
    /// No payload, so it serializes as its name.
    pub unit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeBody {
    Struct(Vec<FieldDecl>),
    /// A named type that is another type on the wire: `struct Id(String)`,
    /// `type ID string`.
    Newtype(TypeRef),
    /// Another name for a type: `pub use … as X`, `type X = Y`.
    Alias(TypeRef),
    Enum(Vec<VariantDecl>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDecl {
    pub name: String,
    pub line: usize,
    pub body: TypeBody,
}

/// What one file says about the API: its tagged ports, the types it
/// declares, and the line of every `@hexa:api` tag that is not on a port or
/// a port's method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiFacts {
    pub ports: Vec<ApiPortDecl>,
    pub types: Vec<TypeDecl>,
    pub stray_tags: Vec<usize>,
}

/// One thing wrong with the contract, where it is written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiDiagnostic {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ApiDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamLocation {
    Path,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiParam {
    pub name: String,
    pub location: ParamLocation,
    /// Resolved: no `Named` but a schema name, no `Optional` (see `required`).
    pub ty: TypeRef,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiBody {
    /// One parameter is the body.
    Whole(TypeRef),
    /// Several parameters, gathered into one object: (wire name, type, required).
    Fields(Vec<(String, TypeRef, bool)>),
}

/// One operation of the resolved contract. Every type in it is resolved:
/// `Named` names an entry in [`ApiContract::schemas`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiOperation {
    pub service: String,
    pub port: String,
    pub method_name: String,
    pub operation_id: String,
    pub http_method: String,
    pub path: String,
    pub description: String,
    pub params: Vec<ApiParam>,
    pub body: Option<ApiBody>,
    pub success: u16,
    /// `None` when the method returns nothing.
    pub response: Option<TypeRef>,
    /// Error statuses, sorted.
    pub errors: Vec<u16>,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiSchema {
    /// (wire name, type, required), in declaration order.
    Object(Vec<(String, TypeRef, bool)>),
    /// A unit-only enum: its variant names.
    StringEnum(Vec<String>),
}

/// The language-neutral contract, from which the OpenAPI document is rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiContract {
    pub title: String,
    pub version: String,
    pub operations: Vec<ApiOperation>,
    pub schemas: std::collections::BTreeMap<String, ApiSchema>,
}

/// What `hexa analyze` reports about the API (ADR-2610092245 §6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiFindings {
    pub operations: usize,
    /// Each one costs what a rule error costs.
    pub errors: Vec<ApiDiagnostic>,
    /// Tagged ports no primary adapter names: declared, not served.
    pub unserved: Vec<String>,
}

/// The words of an identifier in any of the three languages' conventions:
/// `saved_at`, `SavedAt` and `savedAt` are all `saved`, `at`; `GetURL` is
/// `get`, `url`.
pub fn split_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let boundary = c.is_uppercase()
            && !cur.is_empty()
            && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if boundary {
            words.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

pub fn capitalize(word: &str) -> String {
    let mut c = word.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

/// One spelling for a name across languages, so the same port written in
/// Rust, Go and TypeScript has the same operation ids: `list_by_tag`,
/// `ListByTag` and `listByTag` are all `listByTag`.
pub fn lower_camel(name: &str) -> String {
    split_words(name)
        .iter()
        .enumerate()
        .map(|(i, w)| if i == 0 { w.clone() } else { capitalize(w) })
        .collect()
}
