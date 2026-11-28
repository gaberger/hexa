//! Path Normalizer — pure functions for resolving import paths
//! across TypeScript, Go, and Rust.
//!
//! Each language has different import semantics:
//! - TypeScript: relative paths with .js extensions → resolved to .ts
//! - Go: module paths like "github.com/user/pkg" → kept as-is for external,
//!   relative paths within project resolved normally
//! - Rust: crate paths like "crate::core::ports" → converted to file paths
//! - Python: dotted modules, absolute or relative → the `.py` or package
//!   `__init__.py` they name, among the project's files (ADR-2611211324)
//!
//! Ported from `src/core/usecases/path-normalizer.ts`.

use std::collections::HashSet;

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

/// The project file a Python import names (ADR-2611211324 §2), or `None`
/// when it names none (the standard library, a third-party package, or a
/// module that does not exist): such an import is not an edge.
///
/// `module` is as written: "a.b", ".ports", "..". `name` is one name a
/// `from` import takes. `from a import b` is the submodule `a/b.py` when
/// there is one, else the package `a/__init__.py`; `import a.b` is `a.b`
/// only. A relative import starts from the importing file's package; an
/// absolute one from the project root, then `src/`. `files` are the
/// project's source files, relative to its root.
pub fn resolve_python_import(from_file: &str, module: &str, name: Option<&str>, files: &HashSet<String>) -> Option<String> {
    let dots = module.chars().take_while(|c| *c == '.').count();
    let parts: Vec<&str> = module[dots..].split('.').filter(|s| !s.is_empty()).collect();
    let bases: Vec<Vec<&str>> = if dots > 0 {
        let mut dir: Vec<&str> = from_file.split('/').collect();
        dir.pop();
        for _ in 1..dots {
            dir.pop()?;
        }
        vec![dir]
    } else if parts.is_empty() {
        return None;
    } else {
        vec![Vec::new(), vec!["src"]]
    };
    let file_of = |segs: &[&str]| -> Option<String> {
        if segs.is_empty() {
            return None;
        }
        let stem = segs.join("/");
        // A stub is the module only where there is no `.py`: a compiled
        // extension, or a package that ships types (ADR-2611251938 §1).
        [format!("{stem}.py"), format!("{stem}/__init__.py"), format!("{stem}.pyi"), format!("{stem}/__init__.pyi")]
            .into_iter()
            .find(|c| files.contains(c))
    };
    for base in bases {
        let mut module_path = base.clone();
        module_path.extend(&parts);
        if let Some(n) = name.filter(|n| *n != "*") {
            let mut sub = module_path.clone();
            sub.push(n);
            if let Some(f) = file_of(&sub) {
                return Some(f);
            }
        }
        if let Some(f) = file_of(&module_path) {
            return Some(f);
        }
    }
    None
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
        Language::Go | Language::Rust | Language::Python => p,
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

    // self:: and super:: are relative to the importing file's *module*: a
    // `mod.rs`, `lib.rs` or `main.rs` is the module of its directory, and
    // `foo.rs` is the module `foo`. `super::` used to resolve to the literal
    // path `super/…`, which is in no layer — so an import through it was
    // never checked, and matched only when its text happened to contain a
    // layer's folder name. `self::x` from `foo.rs` named a sibling of `foo`
    // rather than its child.
    if import_path.starts_with("self::") || import_path.starts_with("super::") {
        let module_path = rust_module_of(from_file);
        let mut module: Vec<&str> = module_path.split('/').filter(|s| !s.is_empty()).collect();
        let mut segments = import_path.split("::").peekable();
        while let Some(&head) = segments.peek() {
            match head {
                "self" => {}
                "super" => {
                    if module.pop().is_none() {
                        return import_path.to_string();
                    }
                }
                _ => break,
            }
            segments.next();
        }
        let rest: Vec<&str> = segments.collect();
        // The item name is stripped as it is for a `crate::` path.
        let mut with_root = vec!["crate"];
        with_root.extend(&rest);
        let stripped = strip_rust_item_name(&with_root);
        module.extend(stripped[1..].iter().copied());
        return module.join("/");
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
    /// The project's `tsconfig.json` aliases (ADR-2611221645).
    ts_paths: TsPaths,
    /// The project's TypeScript files, for the file an import names.
    ts_files: HashSet<String>,
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

    /// The project's `tsconfig.json` aliases.
    pub fn set_ts_paths(&mut self, paths: TsPaths) {
        self.ts_paths = paths;
    }

    /// A TypeScript or JavaScript file of the project, which an import can name.
    pub fn add_ts_file(&mut self, rel: &str) {
        self.ts_files.insert(rel.to_string());
    }

    /// Whether `rel` is a TypeScript or JavaScript file of the project, by the
    /// name it has: an import that resolved to one keeps that name.
    pub fn knows_ts_file(&self, rel: &str) -> bool {
        self.ts_files.contains(rel)
    }

    /// The project file a TypeScript import names: a relative import by the
    /// extensions and index files it tries, a bare one through the project's
    /// aliases (ADR-2611221645 §1). `None` when it names no project file.
    fn resolve_ts(&self, from_file: &str, import_path: &str) -> Option<String> {
        let exists = |c: &str| self.ts_files.contains(c);
        if import_path.starts_with('.') {
            let joined = join_posix(&[dirname_posix(from_file), import_path]);
            return ts_file_named(&joined, &exists);
        }
        self.ts_paths.resolve(import_path, &exists)
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
        if lang == Language::TypeScript {
            if let Some(f) = self.resolve_ts(from_file, import_path) {
                return Some(f);
            }
        }
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
                Language::Python | Language::Unknown => {}
            }
        }
        None
    }
}

