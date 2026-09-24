//! Path Normalizer — pure functions for resolving import paths
//! across TypeScript, Go, and Rust.
//!
//! Each language has different import semantics:
//! - TypeScript: relative paths with .js extensions → resolved to .ts
//! - Go: module paths like "github.com/user/pkg" → kept as-is for external,
//!   relative paths within project resolved normally
//! - Rust: crate paths like "crate::core::ports" → converted to file paths
//!
//! Ported from `src/core/usecases/path-normalizer.ts`.

use super::domain::Language;

// ── Pure Path Helpers ────────────────────────────────────
//
// These replace node:path/posix to keep this module free of std::path
// (which uses OS-native separators). All paths here use forward slashes.

/// Return the directory portion of a forward-slash path.
fn dirname_posix(p: &str) -> &str {
    match p.rfind('/') {
        None => ".",
        Some(0) => "/",
        Some(idx) => &p[..idx],
    }
}

/// Join path segments with '/' and normalise (collapse '..' and '.', remove double slashes).
fn join_posix(parts: &[&str]) -> String {
    let joined = parts.join("/");
    let mut segments: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." && !segments.is_empty() && segments.last() != Some(&"..") {
            segments.pop();
        } else {
            segments.push(seg);
        }
    }
    if segments.is_empty() {
        ".".to_string()
    } else {
        segments.join("/")
    }
}

// ── Public API ───────────────────────────────────────────

/// Resolve an import path to a project-relative file path.
///
/// # Examples
/// - TypeScript: `"./foo.js"` from `"src/bar.ts"` → `"src/foo.ts"`
/// - Go: `"../ports"` from `"src/adapters/primary/cli.go"` → `"src/ports"`
/// - Rust: `"crate::core::ports"` → `"src/core/ports"`
pub fn resolve_import_path(
    from_file: &str,
    import_path: &str,
    go_module_prefix: Option<&str>,
) -> String {
    let lang = Language::from_path(from_file);
    match lang {
        Language::Go => resolve_go_import(from_file, import_path, go_module_prefix),
        Language::Rust => resolve_rust_import(import_path, from_file),
        _ => resolve_ts_import(from_file, import_path),
    }
}

/// Normalize a file path for comparison: strip leading `./`, fix extensions.
///
/// Infers the language from the path itself. That is right for a real file and
/// wrong for a *resolved import target*, which in Go and Rust is a package or
/// module directory with no extension to infer from — use
/// [`normalize_path_in`] there and pass the importing file's language.
pub fn normalize_path(file_path: &str) -> String {
    normalize_path_in(file_path, Language::from_path(file_path))
}

/// [`normalize_path`], with the language stated rather than guessed.
///
/// This exists because guessing was wrong in a way that produced confident,
/// false violations. A Go import of `myapp/internal/ports` resolves to the
/// directory `internal/ports`. With no extension, the guess fell through to
/// the TypeScript branch and appended `.ts`, giving `internal/ports.ts` — a
/// file that does not exist. The layer classifier then failed to match
/// `/internal/ports/` against it, matched the `/internal/` catch-all instead,
/// and reported "adapters must not import from usecases" against a correctly
/// layered Go project.
pub fn normalize_path_in(file_path: &str, lang: Language) -> String {
    let mut p = file_path.to_string();

    // Strip leading ./
    while p.starts_with("./") {
        p = p[2..].to_string();
    }

    match lang {
        // Go and Rust keep the path as written. A package or module path is a
        // directory, and inventing a filename for it is how the bug above
        // happened.
        Language::Go | Language::Rust => p,
        _ => {
            // TypeScript: Replace .js/.jsx extension with .ts/.tsx
            if p.ends_with(".js") {
                p.truncate(p.len() - 3);
                p.push_str(".ts");
            } else if p.ends_with(".jsx") {
                p.truncate(p.len() - 4);
                p.push_str(".tsx");
            } else if p.ends_with('/') {
                p.push_str("index.ts");
            } else if !p.ends_with(".ts")
                && !p.ends_with(".tsx")
                && !p.contains(':')
                && !p.ends_with(".go")
                && !p.ends_with(".rs")
            {
                p.push_str(".ts");
            }
            p
        }
    }
}

// ── Language-Specific Resolvers ──────────────────────────

fn resolve_ts_import(from_file: &str, import_path: &str) -> String {
    if !import_path.starts_with('.') {
        return normalize_path(import_path);
    }
    let dir = dirname_posix(from_file);
    let resolved = join_posix(&[dir, import_path]);
    normalize_path(&resolved)
}

fn resolve_go_import(from_file: &str, import_path: &str, module_prefix: Option<&str>) -> String {
    if import_path.starts_with('.') {
        let dir = dirname_posix(from_file);
        return join_posix(&[dir, import_path]);
    }
    // Strip Go module prefix to get project-relative path for layer classification
    if let Some(prefix) = module_prefix {
        if let Some(rest) = import_path.strip_prefix(prefix).and_then(|s| s.strip_prefix('/')) {
            return rest.to_string();
        }
    }
    // External or stdlib import — return as-is
    import_path.to_string()
}

