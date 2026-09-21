//! What a layer is allowed to import from outside the project.
//!
//! The dependency rule is checked edge by edge *between layers*, so an import
//! that leaves the project entirely is invisible to it: a domain file that
//! imports `sqlx` has no layer edge to violate and scored A+
//! (ADR-2609211430 §2). The headline rule — "domain imports only domain" — was
//! stricter than what was enforced.
//!
//! This module is the classification half, and it is pure. Reading the rules
//! file, walking the tree and parsing imports belong to the caller; deciding
//! whether `std::fs::File` is standard library, whether `serde_json` is
//! covered by an `allow` of `serde`, and whether either is permitted does not,
//! and is the part worth testing directly.
//!
//! Line matching is the wrong instrument for the same job: `use sqlx::{self,\n
//! PgPool}` split over two lines, a TypeScript `import type`, and a Go
//! `import ( … )` block all defeat a substring pattern, and the analyzer
//! already parses every one of them with tree-sitter to build layer edges.

use super::domain::Language;

/// Where an import comes from, relative to the project doing the importing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Resolves inside this project. Layer edges already govern it, so an
    /// import policy says nothing about it.
    Internal,
    /// The language's own standard library.
    StandardLibrary,
    /// Everything else: a crate, a package, a module from outside.
    External,
}

/// What the project calls itself, which is the only way to tell an import of
/// its own code from an import of somebody else's.
#[derive(Clone, Debug, Default)]
pub struct ProjectNames {
    /// Rust package names with `-` normalised to `_`, as they appear in a
    /// `use` path. A workspace has one per member.
    pub rust_crates: Vec<String>,
    /// The `module` line from `go.mod`.
    pub go_module: Option<String>,
    /// TypeScript `compilerOptions.paths` keys, with any trailing `/*`
    /// removed — `@app/*` is stored as `@app`.
    pub ts_aliases: Vec<String>,
    /// Rust dependency **keys** from the manifest, `-` normalised to `_`
    /// (ADR-2609211600). The key is used rather than `package =`, because the
    /// key is the name code writes: `pg = { package = "tokio-postgres" }` is
    /// referenced as `pg::`.
    pub rust_dependencies: Vec<String>,
}

/// Does this first path segment name something outside the project?
///
/// Only a path that starts with a known-outside name may be judged, and this
/// is why. `O::new()`, `Self::make()`, `Ordering::Less` after a `use`, and
/// `util::f()` for a local `mod util` all have a first segment that
/// [`classify`] would otherwise call `External` — it classifies by shape, and
/// their shape is identical to a crate's. Without the manifest to say which
/// names are dependencies, judging inline paths would report a project's own
/// code as an outside dependency, which is worse than the gap it closes.
///
/// `crate`, `self`, `super` and `Self` are absent from the manifest and so are
/// false by construction, rather than by a list that could fall out of date.
pub fn names_external(first_segment: &str, names: &ProjectNames) -> bool {
    let first = first_segment.trim();
    if first.is_empty() {
        return false;
    }
    if matches!(first, "std" | "core" | "alloc") {
        return true;
    }
    let normalised = first.replace('-', "_");
    names.rust_dependencies.iter().any(|d| d == &normalised)
}

/// The separator that divides one segment of a module path from the next.
fn separator(lang: Language) -> &'static str {
    match lang {
        Language::Rust => "::",
        _ => "/",
    }
}

/// Does `path` sit under `prefix`, on a module-path boundary?
///
/// The boundary is the whole point: an `allow` of `serde` covers
/// `serde::Deserialize` and must not quietly cover `serde_json`, which is a
/// different dependency that nobody allowed.
///
/// Internal: callers want `classify` and `judge`, which apply this.
fn covers(path: &str, prefix: &str, lang: Language) -> bool {
    if path == prefix {
        return true;
    }
    let sep = separator(lang);
    // A TypeScript specifier may be `node:fs` where the prefix is `node:fs`,
    // or `node:fs/promises` where it is not the same string. Both separators
    // are checked for TypeScript so `node:` behaves like the boundary it is.
    if path.starts_with(&format!("{prefix}{sep}")) {
        return true;
    }
    lang == Language::TypeScript && path.starts_with(&format!("{prefix}:"))
}

/// Node's built-in modules, in their bare form. The `node:` prefix is
/// recognised separately, so only the forms that can be written without it
/// are listed.
const NODE_BUILTINS: &[&str] = &[
    "assert", "async_hooks", "buffer", "child_process", "cluster", "console", "constants",
    "crypto", "dgram", "diagnostics_channel", "dns", "domain", "events", "fs", "http", "http2",
    "https", "inspector", "module", "net", "os", "path", "perf_hooks", "process", "punycode",
    "querystring", "readline", "repl", "stream", "string_decoder", "sys", "timers", "tls",
    "trace_events", "tty", "url", "util", "v8", "vm", "wasi", "worker_threads", "zlib",
];