/// The module path of a Rust file: its directory for `mod.rs`, `lib.rs` and
/// `main.rs`, else the file without `.rs`.
fn rust_module_of(from_file: &str) -> String {
    let name = from_file.rsplit('/').next().unwrap_or(from_file);
    if matches!(name, "mod.rs" | "lib.rs" | "main.rs") {
        let dir = dirname_posix(from_file);
        return if dir == "." { String::new() } else { dir.to_string() };
    }
    from_file.trim_end_matches(".rs").to_string()
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

// ── TypeScript path aliases (ADR-2611221645) ─────────────

/// The `compilerOptions.paths` of a project's root `tsconfig.json`, with
/// `extends` followed, each target already made project-relative (against
/// `baseUrl` when one is set, else the directory of the `tsconfig.json`
/// that declares `paths`).
///
/// Without it, `@/lib/http` stayed the literal `@/lib/http.ts`: no project
/// file, no layer, never a violation — and a project that imports through
/// an alias was graded on part of its import graph.
#[derive(Debug, Clone, Default)]
pub struct TsPaths {
    /// (pattern as written, project-relative targets in order).
    patterns: Vec<(String, Vec<String>)>,
}

impl TsPaths {
    /// Load the alias map from `tsconfig.json` at the project root. `read`
    /// returns the text of a project-relative file, or `None`. A relative
    /// `extends` is followed (ADR-2611221645 §2); one that names a package
    /// is not.
    pub fn load(read: &dyn Fn(&str) -> Option<String>) -> Self {
        let (mut chain, mut budget) = (Vec::new(), TSCONFIG_READ_BUDGET);
        let (mut paths, mut base) = tsconfig_settings("tsconfig.json", read, &mut chain, &mut budget);
        // A project with no tsconfig.json paths may declare them in a
        // jsconfig.json, read the same way (ADR-2611270420 §3).
        if paths.is_none() {
            (paths, base) = tsconfig_settings("jsconfig.json", read, &mut chain, &mut budget);
        }
        let Some((map, decl_dir)) = paths else { return Self::default() };
        let base_dir = base.unwrap_or(decl_dir);
        let patterns = map
            .into_iter()
            .map(|(pattern, targets)| {
                let targets = targets.iter().filter(|t| !t.starts_with('/')).map(|t| join_posix(&[&base_dir, t])).collect();
                (pattern, targets)
            })
            .collect();
        Self { patterns }
    }

    /// True when the project declares no alias.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// The project file `spec` names through an alias: the pattern that
    /// matches it (an exact pattern first, else the longest prefix), each
    /// target tried in order, the first that names a file by `exists`.
    /// `None` when no pattern matches or no target names a file
    /// (ADR-2611221645 §1, §3).
    pub fn resolve(&self, spec: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
        let mut best: Option<(&Vec<String>, &str, usize)> = None;
        for (pattern, targets) in &self.patterns {
            match pattern.split_once('*') {
                None => {
                    if pattern == spec {
                        best = Some((targets, "", usize::MAX));
                        break;
                    }
                }
                Some((pre, post)) => {
                    if post.contains('*') || spec.len() < pre.len() + post.len() {
                        continue;
                    }
                    if spec.starts_with(pre) && spec.ends_with(post) && best.is_none_or(|b| pre.len() > b.2) {
                        best = Some((targets, &spec[pre.len()..spec.len() - post.len()], pre.len()));
                    }
                }
            }
        }
        let (targets, capture, _) = best?;
        targets.iter().find_map(|t| ts_file_named(&t.replacen('*', capture, 1), exists))
    }
}

/// The extensions of JavaScript source (ADR-2611270420 §1).
const JS_EXTENSIONS: &[&str] = &["js", "jsx", "mjs", "cjs"];

/// Whether `path` is a JavaScript file by its extension.
pub fn is_js_file(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| JS_EXTENSIONS.contains(&ext))
}