fn resolve_rust_import(import_path: &str, from_file: &str) -> String {
    // crate:: paths map to the importing crate's src/ directory. In a
    // workspace the importing file is `hexa-core/src/domain/x.rs`, so its
    // crate root is `hexa-core/`; without that prefix a `crate::` target
    // named a path in no crate and no Rust cycle inside a workspace member
    // could ever close.
    if let Some(rest) = import_path.strip_prefix("crate::") {
        let segments: Vec<&str> = rest.split("::").collect();
        let stripped = strip_rust_item_name(&segments);
        let crate_root = match from_file.find("src/") {
            Some(i) if i == 0 || from_file.as_bytes()[i - 1] == b'/' => &from_file[..i],
            _ => "",
        };
        return format!("{}src/{}", crate_root, stripped.join("/"));
    }

    // self::foo — current module (resolve relative to importing file's directory)
    if let Some(rest) = import_path.strip_prefix("self::") {
        let dir = dirname_posix(from_file);
        let segments: Vec<&str> = rest.split("::").collect();
        let mut parts = vec![dir];
        parts.extend(segments);
        return join_posix(&parts);
    }

    // super::foo — parent module
    if import_path.starts_with("super::") {
        return import_path.replace("::", "/");
    }

    // External crate or std — return as-is
    import_path.to_string()
}

// ── Workspace Packages ───────────────────────────────────

/// The project's own packages — Cargo crates, Go modules, npm packages —
/// and where each lives, so an import of one resolves to a path in the tree
/// instead of being taken for a third-party dependency.
///
/// Without this, `hexa_core::ports::x`, `example.com/store` and `@fx/store`
/// were all "external": no edge, no layer, no check. A workspace that gives
/// each layer its own package — the layout that should grade best, because
/// the build tool then refuses undeclared imports — had every crossing
/// between its layers invisible.
#[derive(Debug, Clone, Default)]
pub struct WorkspacePackages {
    /// (language, import name, project-relative dir, TS entry file).
    /// Longest name first, so `example.com/a/b` wins over `example.com/a`.
    entries: Vec<(Language, String, String, Option<String>)>,
}

impl WorkspacePackages {
    /// A Cargo package: imported as its name with `-` read as `_`.
    pub fn add_crate(&mut self, name: &str, dir: &str) {
        self.push(Language::Rust, name.replace('-', "_"), dir, None);
    }

    /// A Go module, by its `module` path.
    pub fn add_go_module(&mut self, module: &str, dir: &str) {
        self.push(Language::Go, module.to_string(), dir, None);
    }

    /// An npm package, by its `name`, with the file a bare import of it
    /// reaches.
    pub fn add_npm_package(&mut self, name: &str, dir: &str, entry: &str) {
        self.push(Language::TypeScript, name.to_string(), dir, Some(entry.to_string()));
    }

    fn push(&mut self, lang: Language, name: String, dir: &str, entry: Option<String>) {
        let dir = dir.trim_matches('/').to_string();
        self.entries.push((lang, name, dir, entry));
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.1.len()));
    }

    /// The project-relative path `import_path` names, when it names one of
    /// these packages; `None` for anything else, which keeps its existing
    /// resolution.
    pub fn resolve(&self, from_file: &str, import_path: &str) -> Option<String> {
        let lang = Language::from_path(from_file);
        let join = |dir: &str, rest: &str| {
            if dir.is_empty() { rest.to_string() } else { format!("{dir}/{rest}") }
        };
        for (l, name, dir, entry) in &self.entries {
            if *l != lang {
                continue;
            }
            match lang {
                Language::Rust => {
                    let mut segs = import_path.split("::");
                    if segs.next() != Some(name.as_str()) {
                        continue;
                    }
                    let rest: Vec<&str> = segs.collect();
                    if rest.is_empty() {
                        return Some(join(dir, "src/lib.rs"));
                    }
                    // `name::` stands where `crate::` would, so the item name
                    // is stripped as it is for a `crate::` path.
                    let mut with_root = vec!["crate"];
                    with_root.extend(&rest);
                    let stripped = strip_rust_item_name(&with_root);
                    return Some(join(dir, &format!("src/{}", stripped[1..].join("/"))));
                }
                Language::Go | Language::TypeScript => {
                    let rest = if import_path == name {
                        ""
                    } else if let Some(r) = import_path.strip_prefix(name.as_str()).and_then(|r| r.strip_prefix('/')) {
                        r
                    } else {
                        continue;
                    };
                    return Some(match (rest, entry) {
                        ("", Some(e)) => join(dir, e),
                        ("", None) => if dir.is_empty() { ".".to_string() } else { dir.clone() },
                        (r, _) => join(dir, r),
                    });
                }
                Language::Unknown => {}
            }
        }
        None
    }
}

