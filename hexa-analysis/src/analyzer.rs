//! Architecture Analyzer — orchestrates all analysis checks.
//!
//! Composes the boundary checker, cycle detector, dead export finder,
//! and tree-sitter adapter to produce a complete `ArchAnalysisResult`.
//!
//! ADR-034 Phase 3.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use super::api_contract;
use super::boundary_checker;
use super::cycle_detector;
use super::dead_export_finder::{self, FileData};
use super::domain::{
    ArchAnalysisResult, Coverage, DeadExport, DependencyViolation, HexLayer, ImportEdge, ImportStatement,
    Language,
};
use super::frontend_checker;
use super::layer_classifier::{adapters_from_project, LayerMap};
use super::path_normalizer::{file_id, is_js_file, normalize_path, normalize_path_in, resolve_import_path, resolve_python_import, TsPaths, WorkspacePackages};
use super::ports::{AnalysisError, AstPort, ArchAnalysisPort};

/// Source file glob patterns for supported languages.
const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "rs", "py", "pyi"];

/// The manifests that make a directory a Python project, in the order one is
/// named (ADR-2611211324 §4). Python is graded where one is present, or where
/// `analyze.layers` declares a layer for it (ADR-2611251938 §1).
pub const PYTHON_MANIFESTS: [&str; 4] = ["pyproject.toml", "setup.py", "manage.py", "requirements.txt"];

/// What the grade leaves out. A pattern starting with `/` is a directory,
/// matched as a whole path segment; any other is matched within the path
/// (file-name suffixes). The directories were substrings, so `dist` left out
/// `src/domain/distance.ts` and `tests/` left out `contests/` — and `test/`,
/// where TypeScript keeps its tests, was not excluded at all, which under the
/// coverage ceiling cost grade for test helpers (ADR-2609241707).
const EXCLUDE_PATTERNS: &[&str] = &[
    "/node_modules/",
    "/dist/",
    "/examples/",
    // Tool configuration is consumed by the tool, not imported by code. Its
    // default export read as dead on every project that has one.
    ".config.ts",
    ".config.js",
    ".config.mjs",
    ".config.cjs",
    ".test.ts",
    ".spec.ts",
    // A React test beside its component is a test as a `.ts` one is
    // (ADR-2611221706 §1).
    ".test.tsx",
    ".spec.tsx",
    // JavaScript's tests are named as TypeScript's are (ADR-2611270420 §5).
    ".test.js",
    ".spec.js",
    ".test.jsx",
    ".spec.jsx",
    ".test.mjs",
    ".spec.mjs",
    ".test.cjs",
    ".spec.cjs",
    "_test.go",
    ".test.rs",
    "/tests/",
    "/test/",
    "/__tests__/",
    "/target/",
    // Python environments and caches are not the project's code, and
    // `*_test.py` is test code (ADR-2611211324 §4). Generated migrations are
    // [`PYTHON_ONLY_EXCLUDES`]: a TypeORM or goose migration is written code.
    "/.venv/",
    "/venv/",
    "/site-packages/",
    "/__pycache__/",
    "_test.py",
    // Claude Code agent worktrees: full copies of the tree, gitignored, never
    // the project's own sources.
    "/.claude/worktrees/",
];

/// What the grade leaves out of a `.py` or `.pyi` file only: Django's and
/// Alembic's generated migrations.
const PYTHON_ONLY_EXCLUDES: &[&str] = &["/migrations/"];

/// A file the grade leaves out by the built-in patterns.
fn excluded_by_default(file_path: &str) -> bool {
    matches_exclude(file_path, EXCLUDE_PATTERNS)
        || ((file_path.ends_with(".py") || file_path.ends_with(".pyi")) && matches_exclude(file_path, PYTHON_ONLY_EXCLUDES))
}

fn matches_exclude(file_path: &str, patterns: &[&str]) -> bool {
    let segmented = format!("/{}/", file_path.trim_matches('/'));
    patterns.iter().any(|p| {
        if p.starts_with('/') {
            return segmented.contains(p);
        }
        match p.strip_prefix('*') {
            Some(suffix) => file_path.ends_with(suffix),
            None => file_path.contains(p),
        }
    })
}