/// Classify one raw import specifier.
///
/// Per language:
///
/// | | Inside the project | Standard library | External |
/// |---|---|---|---|
/// | Rust | `crate::`, `self::`, `super::`, the package's own name | `std`, `core`, `alloc` | any other first segment |
/// | TypeScript | relative, `tsconfig` path aliases | `node:` specifiers and bare Node built-ins | any other bare specifier |
/// | Go | paths under the module path in `go.mod` | first element contains no dot | everything else |
pub fn classify(lang: Language, raw: &str, names: &ProjectNames) -> Origin {
    let raw = raw.trim();
    match lang {
        Language::Rust => {
            let first = raw.split("::").next().unwrap_or(raw);
            match first {
                "crate" | "self" | "super" => Origin::Internal,
                "std" | "core" | "alloc" => Origin::StandardLibrary,
                other if names.rust_crates.iter().any(|c| c == other) => Origin::Internal,
                _ => Origin::External,
            }
        }
        Language::TypeScript => {
            if raw.starts_with("./") || raw.starts_with("../") || raw.starts_with('/') {
                return Origin::Internal;
            }
            if names.ts_aliases.iter().any(|a| covers(raw, a, lang)) {
                return Origin::Internal;
            }
            if let Some(rest) = raw.strip_prefix("node:") {
                let _ = rest;
                return Origin::StandardLibrary;
            }
            let head = raw.split('/').next().unwrap_or(raw);
            if NODE_BUILTINS.contains(&head) {
                return Origin::StandardLibrary;
            }
            Origin::External
        }
        Language::Go => {
            if let Some(module) = &names.go_module {
                if covers(raw, module, lang) {
                    return Origin::Internal;
                }
            }
            let head = raw.split('/').next().unwrap_or(raw);
            // Go's own rule for telling its standard library from everything
            // else: a first element with a dot in it is a domain name, and a
            // domain name means somebody else's code.
            if head.contains('.') {
                Origin::External
            } else {
                Origin::StandardLibrary
            }
        }
        Language::Unknown => Origin::External,
    }
}

/// Why one import was or was not permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not this policy's business: it resolves inside the project, and layer
    /// edges already govern it.
    OutOfScope,
    Permitted,
    /// Names the reason so the report can say which half of the policy spoke.
    DeniedByDeny,
    NotAllowed,
}