/// Strip trailing item-name segment from a Rust path.
///
/// If the path has 3+ segments and the last segment starts with an uppercase
/// letter, it's an item name (type/function), not a file/module.
fn strip_rust_item_name<'a>(segments: &'a [&'a str]) -> Vec<&'a str> {
    if segments.len() >= 3 {
        if let Some(last) = segments.last() {
            if last.starts_with(|c: char| c.is_ascii_uppercase()) {
                return segments[..segments.len() - 1].to_vec();
            }
        }
    }
    segments.to_vec()
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // TypeScript resolution
    #[test]
    fn ts_relative_import() {
        assert_eq!(
            resolve_import_path("src/bar.ts", "./foo.js", None),
            "src/foo.ts"
        );
    }

    #[test]
    fn ts_parent_import() {
        assert_eq!(
            resolve_import_path("src/adapters/primary/cli.ts", "../secondary/db.js", None),
            "src/adapters/secondary/db.ts"
        );
    }

    #[test]
    fn ts_absolute_package() {
        assert_eq!(
            resolve_import_path("src/foo.ts", "lodash", None),
            "lodash.ts"
        );
    }

    // Go resolution
    #[test]
    fn go_relative_import() {
        assert_eq!(
            resolve_import_path("internal/adapters/handler.go", "../ports", None),
            "internal/ports"
        );
    }

    #[test]
    fn go_module_prefix_strip() {
        assert_eq!(
            resolve_import_path(
                "cmd/main.go",
                "github.com/org/repo/internal/domain",
                Some("github.com/org/repo")
            ),
            "internal/domain"
        );
    }

    #[test]
    fn go_stdlib_passthrough() {
        assert_eq!(
            resolve_import_path("cmd/main.go", "net/http", None),
            "net/http"
        );
    }

    // Rust resolution
    #[test]
    fn rust_crate_path() {
        assert_eq!(
            resolve_import_path("src/adapters/primary/cli.rs", "crate::core::ports", None),
            "src/core/ports"
        );
    }

    #[test]
    fn rust_crate_path_strips_item_name() {
        assert_eq!(
            resolve_import_path("src/adapters/primary/cli.rs", "crate::core::ports::IFoo", None),
            "src/core/ports"
        );
    }

    #[test]
    fn rust_self_path() {
        assert_eq!(
            resolve_import_path("src/adapters/primary/cli.rs", "self::helpers", None),
            "src/adapters/primary/helpers"
        );
    }

    #[test]
    fn rust_external_crate() {
        assert_eq!(
            resolve_import_path("src/main.rs", "tokio::runtime", None),
            "tokio::runtime"
        );
    }

    // Normalize
    #[test]
    fn normalize_strips_leading_dot_slash() {
        assert_eq!(normalize_path("./src/foo.ts"), "src/foo.ts");
    }

    #[test]
    fn normalize_js_to_ts() {
        assert_eq!(normalize_path("src/foo.js"), "src/foo.ts");
    }

    #[test]
    fn normalize_go_unchanged() {
        assert_eq!(normalize_path("internal/domain.go"), "internal/domain.go");
    }

    #[test]
    fn normalize_rs_unchanged() {
        assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
    }

    // Rust module candidates

    /// A Go package import resolves to a directory. Appending `.ts` to it
    /// produced `internal/ports.ts`, which the layer classifier then matched
    /// against the `/internal/` catch-all instead of `/internal/ports/` — and
    /// reported "adapters must not import from usecases" against a correctly
    /// layered Go project. Found by requiring hexa's own Go scaffold to grade
    /// clean.
    #[test]
    fn a_go_package_path_never_gains_a_typescript_extension() {
        assert_eq!(normalize_path_in("internal/ports", Language::Go), "internal/ports");
        assert_eq!(normalize_path_in("internal/domain", Language::Go), "internal/domain");
        assert_eq!(
            normalize_path_in("adapters/secondary", Language::Go),
            "adapters/secondary"
        );
    }

    /// Same trap for Rust: `crate::ports` resolves to `src/ports`.
    #[test]
    fn a_rust_module_path_never_gains_a_typescript_extension() {
        assert_eq!(normalize_path_in("src/ports", Language::Rust), "src/ports");
    }

    /// And the TypeScript behaviour this all hangs off is unchanged: an
    /// extensionless TS import really does mean a `.ts` file.
    #[test]
    fn a_typescript_import_still_gains_its_extension() {
        assert_eq!(normalize_path_in("src/core/ports", Language::TypeScript), "src/core/ports.ts");
        assert_eq!(normalize_path("src/foo.js"), "src/foo.ts");
    }

}