/// Project-declared exclusions from `.hexa/project.json`:
///
/// ```json
/// { "analyze": { "exclude": ["hexa-cli/assets/scaffold"] } }
/// ```
///
/// This exists so a project can keep embedded template data, vendored code or
/// generated output out of its own grade without that project's directory
/// names being written into the analyzer. The analyzer used to carry
/// `hexa-core/`, `hexa-cli/` and three deleted crate names in its exclusion
/// list, which meant hexa graded itself over six of eight crates and a
/// violation planted in hexa-core was invisible. Names belong in the
/// project's config, not in the tool.
fn project_excludes(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(".hexa").join("project.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    v.get("analyze")
        .and_then(|a| a.get("exclude"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn is_source_file(path: &str) -> bool {
    SOURCE_EXTENSIONS.iter().any(|ext| {
        path.ends_with(&format!(".{}", ext))
    })
}

/// The import edges of one file: each import resolved (the project's own
/// packages first) and both ends given a layer. The one place an edge is
/// built, so the whole-project grade and the single-file check cannot
/// disagree about an import.
fn edges_of(
    rel_path: &str,
    imports: &[ImportStatement],
    packages: &WorkspacePackages,
    layers: &LayerMap,
    go_module_prefix: Option<&str>,
    py_files: &HashSet<String>,
) -> Vec<ImportEdge> {
    let from_file = file_id(rel_path);
    let edge = |to_file: String, imp: &ImportStatement| ImportEdge {
        from_file: from_file.clone(),
        from_layer: layers.classify(&from_file),
        to_layer: layers.classify(&to_file),
        to_file,
        import_path: imp.raw_path.clone(),
        line: imp.line,
    };
    if Language::from_path(rel_path) == Language::Python {
        // An import that names no project file is not an edge (ADR-2611211324 §2).
        return imports.iter().flat_map(|imp| python_targets(rel_path, imp, py_files).into_iter().map(move |t| (t, imp))).map(|(t, imp)| edge(t, imp)).collect();
    }
    imports
        .iter()
        .map(|imp| {
            let resolved = resolve_in(packages, rel_path, &imp.raw_path, go_module_prefix);
            // A project file keeps the name it has: a `.js` import of a `.js` file
            // is not that file's `.ts` namesake (ADR-2611270420 §3). The importing
            // file decides the language, not the resolved target.
            if packages.knows_ts_file(&resolved) {
                return edge(resolved, imp);
            }
            edge(normalize_path_in(&resolved, Language::from_path(rel_path)), imp)
        })
        .collect()
}

/// The project files one Python import reaches: one per name a `from` import
/// takes, or the module itself (ADR-2611211324 §2).
fn python_targets(rel_path: &str, imp: &ImportStatement, py_files: &HashSet<String>) -> Vec<String> {
    let names: Vec<Option<&str>> = if imp.names.is_empty() { vec![None] } else { imp.names.iter().map(|n| Some(n.as_str())).collect() };
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if let Some(t) = resolve_python_import(rel_path, &imp.raw_path, n, py_files) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// Where an import lands, for the dead-export finder: for Python the file it
/// names (its first, when a `from` import takes several), else the path as
/// the language resolves it.
fn resolved_target(
    packages: &WorkspacePackages,
    rel_path: &str,
    imp: &ImportStatement,
    go_module_prefix: Option<&str>,
    py_files: &HashSet<String>,
) -> String {
    if Language::from_path(rel_path) == Language::Python {
        return python_targets(rel_path, imp, py_files).into_iter().next().unwrap_or_else(|| imp.raw_path.clone());
    }
    let resolved = resolve_in(packages, rel_path, &imp.raw_path, go_module_prefix);
    if packages.knows_ts_file(&resolved) {
        return resolved;
    }
    normalize_path_in(&resolved, Language::from_path(rel_path))
}

/// The Python files an import may name: every `.py` file under the root that
/// is not excluded, by the same walk the grade makes. Not filtered by whether
/// the project grades Python: the single-file check asks about a file that may
/// be the first one a layer key covers, before it is written.
async fn python_files(root: &Path) -> Result<HashSet<String>, AnalysisError> {
    Ok(walk_source_files(root)?.into_iter().filter(|f| is_python_file(f)).collect())
}

/// Re-score `result` with the project's error-severity rule findings
/// (ADR-2609211430 §1), keeping the coverage ceiling (ADR-2609241707). The
/// rules file is read by the caller that owns it; the arithmetic is here.
pub fn apply_rule_errors(result: &mut ArchAnalysisResult, rule_errors: usize) {
    result.health_score = ArchAnalysisResult::compute_health_score(
        result.violations.len(),
        result.circular_deps.len(),
        result.dead_exports.len(),
        result.unused_ports.len() + result.api.unserved.len(),
        rule_errors + result.api.errors.len() + result.config_errors.len(),
    )
    .min(result.coverage.ceiling());
}

/// One edge per target a file reaches. A module both imported and named
/// inline is one crossing, and each violation costs ten points: an import
/// edge joins two files, so naming the same file twice, by alias and by
/// relative path, is still one dependency.
fn one_edge_per_target(edges: Vec<ImportEdge>) -> Vec<ImportEdge> {
    let mut seen = HashSet::new();
    edges.into_iter().filter(|e| seen.insert(e.to_file.clone())).collect()
}

/// The boundary violations in one file, by exactly the rules the grade
/// applies: its imports parsed by tree-sitter, resolved through the
/// project's packages, classified by its declared layers. For the post-edit
/// hook, which checks the file just written without grading the tree.
pub async fn file_violations(
    root: &Path,
    rel_path: &str,
    ast: &dyn AstPort,
) -> Result<Vec<DependencyViolation>, AnalysisError> {
    if Language::from_path(rel_path) == Language::Unknown {
        return Ok(Vec::new());
    }
    let source = tokio::fs::read_to_string(root.join(rel_path)).await?;
    source_violations(root, rel_path, &source, ast).await
}

/// The boundary violations `source` would have as the file at `rel_path`,
/// by the same rules as [`file_violations`]. For the pre-edit hook, which
/// judges an edit's result before it is written.
pub async fn source_violations(
    root: &Path,
    rel_path: &str,
    source: &str,
    ast: &dyn AstPort,
) -> Result<Vec<DependencyViolation>, AnalysisError> {
    let lang = Language::from_path(rel_path);
    if lang == Language::Unknown {
        return Ok(Vec::new());
    }
    let layers = LayerMap::from_project(root).map_err(AnalysisError::Other)?;
    // Python the grade does not read has no findings here either. A file a
    // layer key covers opts the project in, even before it is written.
    if lang == Language::Python && layers.declared(rel_path).is_none() && !python_is_graded(root) {
        return Ok(Vec::new());
    }
    let packages = discover_packages(root);
    let go_mod = detect_go_module_prefix(root).await;
    let imports = ast.extract_imports(Path::new(rel_path), source, lang)?;
    let paths = ast.module_paths(Path::new(rel_path), source, lang)?;
    let py_files = if lang == Language::Python { python_files(root).await? } else { HashSet::new() };
    let mut edges = edges_of(rel_path, &imports, &packages, &layers, go_mod.as_deref(), &py_files);
    edges.extend(edges_of(rel_path, &paths, &packages, &layers, go_mod.as_deref(), &py_files));
    let (adapters, _) = adapters_from_project(root, &layers);
    Ok(boundary_checker::find_violations_in(&one_edge_per_target(edges), &adapters))
}

/// Name the file each violation's import lands in. An edge's target is the
/// path the import spells — `src/fmt/render` for `crate::fmt::render` — and a
/// report that says which file to open is the one a reader can act on. A
/// target no graded file matches is left as written.
fn name_target_files(mut violations: Vec<DependencyViolation>, files: &[FileData]) -> Vec<DependencyViolation> {
    let known: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    for v in &mut violations {
        let target = v.edge.to_file.clone();
        let mut prefix = target.as_str();
        loop {
            let found = std::iter::once(prefix.to_string())
                .chain(["rs", "ts", "tsx", "go"].iter().map(|ext| format!("{prefix}.{ext}")))
                .chain(["mod.rs", "index.ts", "index.tsx"].iter().map(|f| format!("{prefix}/{f}")))
                .find(|c| known.contains(c.as_str()));
            if let Some(file) = found {
                v.edge.to_file = file;
                break;
            }
            match prefix.rsplit_once('/') {
                Some((head, _)) => prefix = head,
                None => break,
            }
        }
    }
    violations
}

/// Which graded files have a layer. A build script is not architecture and
/// counts neither way.
fn coverage_of(file_data: &[FileData], layers: &LayerMap) -> Coverage {
    let mut cov = Coverage::default();
    for f in file_data {
        if f.path.rsplit('/').next() == Some("build.rs") {
            continue;
        }
        cov.total += 1;
        if layers.classify(&f.path) == HexLayer::Unknown {
            cov.unclassified.push(f.path.clone());
        } else {
            cov.classified += 1;
        }
    }
    cov.unclassified.sort();
    cov
}

/// Resolve an import: the project's own packages first, then the language's
/// own rules.
fn resolve_in(packages: &WorkspacePackages, from: &str, raw: &str, go_module_prefix: Option<&str>) -> String {
    packages
        .resolve(from, raw)
        .unwrap_or_else(|| resolve_import_path(from, raw, go_module_prefix))
}

/// The project's own packages, from their manifests: `Cargo.toml` package
/// names, `go.mod` module paths, `package.json` names. Walks what the grade
/// walks — the same exclusions, no hidden directories — so a vendored or
/// excluded manifest never claims an import.
pub(crate) fn discover_packages(root: &Path) -> WorkspacePackages {
    let project_ex_owned = project_excludes(root);
    let project_ex: Vec<&str> = project_ex_owned.iter().map(String::as_str).collect();
    let mut pkgs = WorkspacePackages::default();
    pkgs.set_ts_paths(TsPaths::load(&|rel: &str| std::fs::read_to_string(root.join(rel)).ok()));
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rel = dir.strip_prefix(root).unwrap_or(&dir).to_string_lossy().replace('\\', "/");
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                if !name.starts_with('.')
                    && !matches_exclude(&format!("{child}/"), EXCLUDE_PATTERNS)
                    && !matches_exclude(&child, &project_ex)
                {
                    stack.push(p);
                }
                continue;
            }
            if name.ends_with(".ts") || name.ends_with(".tsx") || is_js_file(&name) {
                pkgs.add_ts_file(&if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") });
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            match name.as_str() {
                "Cargo.toml" => {
                    if let Some(n) = cargo_package_name(&text) {
                        pkgs.add_crate(&n, &rel);
                    }
                }
                "go.mod" => {
                    if let Some(m) = text.lines().find_map(|l| l.trim().strip_prefix("module ")) {
                        pkgs.add_go_module(m.trim().trim_matches('"'), &rel);
                    }
                }
                "package.json" => {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                    if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                        let entry = if dir.join("src/index.ts").is_file() {
                            "src/index.ts".to_string()
                        } else if let Some(m) = v.get("main").and_then(|m| m.as_str()) {
                            normalize_path(m.trim_start_matches("./"))
                        } else {
                            "index.ts".to_string()
                        };
                        pkgs.add_npm_package(n, &rel, &entry);
                    }
                }
                _ => {}
            }
        }
    }
    pkgs
}

/// `name` under `[package]` in a Cargo manifest. A workspace root with no
/// `[package]` has none.
fn cargo_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(v) = t.strip_prefix("name") {
                let v = v.trim_start();
                if let Some(v) = v.strip_prefix('=') {
                    return Some(v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    None
}

/// Detect Go module prefix from go.mod file.
async fn detect_go_module_prefix(root: &Path) -> Option<String> {
    for candidate in &["go.mod", "backend/go.mod", "src/go.mod"] {
        let path = root.join(candidate);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("module ") {
                    return Some(rest.trim().to_string());
                }
            }
        }
    }
    None
}

/// The files the grade reads: the grade's own walk ([`walk_graded_files`]),
/// so a hidden folder it reads and a symlinked folder it follows are listed
/// too. A report that counts files (the "Hex layers" tally) lists these, so
/// it counts exactly what the grade reads (ADR-2611221646 §3). A tree the
/// walk cannot read lists nothing, as the grade then grades nothing.
pub fn graded_files_sync(root: &Path) -> Vec<String> {
    walk_graded_files(root).unwrap_or_default()
}

/// The language the graded sources of a `package.json` project are in: `typescript`
/// when any is `.ts` or `.tsx`, else `javascript` when any is JavaScript, else
/// `None` (a `package.json` that is tooling only). The grade's own list, so
/// `node_modules`, build output, test files and `analyze.exclude` do not count
/// (ADR-2611271955 §1, §3, §4).
pub fn node_sources_language(root: &Path) -> Option<&'static str> {
    let files = graded_files_sync(root);
    if files.iter().any(|f| f.ends_with(".ts") || f.ends_with(".tsx")) {
        Some("typescript")
    } else if files.iter().any(|f| is_js_file(f)) {
        Some("javascript")
    } else {
        None
    }
}

/// What a `package.json` project is called: `typescript` when it has a
/// `tsconfig.json` at its root or TypeScript among its graded sources or no
/// source at all, `javascript` when its graded sources are JavaScript only
/// (ADR-2611271955 §1). One rule for every surface that names the language.
pub fn package_language(root: &Path) -> &'static str {
    if root.join("tsconfig.json").is_file() {
        return "typescript";
    }
    node_sources_language(root).unwrap_or("typescript")
}

/// The files `hexa analyze` grades, listed synchronously for the display
/// detectors. Same rules as the async walk: `EXCLUDE_PATTERNS`, the
/// project's `analyze.exclude`, test files out. Paths are project-relative
/// with `/` separators, sorted.
///
/// This exists so every detector scans the same tree. Before it, each
/// display detector walked on its own and none excluded `examples/`, so
/// hexa's own display line counted 136 orphans and 152 duplications that
/// were all in example projects.
pub fn source_files_sync(root: &Path) -> Vec<String> {
    let project_ex_owned = project_excludes(root);
    let project_ex: Vec<&str> = project_ex_owned.iter().map(String::as_str).collect();
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        if e.path() == root {
            return true;
        }
        let rel = e
            .path()
            .strip_prefix(root)
            .unwrap_or(e.path())
            .to_string_lossy()
            .replace('\\', "/");
        let hidden = e.file_name().to_string_lossy().starts_with('.');
        !hidden
            && !excluded_by_default(&rel)
            && !matches_exclude(&rel, &project_ex)
    });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        if is_source_file(&rel) {
            files.push(rel);
        }
    }
    if !python_is_graded(root) {
        files.retain(|f| !is_python_file(f));
    }
    files.sort();
    files
}

