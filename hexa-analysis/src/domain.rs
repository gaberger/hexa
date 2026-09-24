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
    pub frontend: Option<super::frontend_checker::FrontendCheckResult>,
    /// How much of what was read could be placed in a layer (ADR-2609241707).
    #[serde(default)]
    pub coverage: Coverage,
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