/// Apply one policy to one already-classified import.
///
/// An external import is permitted when the standard library covers it or an
/// `allow` prefix does — **unless** a `deny` prefix covers it. `deny` wins,
/// which is what keeps standard-library I/O out of the domain without
/// requiring every safe module to be listed.
pub fn judge(
    lang: Language,
    raw: &str,
    origin: Origin,
    allow: &[String],
    deny: &[String],
) -> Verdict {
    if origin == Origin::Internal {
        return Verdict::OutOfScope;
    }
    if deny.iter().any(|d| covers(raw, d, lang)) {
        return Verdict::DeniedByDeny;
    }
    if origin == Origin::StandardLibrary {
        return Verdict::Permitted;
    }
    if allow.iter().any(|a| covers(raw, a, lang)) {
        return Verdict::Permitted;
    }
    Verdict::NotAllowed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> ProjectNames {
        ProjectNames {
            rust_crates: vec!["my_app".to_string()],
            go_module: Some("github.com/acme/app".to_string()),
            ts_aliases: vec!["@app".to_string()],
            rust_dependencies: vec!["sqlx".to_string(), "tokio".to_string(), "serde".to_string()],
        }
    }

    // ── which inline paths may be judged at all ───────────────────────────

    #[test]
    fn only_a_declared_dependency_or_the_standard_library_names_something_outside() {
        let n = names();
        for outside in ["std", "core", "alloc", "sqlx", "tokio", "serde"] {
            assert!(names_external(outside, &n), "{outside}");
        }
        // The whole reason this function exists: every one of these has the
        // shape of a crate path and is local code.
        for local in ["crate", "self", "super", "Self", "O", "Ordering", "util", "Colour"] {
            assert!(!names_external(local, &n), "{local} is this project's own");
        }
    }

    #[test]
    fn a_dependency_written_with_dashes_is_the_same_dependency() {
        // Cargo accepts `some-crate` in the manifest; code writes `some_crate`.
        let n = ProjectNames {
            rust_dependencies: vec!["some_crate".to_string()],
            ..Default::default()
        };
        assert!(names_external("some_crate", &n));
        assert!(names_external("some-crate", &n));
        assert!(!names_external("some_other", &n));
    }

    #[test]
    fn an_empty_first_segment_names_nothing() {
        // `::sqlx::query` splits to an empty first segment; the caller strips
        // the prefix before asking, and a bare empty string is not a name.
        assert!(!names_external("", &names()));
    }

    // ── the boundary ──────────────────────────────────────────────────────

    #[test]
    fn a_prefix_covers_only_whole_segments() {
        assert!(covers("serde::Deserialize", "serde", Language::Rust));
        assert!(covers("serde", "serde", Language::Rust));
        assert!(
            !covers("serde_json::Value", "serde", Language::Rust),
            "serde_json is a different dependency than the one that was allowed"
        );
        assert!(covers("std::fs::File", "std::fs", Language::Rust));
        assert!(!covers("std::fstab", "std::fs", Language::Rust));
    }

    #[test]
    fn the_boundary_holds_for_slash_separated_languages() {
        assert!(covers("net/http", "net", Language::Go));
        assert!(!covers("nethttp", "net", Language::Go));
        assert!(covers("node:fs/promises", "node:fs", Language::TypeScript));
        assert!(!covers("pgx", "pg", Language::TypeScript));
    }

    // ── Rust ──────────────────────────────────────────────────────────────

    #[test]
    fn rust_tells_its_own_code_from_everybody_elses() {
        let n = names();
        for internal in ["crate::domain::Order", "self::x", "super::y", "my_app::domain::Order"] {
            assert_eq!(classify(Language::Rust, internal, &n), Origin::Internal, "{internal}");
        }
        for std_lib in ["std::fs::File", "core::mem", "alloc::vec::Vec"] {
            assert_eq!(classify(Language::Rust, std_lib, &n), Origin::StandardLibrary, "{std_lib}");
        }
        for external in ["sqlx::PgPool", "tokio::spawn", "serde::Deserialize"] {
            assert_eq!(classify(Language::Rust, external, &n), Origin::External, "{external}");
        }
    }

    // ── TypeScript ────────────────────────────────────────────────────────

    #[test]
    fn typescript_tells_relative_from_bare() {
        let n = names();
        assert_eq!(classify(Language::TypeScript, "./count.js", &n), Origin::Internal);
        assert_eq!(classify(Language::TypeScript, "../ports/store.js", &n), Origin::Internal);
        assert_eq!(classify(Language::TypeScript, "@app/domain", &n), Origin::Internal);
        assert_eq!(classify(Language::TypeScript, "node:fs", &n), Origin::StandardLibrary);
        assert_eq!(classify(Language::TypeScript, "fs", &n), Origin::StandardLibrary);
        assert_eq!(classify(Language::TypeScript, "pg", &n), Origin::External);
    }

    // ── Go ────────────────────────────────────────────────────────────────

    #[test]
    fn go_tells_the_standard_library_by_the_dot() {
        let n = names();
        assert_eq!(classify(Language::Go, "github.com/acme/app/internal/domain", &n), Origin::Internal);
        assert_eq!(classify(Language::Go, "os", &n), Origin::StandardLibrary);
        assert_eq!(classify(Language::Go, "net/http", &n), Origin::StandardLibrary);
        assert_eq!(classify(Language::Go, "github.com/jackc/pgx/v5", &n), Origin::External);
    }

    // ── the verdict ───────────────────────────────────────────────────────

    #[test]
    fn deny_beats_the_standard_library() {
        // The whole reason `deny` exists: `std` is permitted wholesale, and
        // `std::fs` inside a domain is still an outside capability.
        assert_eq!(
            judge(
                Language::Rust,
                "std::fs::File",
                Origin::StandardLibrary,
                &[],
                &["std::fs".to_string()]
            ),
            Verdict::DeniedByDeny
        );
        assert_eq!(
            judge(Language::Rust, "std::cmp::Ordering", Origin::StandardLibrary, &[], &["std::fs".to_string()]),
            Verdict::Permitted
        );
    }

    #[test]
    fn an_external_import_needs_naming_and_an_internal_one_is_not_ours() {
        let allow = vec!["serde".to_string()];
        assert_eq!(
            judge(Language::Rust, "sqlx::PgPool", Origin::External, &allow, &[]),
            Verdict::NotAllowed
        );
        assert_eq!(
            judge(Language::Rust, "serde::Deserialize", Origin::External, &allow, &[]),
            Verdict::Permitted
        );
        assert_eq!(
            judge(Language::Rust, "serde_json::Value", Origin::External, &allow, &[]),
            Verdict::NotAllowed,
            "the allowlist is a list of dependencies, not of prefixes of names"
        );
        assert_eq!(
            judge(Language::Rust, "crate::domain::Order", Origin::Internal, &[], &[]),
            Verdict::OutOfScope
        );
    }

    #[test]
    fn deny_beats_allow_when_both_name_it() {
        assert_eq!(
            judge(
                Language::Go,
                "net/http",
                Origin::StandardLibrary,
                &["net".to_string()],
                &["net/http".to_string()]
            ),
            Verdict::DeniedByDeny,
            "the more specific statement is the deliberate one"
        );
    }
}