/// The files the API contract is read from, by `hexa api` and by the grade's
/// API findings alike. Its Python is exactly the Python the grade reads
/// (ADR-2611261303 §5); the other languages are listed as they always were.
pub fn api_files_sync(root: &Path) -> std::io::Result<Vec<String>> {
    let mut files = source_files_sync(root);
    files.retain(|f| !is_python_file(f));
    // A tree that cannot be walked is an error: read as empty, it said there
    // were no tagged ports.
    files.extend(walk_graded_files(root)?.into_iter().filter(|f| is_python_file(f)));
    files.sort();
    Ok(files)
}

/// Exports and identifier counts for every graded file, synchronously.
/// Imports are left empty; the display detectors do not read them.
pub fn load_file_data_sync(root: &Path, adapter: &dyn AstPort) -> Vec<FileData> {
    let mut out = Vec::new();
    for rel in source_files_sync(root) {
        let Ok(source) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let lang = Language::from_path(&rel);
        let exports = adapter.extract_exports(Path::new(&rel), &source, lang).unwrap_or_default();
        let references = adapter.extract_references(Path::new(&rel), &source, lang).unwrap_or_default();
        let members = adapter.extract_members(Path::new(&rel), &source, lang).unwrap_or_default();
        out.push(FileData { path: file_id(&rel), imports: vec![], exports, references, members });
    }
    out
}