/// The name a file goes by in the graph. A TypeScript file is named as it has
/// always been (`.js` read as `.ts`: a NodeNext import says `./a.js` for
/// `a.ts`); a JavaScript file is a file of its own and keeps the name it has
/// (ADR-2611270420 §1).
pub fn file_id(path: &str) -> String {
    if is_js_file(path) {
        let mut p = path;
        while let Some(rest) = p.strip_prefix("./") {
            p = rest;
        }
        p.to_string()
    } else {
        normalize_path(path)
    }
}

/// The file a TypeScript or JavaScript module path names, by the extensions and
/// index files an import tries: as written, `.js`/`.jsx` read as `.ts`/`.tsx`
/// when that file exists, then `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`
/// added to it, then the same under `/index` (ADR-2611221645 §1,
/// ADR-2611270420 §3). A TypeScript file wins a tie, as the compiler's mapping
/// of `./a.js` to `a.ts` does.
fn ts_file_named(path: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(stem) = path.strip_suffix(".js") {
        candidates.extend([format!("{stem}.ts"), format!("{stem}.tsx")]);
    } else if let Some(stem) = path.strip_suffix(".jsx") {
        candidates.push(format!("{stem}.tsx"));
    }
    candidates.push(path.to_string());
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        candidates.push(format!("{path}.{ext}"));
    }
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        candidates.push(format!("{path}/index.{ext}"));
    }
    candidates.into_iter().find(|c| exists(c))
}

type PathsMap = Vec<(String, Vec<String>)>;

/// How many tsconfig files one load may read: a bound on a diamond-heavy
/// `extends` graph, which the per-chain cycle check alone does not bound.
const TSCONFIG_READ_BUDGET: usize = 256;

/// The `paths` (with the directory of the file that declares them) and the
/// project-relative `baseUrl` that `rel` settles on after its `extends`.
/// `chain` is the `extends` path from the root to `rel`: only a file already
/// on it is a cycle, so a base shared by two branches is read for both.
fn tsconfig_settings(
    rel: &str,
    read: &dyn Fn(&str) -> Option<String>,
    chain: &mut Vec<String>,
    budget: &mut usize,
) -> (Option<(PathsMap, String)>, Option<String>) {
    if chain.iter().any(|s| s == rel) || chain.len() > 32 || *budget == 0 {
        return (None, None);
    }
    *budget -= 1;
    chain.push(rel.to_string());
    let out = tsconfig_settings_of(rel, read, chain, budget);
    chain.pop();
    out
}

fn tsconfig_settings_of(
    rel: &str,
    read: &dyn Fn(&str) -> Option<String>,
    chain: &mut Vec<String>,
    budget: &mut usize,
) -> (Option<(PathsMap, String)>, Option<String>) {
    let Some(text) = read(rel) else { return (None, None) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)) else { return (None, None) };
    let dir = dirname_posix(rel).to_string();
    let (mut paths, mut base) = (None, None);
    let extends: Vec<&str> = match v.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.as_str()],
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect(),
        _ => Vec::new(),
    };
    for ext in extends {
        if !(ext.starts_with("./") || ext.starts_with("../")) {
            continue;
        }
        let target = join_posix(&[&dir, ext]);
        // Only project files are read: a tsconfig above the root is not
        // followed (ADR-2611221645 §4, as the graph does).
        if target == ".." || target.starts_with("../") {
            continue;
        }
        let target = if read(&target).is_none() && !target.ends_with(".json") { format!("{target}.json") } else { target };
        let (p, b) = tsconfig_settings(&target, read, chain, budget);
        if p.is_some() {
            paths = p;
        }
        if b.is_some() {
            base = b;
        }
    }
    let opts = v.get("compilerOptions");
    if let Some(b) = opts.and_then(|o| o.get("baseUrl")).and_then(|b| b.as_str()) {
        base = Some(join_posix(&[&dir, b]));
    }
    if let Some(map) = opts.and_then(|o| o.get("paths")).and_then(|p| p.as_object()) {
        let map = map
            .iter()
            .map(|(k, t)| {
                let targets = t.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                (k.clone(), targets)
            })
            .collect();
        paths = Some((map, dir));
    }
    (paths, base)
}