/// Recursively collect source files under a directory.
pub(crate) async fn collect_source_files(root: &Path) -> Result<Vec<String>, AnalysisError> {
    Ok(walk_graded_files(root)?)
}

fn is_python_file(path: &str) -> bool {
    Language::from_path(path) == Language::Python
}

/// Whether the project says it is Python (ADR-2611251938 §1): it has a Python
/// manifest, or an `analyze.layers` key covers one of the Python files in
/// `files`.
fn declares_python(root: &Path, files: &[String]) -> bool {
    if has_python_manifest(root) {
        return true;
    }
    // A layer map that does not parse is reported by the analysis itself.
    let layers = LayerMap::from_project(root).unwrap_or_default();
    files.iter().any(|f| is_python_file(f) && layers.declared(f).is_some())
}

fn has_python_manifest(root: &Path) -> bool {
    PYTHON_MANIFESTS.iter().any(|m| root.join(m).is_file())
}

/// Whether the grade reads this project's Python. A manifest answers without
/// walking the tree, so an unreadable folder cannot turn Python off.
pub fn python_is_graded(root: &Path) -> bool {
    has_python_manifest(root) || walk_source_files(root).is_ok_and(|files| declares_python(root, &files))
}

/// How many Python files the grade would read and does not, because the
/// project does not say it is Python.
pub fn ungraded_python_count(root: &Path) -> std::io::Result<usize> {
    let files = walk_source_files(root)?;
    if declares_python(root, &files) {
        return Ok(0);
    }
    Ok(files.iter().filter(|f| is_python_file(f)).count())
}

/// The grade's walk: [`walk_source_files`], less the Python of a project that
/// does not say it is Python. One walk, shared by the grade and by
/// [`graded_files_sync`], so the two cannot drift apart.
fn walk_graded_files(root: &Path) -> std::io::Result<Vec<String>> {
    let mut files = walk_source_files(root)?;
    if !declares_python(root, &files) {
        files.retain(|f| !is_python_file(f));
    }
    Ok(files)
}

/// Every directory not matched by `EXCLUDE_PATTERNS` or `analyze.exclude`
/// (symlinked directories followed), every source file in them that is not a
/// Python test or excluded.
fn walk_source_files(root: &Path) -> std::io::Result<Vec<String>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let project_ex_owned = project_excludes(root);
    let project_ex: Vec<&str> = project_ex_owned.iter().map(String::as_str).collect();

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let Ok(entry) = entry else { break };
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            if path.is_dir() {
                // Skip excluded directories
                if !matches_exclude(&rel, EXCLUDE_PATTERNS) && !matches_exclude(&rel, &project_ex) {
                    stack.push(path);
                }
            } else if is_source_file(&rel)
                && !is_python_test(&rel)
                && !excluded_by_default(&rel)
                && !matches_exclude(&rel, &project_ex)
            {
                files.push(rel);
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Whether `path` is a `.pyi` stub with its `.py` module beside it.
fn describes_a_module(path: &str, py_files: &HashSet<String>) -> bool {
    path.strip_suffix(".pyi").is_some_and(|stem| py_files.contains(&format!("{stem}.py")))
}

/// Per file, the names its own code uses (ADR-2611251938 §2).
type HomeUses = HashMap<String, HashSet<String>>;

/// `dead`, less each name the module that defines it uses itself.
fn without_home_uses(mut dead: Vec<DeadExport>, uses: &HomeUses) -> Vec<DeadExport> {
    dead.retain(|d| !uses.get(&d.file).is_some_and(|names| names.contains(&d.export_name)));
    dead
}

/// Test file patterns — files matching these are collected as additional consumers.
const TEST_PATTERNS: &[&str] = &[
    ".test.ts", ".spec.ts", ".test.tsx", ".spec.tsx", ".test.js", ".spec.js", ".test.jsx", ".spec.jsx", ".test.mjs", ".spec.mjs",
    ".test.cjs", ".spec.cjs", "_test.go", ".test.rs",
];

/// A Python test module by its name: `test_*.py` or `*_test.py` (pytest's
/// discovery rule; ADR-2611211324 §4).
fn is_python_test(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.ends_with(".py") && (base.starts_with("test_") || base.ends_with("_test.py"))
}

fn is_test_file(path: &str) -> bool {
    let segmented = format!("/{}/", path.trim_matches('/'));
    is_python_test(path)
        || TEST_PATTERNS.iter().any(|p| path.ends_with(p))
        || ["/tests/", "/test/", "/__tests__/"].iter().any(|d| segmented.contains(d))
}

/// Collect test files that may import from main source files.
async fn collect_test_files(root: &Path) -> Result<Vec<String>, AnalysisError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let skip_dirs = ["node_modules", "dist", "examples", "target", ".venv", "venv", "site-packages", "__pycache__"];

    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            if path.is_dir() {
                if !rel.split('/').any(|seg| skip_dirs.contains(&seg)) {
                    stack.push(path);
                }
            } else if is_source_file(&rel) && is_test_file(&rel) {
                files.push(rel);
            }
        }
    }

    files.sort();
    Ok(files)
}

// ── Analyzer ─────────────────────────────────────────────

/// Orchestrates all architecture analysis checks.
pub struct ArchAnalyzer {
    ast: Arc<dyn AstPort>,
}

impl ArchAnalyzer {
    pub fn new(ast: Arc<dyn AstPort>) -> Self {
        Self { ast }
    }

    /// Parse all source files and build import edges + export data.
    async fn collect_file_data(
        &self,
        root: &Path,
        go_module_prefix: Option<&str>,
    ) -> Result<(Vec<ImportEdge>, Vec<FileData>, HomeUses), AnalysisError> {
        let source_files = collect_source_files(root).await?;
        let py_files: HashSet<String> = source_files.iter().filter(|f| is_python_file(f)).cloned().collect();
        let layers = LayerMap::from_project(root).map_err(AnalysisError::Other)?;
        let packages = discover_packages(root);
        let mut all_edges = Vec::new();
        let mut all_file_data = Vec::new();
        let mut home_uses = HomeUses::new();

        for rel_path in &source_files {
            let abs_path = root.join(rel_path);
            let source = tokio::fs::read_to_string(&abs_path).await?;
            let lang = Language::from_path(rel_path);

            let imports = self
                .ast
                .extract_imports(Path::new(rel_path), &source, lang)?;
            let mut exports = self
                .ast
                .extract_exports(Path::new(rel_path), &source, lang)?;
            let mut references = self
                .ast
                .extract_references(Path::new(rel_path), &source, lang)?;
            // A stub beside its module describes it (ADR-2611251938 §1): the
            // names are the module's, judged there. Left in, each read as a
            // dead export, and naming them hid the module's own dead ones.
            if describes_a_module(rel_path, &py_files) {
                for export in exports.drain(..) {
                    references.remove(&export.name);
                }
            }

            let from_file = file_id(rel_path);
            let used = self.ast.names_used_at_home(&source, lang)?;
            if !used.is_empty() {
                home_uses.insert(from_file.clone(), used);
            }

            let paths = self.ast.module_paths(Path::new(rel_path), &source, lang)?;
            let mut edges = edges_of(rel_path, &imports, &packages, &layers, go_module_prefix, &py_files);
            edges.extend(edges_of(rel_path, &paths, &packages, &layers, go_module_prefix, &py_files));
            all_edges.extend(one_edge_per_target(edges));

            let members = self
                .ast
                .extract_members(Path::new(rel_path), &source, lang)
                .unwrap_or_default();
            all_file_data.push(FileData {
                path: from_file,
                imports: imports
                    .into_iter()
                    .map(|mut imp| {
                        imp.resolved_path = resolved_target(&packages, rel_path, &imp, go_module_prefix, &py_files);
                        imp
                    })
                    .collect(),
                exports,
                references,
                members,
            });
        }

        Ok((all_edges, all_file_data, home_uses))
    }