/// JSON with comments and trailing commas (as `tsconfig.json` allows) made
/// plain JSON: `//` and `/* */` comments outside strings removed, and a comma
/// before a closing `}` or `]` dropped.
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '\\' && i + 1 < chars.len() {
                    out.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                i += 1;
                if chars[i - 1] == '"' {
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            // Skip whitespace and comments to see what the comma precedes.
            loop {
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if chars.get(j) == Some(&'/') && chars.get(j + 1) == Some(&'/') {
                    while j < chars.len() && chars[j] != '\n' {
                        j += 1;
                    }
                } else if chars.get(j) == Some(&'/') && chars.get(j + 1) == Some(&'*') {
                    j += 2;
                    while j < chars.len() && !(chars[j] == '*' && chars.get(j + 1) == Some(&'/')) {
                        j += 1;
                    }
                    j += 2;
                } else {
                    break;
                }
            }
            if matches!(chars.get(j), Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
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
        // In `cli.rs`, `self` is the module `cli`: `self::helpers` is
        // `cli/helpers.rs`. This asserted the sibling `primary/helpers`,
        // pinning the resolver's old mistake as the expected answer.
        assert_eq!(
            resolve_import_path("src/adapters/primary/cli.rs", "self::helpers", None),
            "src/adapters/primary/cli/helpers"
        );
        // In `mod.rs`, `self` is the directory's module.
        assert_eq!(
            resolve_import_path("src/adapters/primary/mod.rs", "self::cli", None),
            "src/adapters/primary/cli"
        );
    }

    #[test]
    fn rust_super_path_is_the_parent_module() {
        assert_eq!(
            resolve_import_path("src/domain/order.rs", "super::money::Money", None),
            "src/domain/money"
        );
        assert_eq!(
            resolve_import_path("src/domain/order.rs", "super::super::store::Disk", None),
            "src/store"
        );
        assert_eq!(
            resolve_import_path("src/domain/mod.rs", "super::ports::StorePort", None),
            "src/ports"
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

    // TypeScript path aliases (ADR-2611221645)

    fn ts_paths(files: &[(&str, &str)]) -> TsPaths {
        let files: Vec<(String, String)> = files.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        TsPaths::load(&|rel: &str| files.iter().find(|(p, _)| p == rel).map(|(_, t)| t.clone()))
    }

    #[test]
    fn a_tsconfig_with_comments_and_trailing_commas_is_read() {
        let p = ts_paths(&[(
            "tsconfig.json",
            "{\n // the alias\n \"compilerOptions\": { /* \"x\" */ \"paths\": { \"@/*\": [\"./src/*\",], }, },\n \"url\": \"http://a//b\",\n}",
        )]);
        let files: HashSet<String> = ["src/lib/http.ts".to_string()].into();
        assert_eq!(p.resolve("@/lib/http", &|c| files.contains(c)).as_deref(), Some("src/lib/http.ts"));
    }

    #[test]
    fn a_scoped_package_that_resembles_an_alias_is_not_rewritten() {
        let p = ts_paths(&[("tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#)]);
        let files: HashSet<String> = ["src/types/node.ts".to_string(), "srctypes/node.ts".to_string()].into();
        assert_eq!(p.resolve("@types/node", &|c| files.contains(c)), None);
        assert_eq!(p.resolve("react", &|c| files.contains(c)), None);
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        let p = ts_paths(&[(
            "tsconfig.json",
            r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"], "@/lib/*": ["./vendor/*"]}}}"#,
        )]);
        let files: HashSet<String> = ["src/lib/http.ts".to_string(), "vendor/http.ts".to_string()].into();
        assert_eq!(p.resolve("@/lib/http", &|c| files.contains(c)).as_deref(), Some("vendor/http.ts"));
    }
}