    /// Collect test files as import consumers (their imports count, exports don't).
    async fn collect_test_file_data(
        &self,
        root: &Path,
        go_module_prefix: Option<&str>,
    ) -> Result<Vec<FileData>, AnalysisError> {
        let mut test_files = collect_test_files(root).await?;
        // Ungraded Python is not parsed, so it consumes nothing either.
        if !python_is_graded(root) {
            test_files.retain(|f| !is_python_file(f));
        }
        let packages = discover_packages(root);
        let py_files = python_files(root).await?;
        let mut test_data = Vec::new();

        for rel_path in &test_files {
            let abs_path = root.join(rel_path);
            let source = match tokio::fs::read_to_string(&abs_path).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let lang = Language::from_path(rel_path);
            let imports = match self.ast.extract_imports(Path::new(rel_path), &source, lang) {
                Ok(i) => i,
                Err(_) => continue,
            };
            let references = self
                .ast
                .extract_references(Path::new(rel_path), &source, lang)
                .unwrap_or_default();

            let from_file = file_id(rel_path);
            test_data.push(FileData {
                path: from_file,
                imports: imports
                    .into_iter()
                    .map(|mut imp| {
                        imp.resolved_path = resolved_target(&packages, rel_path, &imp, go_module_prefix, &py_files);
                        imp
                    })
                    .collect(),
                exports: vec![],
                references,
                members: HashMap::new(),
            });
        }

        Ok(test_data)
    }
}

#[async_trait]
impl ArchAnalysisPort for ArchAnalyzer {
    async fn analyze(&self, root_path: &Path) -> Result<ArchAnalysisResult, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (edges, file_data, home_uses) =
            self.collect_file_data(root_path, go_mod.as_deref()).await?;

        let layers = LayerMap::from_project(root_path).map_err(AnalysisError::Other)?;
        let (adapters, adapters_error) = adapters_from_project(root_path, &layers);
        let mut config_errors: Vec<String> = adapters_error.into_iter().collect();
        // A layer key that matches no graded file is a declaration the grade
        // would otherwise drop silently (ADR-2611221705 §1).
        config_errors.extend(layers.unmatched_keys(file_data.iter().map(|f| f.path.as_str())));
        let violations = name_target_files(
            boundary_checker::find_violations_in(&edges, &adapters),
            &file_data,
        );
        let circular_deps = cycle_detector::detect_cycles(&edges);

        // Collect test files as additional import consumers for dead export analysis.
        // This prevents false "dead export" reports for symbols only used in tests.
        let test_file_data = self
            .collect_test_file_data(root_path, go_mod.as_deref())
            .await?;
        let dead_exports = without_home_uses(
            dead_export_finder::find_dead_exports_with(&file_data, &test_file_data, &layers),
            &home_uses,
        );

        // Orphan files: no incoming or outgoing edges
        let connected: HashSet<&str> = edges
            .iter()
            .flat_map(|e| [e.from_file.as_str(), e.to_file.as_str()])
            .collect();

        // Resolve Rust `mod foo;` declarations → actual files they reference.
        // `self::mod_name` edges may not match normalized file paths directly,
        // so we do a second pass to connect parent mod.rs → child modules.
        let mut mod_targets: HashSet<String> = HashSet::new();
        for edge in &edges {
            if !edge.import_path.starts_with("self::") {
                continue;
            }
            let mod_name = &edge.import_path["self::".len()..];
            let from_dir = edge.from_file
                .rsplit_once('/')
                .map(|(d, _)| d)
                .unwrap_or("");
            for fd in &file_data {
                let basename = fd.path.rsplit('/').next().unwrap_or(&fd.path);
                let in_same_dir = fd.path.starts_with(from_dir) && fd.path != edge.from_file;
                if in_same_dir
                    && (basename == format!("{}.rs", mod_name)
                        || (basename == "mod.rs"
                            && fd.path.contains(&format!("/{}/", mod_name))))
                {
                    mod_targets.insert(fd.path.clone());
                }
            }
        }

        let orphan_files: Vec<String> = file_data
            .iter()
            .map(|f| f.path.as_str())
            .filter(|f| !connected.contains(f) && !mod_targets.contains(*f))
            .filter(|f| {
                let basename = f.rsplit('/').next().unwrap_or(f);
                // Cargo build scripts are implicitly invoked — never orphans
                if basename == "build.rs" {
                    return false;
                }
                // Standalone scripts are not part of the import graph
                if f.starts_with("scripts/") || f.contains("/scripts/") {
                    return false;
                }
                true
            })
            .map(|f| f.to_string())
            .collect();

        // Unused ports: port interfaces with no adapter importing them
        let unused_ports = detect_unused_ports(&file_data);

        // 0 rule errors: this crate analyses structure and never reads the
        // project's rules file. The caller that owns that file applies the
        // term (ADR-2609211430 §1) — in hexa's case `analyze::deep_analysis`,
        // which is the one door every graded surface goes through.
        let coverage = coverage_of(&file_data, &layers);

        // The API contract (ADR-2610092245 §6): a tag that cannot be read is
        // a rule error, and a tagged port no primary adapter names is a port
        // nothing uses.
        let api = api_contract::findings(root_path, &api_files_sync(root_path)?, self.ast.as_ref())?;
        let health_score = ArchAnalysisResult::compute_health_score(
            violations.len(),
            circular_deps.len(),
            dead_exports.len(),
            unused_ports.len() + api.unserved.len(),
            api.errors.len() + config_errors.len(),
        )
        .min(coverage.ceiling());

        // ADR-056: Frontend hexagonal architecture checks (skipped if no assets/src/)
        let frontend = frontend_checker::check_frontend(root_path);

        Ok(ArchAnalysisResult {
            violations,
            dead_exports,
            circular_deps,
            orphan_files,
            unused_ports,
            health_score,
            file_count: file_data.len(),
            edge_count: edges.len(),
            frontend,
            coverage,
            api,
            config_errors,
            ungraded_python_files: ungraded_python_count(root_path)?,
        })
    }

    async fn validate_boundaries(
        &self,
        root_path: &Path,
    ) -> Result<Vec<DependencyViolation>, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (edges, file_data, _) = self.collect_file_data(root_path, go_mod.as_deref()).await?;
        let layers = LayerMap::from_project(root_path).map_err(AnalysisError::Other)?;
        let (adapters, _) = adapters_from_project(root_path, &layers);
        Ok(name_target_files(boundary_checker::find_violations_in(&edges, &adapters), &file_data))
    }

    async fn find_dead_exports(
        &self,
        root_path: &Path,
    ) -> Result<Vec<DeadExport>, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (_, file_data, home_uses) = self.collect_file_data(root_path, go_mod.as_deref()).await?;
        // Test files consume; an export only a test names is alive.
        let tests = self.collect_test_file_data(root_path, go_mod.as_deref()).await?;
        let layers = LayerMap::from_project(root_path).map_err(AnalysisError::Other)?;
        Ok(without_home_uses(dead_export_finder::find_dead_exports_with(&file_data, &tests, &layers), &home_uses))
    }

    async fn detect_circular_deps(
        &self,
        root_path: &Path,
    ) -> Result<Vec<Vec<String>>, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (edges, _, _) = self.collect_file_data(root_path, go_mod.as_deref()).await?;
        Ok(cycle_detector::detect_cycles(&edges))
    }
}

/// Is this import target inside a ports layer? Matches a file under `ports/`
/// and a Go package path that ends at `ports`.
fn is_ports_path(p: &str) -> bool {
    p.contains("/ports/") || p.ends_with("/ports") || p == "ports"
}

/// Two paths refer to the same module when they match after stripping the
/// extension. Import resolution may carry `.js` (NodeNext), `.ts`, or nothing,
/// and the declaring file carries `.ts`.
fn same_module(a: &str, b: &str) -> bool {
    fn stem(p: &str) -> &str {
        let p = p.strip_suffix(".js").or_else(|| p.strip_suffix(".mjs")).or_else(|| p.strip_suffix(".cjs")).or_else(|| p.strip_suffix(".jsx")).or_else(|| p.strip_suffix(".ts")).or_else(|| p.strip_suffix(".rs")).or_else(|| p.strip_suffix(".go")).unwrap_or(p);
        p.strip_suffix("/index").or_else(|| p.strip_suffix("/mod")).unwrap_or(p)
    }
    let (sa, sb) = (stem(a), stem(b));
    sa == sb || sa.ends_with(sb) || sb.ends_with(sa)
}

fn detect_unused_ports(file_data: &[FileData]) -> Vec<String> {
    // Step 1: Collect port interface names
    let mut port_interfaces: HashSet<String> = HashSet::new();
    let mut port_methods: HashMap<String, HashSet<String>> = HashMap::new(); // port_name → method names

    for file in file_data {
        if !file.path.contains("/ports/") {
            continue;
        }
        for exp in &file.exports {
            if exp.name.ends_with("Port") {
                port_interfaces.insert(exp.name.clone());
            }
        }
        // Collect method-like exports from port files (for Go structural matching)
        // In Go, interface methods are exported as functions from the port package
        for exp in &file.exports {
            if exp.name.ends_with("Port") {
                // The port interface itself — methods would be in the same file
                // as separate function exports (Go) or inside the trait (Rust)
                continue;
            }
            // Associate methods with their likely port (heuristic: same file)
            for port in &port_interfaces {
                port_methods
                    .entry(port.clone())
                    .or_default()
                    .insert(exp.name.clone());
            }
        }
    }

    // Which port interfaces does each ports/ file export? Needed below so an
    // import of the *file* can mark its ports as used.
    let mut ports_by_file: HashMap<String, Vec<String>> = HashMap::new();
    for file in file_data {
        if !file.path.contains("/ports/") {
            continue;
        }
        let names: Vec<String> = file
            .exports
            .iter()
            .filter(|e| e.name.ends_with("Port"))
            .map(|e| e.name.clone())
            .collect();
        if !names.is_empty() {
            ports_by_file.insert(file.path.clone(), names);
        }
    }

    // Step 2: Check imports by adapters/usecases.
    //
    // A port is used if an adapter or use case imports its interface by name,
    // OR imports anything at all from the file that declares it. The second
    // clause is the fix for TypeScript, where the idiomatic port is
    //
    //     export interface GraphInsightPort { ... }
    //     export const graphInsightPort: GraphInsightPort = Object.freeze({ ... });
    //
    // and a component imports the value `graphInsightPort`, never the type.
    // The name check alone reported four correct ports as unused on a real
    // project and cost the refactor that created them four points of grade.
    // Importing from the port's module is what "using the port" looks like in
    // all three languages: Go imports the package, Rust `use`s the module,
    // TypeScript imports from the file.
    let mut implemented_ports: HashSet<String> = HashSet::new();
    for file in file_data {
        let is_adapter = file.path.contains("/adapters/");
        let is_usecase = file.path.contains("/usecases/");
        if !is_adapter && !is_usecase {
            continue;
        }
        for imp in &file.imports {
            for name in &imp.names {
                if port_interfaces.contains(name) {
                    implemented_ports.insert(name.clone());
                }
                // Wildcard import from ports/ means all ports are used. A Go
                // package path ends at `/ports` with no trailing slash, which
                // the old `contains("/ports/")` check missed, so every Go port
                // read as unused.
                if name == "*" && is_ports_path(&imp.resolved_path) {
                    for p in &port_interfaces {
                        implemented_ports.insert(p.clone());
                    }
                }
            }
            // Any import from a ports/ file marks that file's ports as used.
            if is_ports_path(&imp.resolved_path) {
                for (port_file, names) in &ports_by_file {
                    if imp.resolved_path.ends_with(port_file)
                        || port_file.ends_with(&imp.resolved_path)
                        || same_module(&imp.resolved_path, port_file)
                    {
                        for n in names {
                            implemented_ports.insert(n.clone());
                        }
                    }
                }
            }
        }
    }

    // Step 3: Go/Rust structural interface matching
    // If an adapter exports methods that overlap with a port's methods,
    // it likely implements that port (Go implicit interface satisfaction)
    for file in file_data {
        if !file.path.contains("/adapters/") {
            continue;
        }
        if !file.path.ends_with(".go") && !file.path.ends_with(".rs") {
            continue;
        }
        let adapter_methods: HashSet<&str> = file
            .exports
            .iter()
            .map(|e| e.name.as_str())
            .collect();

        for (port_name, methods) in &port_methods {
            if implemented_ports.contains(port_name) {
                continue;
            }
            if methods.is_empty() {
                continue;
            }
            // If all port methods are found in the adapter, it likely implements the port
            let all_match = methods.iter().all(|m| adapter_methods.contains(m.as_str()));
            if all_match {
                implemented_ports.insert(port_name.clone());
            }
        }
    }

    port_interfaces
        .difference(&implemented_ports)
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::treesitter_adapter::TreeSitterAdapter;

    fn make_analyzer() -> ArchAnalyzer {
        ArchAnalyzer::new(Arc::new(TreeSitterAdapter::new()))
    }

    /// Claude Code agent worktrees live under `.claude/worktrees/<name>/` and
    /// hold full copies of the source. Counted, they were 61 files with no
    /// layer and capped one project's grade at B 84 (coverage 321/382).
    #[tokio::test]
    async fn claude_agent_worktrees_are_not_project_sources() {
        let dir = tempfile::tempdir().unwrap();
        for rel in ["src/lib.rs", ".claude/worktrees/agent-1/src/lib.rs"] {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "pub fn f() {}\n").unwrap();
        }
        let files = collect_source_files(dir.path()).await.unwrap();
        assert_eq!(files, vec!["src/lib.rs".to_string()], "{files:?}");
    }

    /// The exclusion is the directory, not the word: a crate named `worktrees`
    /// is analysed.
    #[test]
    fn a_directory_that_merely_contains_the_word_is_still_analysed() {
        assert!(matches_exclude(".claude/worktrees/agent-1/src/lib.rs", EXCLUDE_PATTERNS));
        assert!(!matches_exclude("crates/worktrees/src/lib.rs", EXCLUDE_PATTERNS));
        assert!(!matches_exclude("src/claude/worktrees/lib.rs", EXCLUDE_PATTERNS));
    }

    /// A generated Python migration is not graded; a TypeScript or Go
    /// migration is code a person wrote, and is.
    #[test]
    fn only_a_python_migration_is_left_out() {
        assert!(excluded_by_default("pkg/migrations/0001_initial.py"));
        assert!(!excluded_by_default("src/migrations/1700000000-AddUser.ts"));
        assert!(!excluded_by_default("db/migrations/00001_init.go"));
        assert!(excluded_by_default("app/.venv/lib/x.ts"), "an environment is never project code");
    }

    #[tokio::test]
    async fn analyze_nonexistent_dir() {
        let analyzer = make_analyzer();
        let result = analyzer.analyze(Path::new("/nonexistent/dir")).await;
        assert!(result.is_err());
    }

    #[test]
    fn health_score_perfect() {
        assert_eq!(ArchAnalysisResult::compute_health_score(0, 0, 0, 0, 0), 100);
    }

    #[test]
    fn health_score_with_violations() {
        // 2 violations = -20, 1 cycle = -15 → 65
        assert_eq!(ArchAnalysisResult::compute_health_score(2, 1, 0, 0, 0), 65);
    }

    #[test]
    fn health_score_capped_dead_exports() {
        // 50 dead exports capped at -20
        assert_eq!(ArchAnalysisResult::compute_health_score(0, 0, 50, 0, 0), 80);
    }

    #[test]
    fn health_score_floor_at_zero() {
        assert_eq!(ArchAnalysisResult::compute_health_score(10, 5, 30, 10, 0), 0);
    }
}
