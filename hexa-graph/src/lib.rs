//! hexa-graph — Rust-native knowledge-graph engine.
//!
//! Builds a typed knowledge graph from a project's source + docs: AST entities and
//! imports become `Extracted` nodes/edges; an injected [`SemanticExtractor`] can add
//! `Inferred`/`Ambiguous` relationships from prose. Communities are detected via
//! label propagation. Querying (`query`/`shortest_path`/`explain`) lives in [`query`].
//!
//! The engine is network-free and deterministic — the LLM, the daemon, and the
//! filesystem walk policy are the only impure inputs, all explicit.

// `#[async_trait]` marks each method `#[must_use]` and returns a boxed future, which is
// already must-use; clippy 1.99's `double_must_use` flags the pair in the expansion.
#![allow(clippy::double_must_use)]

pub mod community;
pub mod context;
pub mod extract;
pub mod model;
pub mod ports;
pub mod query;
pub mod semantic;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use walkdir::WalkDir;

use extract::code::{self, Language};
use extract::markdown;
use ports::ImportKind;
use model::{edge_id, id_for, Confidence, Edge, EdgeKind, KnowledgeGraph, Node, NodeKind};
use semantic::{SemanticContext, SemanticExtractor};

/// Build mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// AST + docs only — no LLM calls (deterministic, free).
    Ast,
    /// AST + docs + LLM-inferred semantic edges from prose.
    Deep,
}

impl Mode {
    /// Read a mode from its label. Anything unrecognised is `Ast`, so this
    /// cannot fail and is not a `FromStr` implementation.
    pub fn from_label(s: &str) -> Self {
        match s {
            "deep" => Mode::Deep,
            _ => Mode::Ast,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Ast => "ast",
            Mode::Deep => "deep",
        }
    }
}

/// Build options.
#[derive(Debug, Clone)]
pub struct BuildOpts {
    pub project_id: String,
    pub mode: Mode,
    /// Include Markdown/text docs as nodes.
    pub include_docs: bool,
    /// Max file size to read, in bytes (skip larger).
    pub max_file_bytes: u64,
}

impl Default for BuildOpts {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            mode: Mode::Ast,
            include_docs: true,
            max_file_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Max distinct entities an ambiguous imported name may link to before we drop it
/// as too noisy (prevents common names from forming false hubs).
const MAX_AMBIGUOUS_REFS: usize = 3;

/// Where `hexa graph build` writes the graph, relative to the project root.
/// Under `.hexa/`, which a hexa project already ignores, so a build leaves no
/// untracked directory at the repository root.
pub const GRAPH_FILE: &str = ".hexa/graph/graph.json";

/// Where the graph was written before it moved under `.hexa/`. Still read, so
/// a graph built by an older hexa keeps answering until the next build.
pub const LEGACY_GRAPH_FILE: &str = "graph-out/graph.json";

/// The graph file to read for `root`: the current location when it exists,
/// else the legacy one when that exists, else the current location (which a
/// build would create, and which a "no graph" error should name).
pub fn graph_file(root: &Path) -> std::path::PathBuf {
    let current = root.join(GRAPH_FILE);
    if current.is_file() {
        return current;
    }
    let legacy = root.join(LEGACY_GRAPH_FILE);
    if legacy.is_file() {
        return legacy;
    }
    current
}

const EXCLUDE_DIRS: &[&str] = &[
    // hexa's and Claude Code's own folders: skill docs and caches, not the
    // project. Indexed, `.claude/` skills outranked project code in `query`.
    ".claude",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".git",
    ".hexa",
    "graph-out",
    ".next",
    "__pycache__",
];

/// Build a knowledge graph rooted at `root`.
pub async fn build(
    root: &Path,
    opts: BuildOpts,
    semantic: &dyn SemanticExtractor,
) -> anyhow::Result<KnowledgeGraph> {
    let mut graph = KnowledgeGraph {
        project_id: opts.project_id.clone(),
        ..Default::default()
    };

    // Dedup sets so repeated ids never produce duplicate nodes/edges.
    let mut node_ids: HashSet<String> = HashSet::new();
    let mut edge_ids: HashMap<String, usize> = HashMap::new();
    // label (lowercased) → entity node ids, for cross-file reference edges.
    let mut label_index: HashMap<String, Vec<String>> = HashMap::new();
    // entity node id → its file, to resolve a reference to the right definition.
    let mut id_file: HashMap<String, String> = HashMap::new();
    // set of known relative file paths, for import resolution.
    let mut file_set: HashSet<String> = HashSet::new();
    // Go file → its package clause, and the Go method entity ids: the
    // same-package links are keyed by (directory, package) and never target
    // a method.
    let mut go_package: HashMap<String, String> = HashMap::new();
    let mut go_method_ids: HashSet<String> = HashSet::new();
    // Go file → the (type, key) of each bare key of a literal of a named type.
    let mut go_literal_keys: HashMap<String, Vec<(String, String)>> = HashMap::new();
    // collected (file_rel, raw_path, names, kind) imports to resolve after all files seen.
    let mut pending_imports: Vec<(String, String, Vec<String>, ImportKind)> = Vec::new();
    // prose chunks for the deep semantic pass: (file_rel, prose).
    let mut prose_chunks: Vec<(String, String)> = Vec::new();

    // Returns true only the first time an id is seen, so callers can keep their
    // own per-node bookkeeping (label_index) free of duplicates.
    let push_node = |graph: &mut KnowledgeGraph, seen: &mut HashSet<String>, node: Node| -> bool {
        if seen.insert(node.id.clone()) {
            graph.nodes.push(node);
            return true;
        }
        false
    };

    // The project's own files only (ADR-2611221610 §1): sorted, so the
    // traversal is deterministic and independent of filesystem order.
    for rel in source_files(root) {
        let path_buf = root.join(&rel);
        let path = path_buf.as_path();
        let rel_lossy = rel.clone();

        // Size guard.
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.len() > opts.max_file_bytes {
                continue;
            }
        }

        let is_code = Language::from_path(&rel_lossy).is_some();
        let is_doc = opts.include_docs && markdown::is_doc_file(&rel_lossy);
        if !is_code && !is_doc {
            continue;
        }

        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue, // binary / unreadable — skip
        };

        // File node.
        let file_id = id_for(NodeKind::File, &rel_lossy, &rel_lossy);
        file_set.insert(rel_lossy.clone());
        push_node(
            &mut graph,
            &mut node_ids,
            Node {
                id: file_id.clone(),
                kind: NodeKind::File,
                label: rel_lossy.clone(),
                file: rel_lossy.clone(),
                line: 0,
                degree: 0,
                community: 0,
            },
        );

        if let Some(lang) = Language::from_path(&rel_lossy) {
            let fx = code::extract_file(&source, lang);
            for ent in fx.entities {
                let eid = id_for(ent.kind, &rel_lossy, &ent.name);
                let is_new = push_node(
                    &mut graph,
                    &mut node_ids,
                    Node {
                        id: eid.clone(),
                        kind: ent.kind,
                        label: ent.name.clone(),
                        file: rel_lossy.clone(),
                        line: ent.line,
                        degree: 0,
                        community: 0,
                    },
                );
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Defines,
                    &file_id,
                    &eid,
                    Confidence::Extracted,
                );
                id_file.insert(eid.clone(), rel_lossy.clone());
                // Only index a freshly created node. Two entities can share a kind,
                // file and name — cfg-gated Rust duplicates, Go methods with the same
                // name on different receivers — and they collapse onto one node id.
                // Indexing the id twice would make the ambiguity counts below measure
                // entries rather than distinct entities.
                if is_new {
                    label_index
                        .entry(ent.name.to_ascii_lowercase())
                        .or_default()
                        .push(eid);
                }
            }
            if let Some(pkg) = fx.go_package {
                go_package.insert(rel_lossy.clone(), pkg);
            }
            for m in &fx.go_methods {
                go_method_ids.insert(id_for(NodeKind::Function, &rel_lossy, m));
            }
            if !fx.go_literal_keys.is_empty() {
                go_literal_keys.insert(rel_lossy.clone(), fx.go_literal_keys);
            }
            for imp in fx.imports {
                pending_imports.push((rel_lossy.clone(), imp.raw_path, imp.names, imp.kind));
            }
        }

        if is_doc {
            let dx = markdown::extract_doc(&source);
            for h in dx.headings {
                let cid = id_for(NodeKind::DocConcept, &rel_lossy, &h.title);
                push_node(
                    &mut graph,
                    &mut node_ids,
                    Node {
                        id: cid.clone(),
                        kind: NodeKind::DocConcept,
                        label: h.title.clone(),
                        file: rel_lossy.clone(),
                        line: h.line,
                        degree: 0,
                        community: 0,
                    },
                );
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Defines,
                    &file_id,
                    &cid,
                    Confidence::Extracted,
                );
            }
            if opts.mode == Mode::Deep && !dx.prose.trim().is_empty() {
                prose_chunks.push((rel_lossy.clone(), dx.prose));
            }
        }
    }

    // ── Resolve import + reference edges (now that all files/entities are known).
    let crate_roots = crate_roots(root, &file_set);
    let ts_paths = load_ts_paths(root);
    let id_label: HashMap<String, String> =
        graph.nodes.iter().map(|n| (n.id.clone(), n.label.clone())).collect();
    for (from_rel, raw_path, names, kind) in &pending_imports {
        let from_file_id = id_for(NodeKind::File, from_rel, from_rel);
        // A Python `from` import can reach one file per name it takes
        // (`from pkg import a, b`); the first is the one names resolve
        // against, and every one is an import edge (ADR-2611211324 §2).
        let (resolved, also): (Option<String>, Vec<String>) = if is_python(from_rel) {
            let mut t = python_targets(from_rel, raw_path, names, &file_set).into_iter();
            (t.next(), t.collect())
        } else if is_typescript(from_rel) && !raw_path.starts_with('.') && !ts_paths.is_empty() {
            let aliased = resolve_ts_alias(&ts_paths, raw_path, &file_set).filter(|t| t != from_rel);
            (aliased.or_else(|| resolve_import(from_rel, raw_path, &file_set, &crate_roots)), Vec::new())
        } else {
            (resolve_import(from_rel, raw_path, &file_set, &crate_roots), Vec::new())
        };

        match kind {
            // `mod x;` wires a module in. It is not a use of its items.
            ImportKind::ModDecl => {
                if let Some(target_rel) = &resolved {
                    let to_file_id = id_for(NodeKind::File, target_rel, target_rel);
                    add_edge(
                        &mut graph,
                        &mut edge_ids,
                        EdgeKind::Declares,
                        &from_file_id,
                        &to_file_id,
                        Confidence::Extracted,
                    );
                }
                continue;
            }
            // A qualified path links only to an item the resolved file defines.
            // An unresolvable prefix falls back to the crate root, and a name
            // match there or repo-wide would be a guess.
            ImportKind::PathExpr => {
                let Some(target_rel) = &resolved else { continue };
                let hits: Vec<String> = names
                    .iter()
                    .filter_map(|name| label_index.get(&name.to_ascii_lowercase()))
                    .flatten()
                    .filter(|tid| {
                        id_file.get(tid.as_str()).map(|f| f.as_str()) == Some(target_rel.as_str())
                    })
                    .cloned()
                    .collect();
                if hits.is_empty() {
                    continue;
                }
                let to_file_id = id_for(NodeKind::File, target_rel, target_rel);
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Imports,
                    &from_file_id,
                    &to_file_id,
                    Confidence::Extracted,
                );
                for tid in &hits {
                    add_edge(
                        &mut graph,
                        &mut edge_ids,
                        EdgeKind::References,
                        &from_file_id,
                        tid,
                        Confidence::Extracted,
                    );
                }
                continue;
            }
            // A Go file names its package's entities without an import: link
            // each name to the entity of exactly that name another Go file of
            // the same directory and package clause defines. A method is not
            // a target: a bare name never denotes one.
            ImportKind::SamePackage => {
                let dir = std::path::Path::new(from_rel).parent();
                let pkg = go_package.get(from_rel.as_str());
                let in_package = |f: &str| {
                    f.ends_with(".go") && std::path::Path::new(f).parent() == dir && go_package.get(f) == pkg
                };
                // A literal's bare key is a field name when the package
                // declares its type a struct; for any other named type it is
                // an expression, and may be a use (over-report when unsure).
                let is_package_struct = |ty: &str| {
                    label_index.get(&ty.to_ascii_lowercase()).is_some_and(|ids| {
                        ids.iter().any(|tid| {
                            id_file.get(tid.as_str()).is_some_and(|f| {
                                in_package(f) && *tid == id_for(NodeKind::Struct, f, ty)
                            })
                        })
                    })
                };
                let mut names = names.clone();
                for (ty, key) in go_literal_keys.get(from_rel.as_str()).into_iter().flatten() {
                    if !is_package_struct(ty) && !names.contains(key) {
                        names.push(key.clone());
                    }
                }
                for name in &names {
                    let Some(targets) = label_index.get(&name.to_ascii_lowercase()) else {
                        continue;
                    };
                    for tid in targets {
                        let Some(f) = id_file.get(tid.as_str()) else { continue };
                        if f == from_rel
                            || !f.ends_with(".go")
                            || std::path::Path::new(f).parent() != dir
                            || go_package.get(f.as_str()) != pkg
                            || go_method_ids.contains(tid.as_str())
                            || id_label.get(tid.as_str()) != Some(name)
                        {
                            continue;
                        }
                        add_edge(
                            &mut graph,
                            &mut edge_ids,
                            EdgeKind::References,
                            &from_file_id,
                            tid,
                            Confidence::Extracted,
                        );
                    }
                }
                continue;
            }
            ImportKind::Use => {}
        }

        // File→file import edge when the path resolves inside the project.
        for target_rel in resolved.iter().chain(also.iter()) {
            let to_file_id = id_for(NodeKind::File, target_rel, target_rel);
            add_edge(
                &mut graph,
                &mut edge_ids,
                EdgeKind::Imports,
                &from_file_id,
                &to_file_id,
                Confidence::Extracted,
            );
        }

        // File→entity reference edges. Precision matters: linking an imported name
        // to *every* entity that shares that label repo-wide turns common type names
        // ("State", "Config") into false hubs that collapse community detection.
        // So we prefer the definition in the resolved import file, then a unique
        // repo-wide match, and only tag a small ambiguous fan-out — never a large one.
        for name in names {
            if name == "*" || name == "default" {
                continue;
            }
            let Some(targets) = label_index.get(&name.to_ascii_lowercase()) else {
                continue;
            };
            // Entities not defined in the importing file itself.
            let candidates: Vec<&String> = targets
                .iter()
                .filter(|tid| {
                    id_file.get(tid.as_str()).map(|f| f.as_str()) != Some(from_rel.as_str())
                })
                .collect();
            if candidates.is_empty() {
                continue;
            }

            // 1. Precise — the resolved import file defines this symbol.
            if let Some(target_rel) = &resolved {
                if let Some(tid) = candidates.iter().find(|tid| {
                    id_file.get(tid.as_str()).map(|f| f.as_str()) == Some(target_rel.as_str())
                }) {
                    add_edge(
                        &mut graph,
                        &mut edge_ids,
                        EdgeKind::References,
                        &from_file_id,
                        tid,
                        Confidence::Extracted,
                    );
                    continue;
                }
            }

            // 2. Unambiguous — exactly one entity repo-wide carries this label.
            if candidates.len() == 1 {
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::References,
                    &from_file_id,
                    candidates[0],
                    Confidence::Extracted,
                );
                continue;
            }

            // 3. Mildly ambiguous — link a few, tagged Ambiguous. Names matching
            //    many entities are dropped as too noisy to be meaningful.
            if candidates.len() <= MAX_AMBIGUOUS_REFS {
                for tid in candidates {
                    add_edge(
                        &mut graph,
                        &mut edge_ids,
                        EdgeKind::References,
                        &from_file_id,
                        tid,
                        Confidence::Ambiguous,
                    );
                }
            }
        }
    }

    // ── Deep mode: LLM-inferred semantic edges from doc prose.
    if opts.mode == Mode::Deep {
        let known_labels: Vec<String> = graph
            .nodes
            .iter()
            .map(|n| n.label.clone())
            .take(400)
            .collect();
        for (file_rel, prose) in &prose_chunks {
            let ctx = SemanticContext {
                text: prose.clone(),
                known_labels: known_labels.clone(),
                file: file_rel.clone(),
            };
            for tri in semantic.infer_edges(&ctx).await {
                let src_id = ensure_concept(&mut graph, &mut node_ids, &label_index, &tri.source);
                let dst_id = ensure_concept(&mut graph, &mut node_ids, &label_index, &tri.target);
                if src_id == dst_id {
                    continue;
                }
                let conf = if tri.confident {
                    Confidence::Inferred
                } else {
                    Confidence::Ambiguous
                };
                add_edge(
                    &mut graph,
                    &mut edge_ids,
                    EdgeKind::Related,
                    &src_id,
                    &dst_id,
                    conf,
                );
            }
        }
    }

    // ── Degrees, communities, metadata.
    community::compute_degrees(&mut graph);
    community::detect_communities(&mut graph, 20);
    let gods = community::god_nodes(&graph, 10);
    graph.meta = model::GraphMeta {
        mode: opts.mode.as_str().to_string(),
        node_count: graph.nodes.len(),
        edge_count: graph.edges.len(),
        community_count: graph.communities.len(),
        god_nodes: gods,
    };

    Ok(graph)
}

/// Files a build would index that changed after `since` (the time a graph was
/// written), project-relative, sorted, at most `limit`. A non-empty answer means
/// the graph on disk no longer describes the tree, and a consumer verdict read
/// from it may be wrong.
pub fn newer_sources(root: &Path, since: std::time::SystemTime, limit: usize) -> Vec<String> {
    source_files(root)
        .into_iter()
        .filter(|rel| {
            std::fs::metadata(root.join(rel)).and_then(|m| m.modified()).is_ok_and(|m| m > since)
        })
        .take(limit)
        .collect()
}

/// Where a build records what it read, beside the graph (ADR-2611221610 §2).
pub const STAMP_FILE: &str = ".hexa/graph/stamp.json";

/// The files a build reads, project-relative, forward-slashed and sorted
/// (ADR-2611221610 §1): code and docs only. In a git checkout, the files git
/// would track at `root` (tracked, plus untracked files that are not
/// ignored); outside git, a walk. Either way nothing under a directory below
/// `root` that holds its own `.git` (a nested repository or worktree) and
/// nothing under an excluded directory name.
pub fn source_files(root: &Path) -> Vec<String> {
    let listed = git_listing(root).unwrap_or_else(|| walk_listing(root));
    let mut nested: HashMap<std::path::PathBuf, bool> = HashMap::new();
    let mut out: Vec<String> = listed
        .into_iter()
        .filter(|rel| Language::from_path(rel).is_some() || markdown::is_doc_file(rel))
        .filter(|rel| !rel.split('/').any(|c| EXCLUDE_DIRS.contains(&c)))
        .filter(|rel| !under_nested_repo(root, rel, &mut nested))
        .filter(|rel| root.join(rel).is_file())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// What git would track at `root`, or `None` when `root` is in no checkout.
fn git_listing(root: &Path) -> Option<Vec<String>> {
    let o = std::process::Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(
        o.stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).replace('\\', "/"))
            .collect(),
    )
}

/// Every file under `root`, skipping excluded and nested-repository directories.
fn walk_listing(root: &Path) -> Vec<String> {
    WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !is_excluded(e))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| rel_path(root, e.path()))
        .collect()
}

/// Whether a directory between `root` (exclusive) and `rel` holds a `.git`.
fn under_nested_repo(root: &Path, rel: &str, seen: &mut HashMap<std::path::PathBuf, bool>) -> bool {
    let mut dir = Path::new(rel).parent();
    while let Some(d) = dir {
        if d.as_os_str().is_empty() {
            return false;
        }
        let nested = *seen.entry(d.to_path_buf()).or_insert_with(|| root.join(d).join(".git").exists());
        if nested {
            return true;
        }
        dir = d.parent();
    }
    false
}

/// A sha256 over every file [`source_files`] lists, with its path, size and
/// modification time (ADR-2611221610 §2). An edit, an added file, a deleted
/// file and a checkout of another branch each change it; nothing is parsed.
pub fn tree_fingerprint(root: &Path) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for rel in source_files(root) {
        let (len, mtime) = std::fs::metadata(root.join(&rel))
            .map(|m| {
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                (m.len(), t)
            })
            .unwrap_or((0, 0));
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(len.to_le_bytes());
        h.update(mtime.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

// ── helpers ──────────────────────────────────────────────

fn add_edge(
    graph: &mut KnowledgeGraph,
    seen: &mut HashMap<String, usize>,
    kind: EdgeKind,
    src: &str,
    dst: &str,
    conf: Confidence,
) {
    let id = edge_id(kind, src, dst);
    // The id carries no confidence, so a second write for the same
    // (kind, src, dst) is the same edge seen with different certainty. Imports
    // resolve in file order, so the certain write can arrive after the guess:
    // keep the strongest confidence, not the first one.
    if let Some(&idx) = seen.get(&id) {
        let existing = &mut graph.edges[idx];
        if confidence_rank(conf) > confidence_rank(existing.confidence) {
            existing.confidence = conf;
        }
        return;
    }
    seen.insert(id.clone(), graph.edges.len());
    graph.edges.push(Edge {
        id,
        src: src.to_string(),
        dst: dst.to_string(),
        kind,
        confidence: conf,
        weight: 1.0,
    });
}

/// How certain a confidence level is. Higher wins when the same edge is
/// written twice.
fn confidence_rank(conf: Confidence) -> u8 {
    match conf {
        Confidence::Ambiguous => 0,
        Confidence::Inferred => 1,
        Confidence::Extracted => 2,
    }
}

/// Resolve a concept label to an existing node id, or create a `DocConcept` node.
fn ensure_concept(
    graph: &mut KnowledgeGraph,
    seen: &mut HashSet<String>,
    label_index: &HashMap<String, Vec<String>>,
    label: &str,
) -> String {
    let key = label.to_ascii_lowercase();
    if let Some(ids) = label_index.get(&key) {
        if let Some(first) = ids.first() {
            return first.clone();
        }
    }
    if let Some(existing) = graph.node_by_label(label) {
        return existing.id.clone();
    }
    let id = id_for(NodeKind::DocConcept, "", label);
    if seen.insert(id.clone()) {
        graph.nodes.push(Node {
            id: id.clone(),
            kind: NodeKind::DocConcept,
            label: label.to_string(),
            file: String::new(),
            line: 0,
            degree: 0,
            community: 0,
        });
    }
    id
}

fn is_excluded(entry: &walkdir::DirEntry) -> bool {
    // walkdir applies filter_entry to the root too; rejecting it skips the whole
    // walk, so a root literally named `build` or `target` would yield nothing.
    if entry.depth() == 0 {
        return false;
    }
    if entry.file_type().is_dir() {
        if let Some(name) = entry.file_name().to_str() {
            if EXCLUDE_DIRS.contains(&name) {
                return true;
            }
        }
        // Another repository or worktree inside this one (ADR-2611221610 §1).
        return entry.path().join(".git").exists();
    }
    false
}

/// Project-relative, forward-slashed path.
fn rel_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

/// Resolve an import path to a known project-relative file, if possible.
/// Handles TypeScript relative imports and Rust `self::mod` declarations.
/// Crate ident (`hexa_core`) to that crate's root file (`hexa-core/src/lib.rs`).
///
/// A Rust cross-crate `use` names the crate, never the file. Without this map
/// no edge ever reaches a crate root, and `graph consumers` reports the core
/// of the workspace as safe to delete (ADR-2609122048).
fn crate_roots(root: &Path, files: &HashSet<String>) -> HashMap<String, String> {
    let mut candidates: Vec<(String, String, String)> = Vec::new();
    for f in files {
        // A single-crate project puts its root file at `src/lib.rs`, with no
        // crate directory in the path to read the name from, so it comes from
        // the graph root instead. This is the layout `hexa new` scaffolds.
        let name = match f.strip_suffix("/src/lib.rs") {
            Some(dir) => dir.rsplit('/').next().unwrap_or(dir).to_string(),
            None if f == "src/lib.rs" => match root_crate_name(root) {
                Some(name) => name,
                None => continue,
            },
            None => continue,
        };
        candidates.push((name.replace('-', "_"), name, f.clone()));
    }
    // Two directories can claim one ident: `services/api` and `tools/api`, or
    // `foo-bar` and `foo_bar`, which both normalize to `foo_bar`. `files` is a
    // `HashSet`, whose iteration order std randomizes per process, so letting
    // the last writer win would move the same repo's Imports edge between
    // builds. Sort first and keep the best candidate per ident instead.
    candidates.sort_by(|a, b| crate_root_key(a).cmp(&crate_root_key(b)));
    let mut out = HashMap::new();
    for (ident, _, file) in candidates {
        out.entry(ident).or_insert(file);
    }
    out
}

/// Sort key that picks one crate root per ident, lowest first. A directory that
/// already spells the ident (`foo_bar`) beats one that needed `-` to become `_`
/// (`foo-bar`), because a Rust `use foo_bar::X` names the ident literally. Two
/// equal claims are a tie no path can break, so the shallowest path wins, then
/// the first in alphabetical order — arbitrary, but the same on every build.
fn crate_root_key(cand: &(String, String, String)) -> (&str, u8, usize, &str) {
    let (ident, dir_name, file) = cand;
    (
        ident,
        u8::from(dir_name.contains('-')),
        file.matches('/').count(),
        file,
    )
}

/// The directory name of the graph root, which is the crate name of a crate
/// that sits at the root. `.` and `..` carry no name, so they are resolved.
fn root_crate_name(root: &Path) -> Option<String> {
    match root.file_name() {
        Some(name) => Some(name.to_string_lossy().to_string()),
        None => Some(
            root.canonicalize()
                .ok()?
                .file_name()?
                .to_string_lossy()
                .to_string(),
        ),
    }
}

fn is_typescript(rel: &str) -> bool {
    [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"].iter().any(|e| rel.ends_with(e))
}

fn is_python(rel: &str) -> bool {
    rel.ends_with(".py") || rel.ends_with(".pyi")
}

/// The project files one Python import reaches, in the order its names are
/// written, never the importing file itself. `names` is `["*"]` for a plain
/// `import a.b`. Importing `a.b` runs `a/__init__.py` first, so each package
/// above a target is reached too.
fn python_targets(from_rel: &str, module: &str, names: &[String], files: &HashSet<String>) -> Vec<String> {
    let mut out = python_named_targets(from_rel, module, names, files);
    let mut parents: Vec<String> = Vec::new();
    for t in &out {
        let mut dir: Vec<&str> = t.split('/').collect();
        dir.pop();
        if t.ends_with("/__init__.py") {
            dir.pop();
        }
        while !dir.is_empty() {
            let pkg = dir.join("/");
            let init = format!("{pkg}/__init__.py");
            // A module inside the package does not import its own package by
            // importing a sibling: `from .x import y` in `a/b.py` runs nothing
            // new, and an edge to `a/__init__.py` would be a loop of noise.
            let inside = from_rel.starts_with(&format!("{pkg}/"));
            if files.contains(&init) && !inside {
                parents.push(init);
            }
            dir.pop();
        }
    }
    for p in parents {
        if p != from_rel && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The files the names of one Python import resolve to, first one first.
fn python_named_targets(from_rel: &str, module: &str, names: &[String], files: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let taken: Vec<Option<&str>> = if names.is_empty() { vec![None] } else { names.iter().map(|n| Some(n.as_str())).collect() };
    for name in taken {
        if let Some(t) = resolve_python(from_rel, module, name.filter(|n| *n != "*"), files) {
            if t != from_rel && !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// The project file a Python import names, or `None` for the standard library,
/// a third-party package or a module that does not exist. The rule is
/// `hexa_analysis::path_normalizer::resolve_python_import`'s (ADR-2611211324
/// §2), restated because this crate does not depend on that one; the two are
/// held together by the graph and analysis tests over the same import forms.
/// `from a import b` is the submodule `a/b.py` when there is one, else the
/// package; a relative import starts from the importing file's package, an
/// absolute one from the root, then `src/`.
fn resolve_python(from_rel: &str, module: &str, name: Option<&str>, files: &HashSet<String>) -> Option<String> {
    let dots = module.chars().take_while(|c| *c == '.').count();
    let parts: Vec<&str> = module[dots..].split('.').filter(|s| !s.is_empty()).collect();
    let bases: Vec<Vec<&str>> = if dots > 0 {
        let mut dir: Vec<&str> = from_rel.split('/').collect();
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
        [format!("{stem}.py"), format!("{stem}/__init__.py")].into_iter().find(|c| files.contains(c))
    };
    for base in bases {
        let mut module_path = base;
        module_path.extend(&parts);
        if let Some(n) = name {
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

fn resolve_import(
    from_rel: &str,
    raw: &str,
    files: &HashSet<String>,
    crates: &HashMap<String, String>,
) -> Option<String> {
    if raw.starts_with("./") || raw.starts_with("../") {
        return resolve_relative_ts(from_rel, raw, files);
    }
    if let Some(modname) = raw.strip_prefix("self::") {
        return resolve_rust_mod(from_rel, modname, files);
    }
    let resolved = if let Some(rest) = raw.strip_prefix("crate::") {
        crate_src_dir(from_rel).and_then(|base| resolve_module_in(&base, rest, files))
    } else if let Some(rest) = raw.strip_prefix("super::") {
        resolve_rust_super(from_rel, rest, files)
    } else {
        // Cross-crate. `hexa_core::domain::Foo` names the crate; the file it
        // resolves to is that crate's root.
        raw.split_once("::")
            .and_then(|(head, _)| crates.get(head))
            .cloned()
    };
    // A file never imports itself.
    resolved.filter(|target| target != from_rel)
}

/// The `src` directory of the crate owning this file: `a/src/b/c.rs` to `a/src`.
/// A crate at the graph root has no leading directory: `src/b/c.rs` to `src`.
fn crate_src_dir(from_rel: &str) -> Option<String> {
    if let Some(i) = from_rel.find("/src/") {
        return Some(from_rel[..i + 4].to_string());
    }
    from_rel.starts_with("src/").then(|| "src".to_string())
}

/// Walk a `::` path from longest prefix to shortest and take the first module
/// file that exists. The trailing segments of a `use` name items, not modules,
/// and how many is not knowable from the path alone.
fn resolve_module_in(base: &str, path: &str, files: &HashSet<String>) -> Option<String> {
    let segs: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    for take in (1..=segs.len()).rev() {
        let joined = segs[..take].join("/");
        for cand in [
            format!("{base}/{joined}.rs"),
            format!("{base}/{joined}/mod.rs"),
        ] {
            if files.contains(&cand) {
                return Some(cand);
            }
        }
    }
    // Nothing under the crate matched, so the import reaches the crate root.
    [format!("{base}/lib.rs"), format!("{base}/main.rs")]
        .into_iter()
        .find(|c| files.contains(c))
}

/// `super::` is the parent module: the directory above for a `mod.rs`, this
/// directory for any other file.
fn resolve_rust_super(from_rel: &str, rest: &str, files: &HashSet<String>) -> Option<String> {
    let parent = parent_dir(from_rel);
    let stem = file_stem(from_rel);
    let base = if stem == "mod" || stem == "lib" || stem == "main" {
        parent_dir(&parent)
    } else {
        parent
    };
    if base.is_empty() {
        return None;
    }
    resolve_module_in(&base, rest, files)
}

fn resolve_relative_ts(from_rel: &str, raw: &str, files: &HashSet<String>) -> Option<String> {
    let base_dir = parent_dir(from_rel);
    // Strip a NodeNext `.js`/`.jsx` suffix; the on-disk file is `.ts`/`.tsx`.
    let raw_noext = raw
        .strip_suffix(".js")
        .or_else(|| raw.strip_suffix(".jsx"))
        .unwrap_or(raw);
    let joined = normalize_join(&base_dir, raw_noext)?;
    ts_file_in(&joined, files)
}

/// The file a TypeScript module path names, by the extensions and index
/// files an import tries.
fn ts_file_in(joined: &str, files: &HashSet<String>) -> Option<String> {
    let joined = joined.strip_suffix(".js").or_else(|| joined.strip_suffix(".jsx")).unwrap_or(joined);
    // A TypeScript file wins a tie with its JavaScript namesake, as the
    // compiler's mapping of `./a.js` to `a.ts` does (ADR-2611270420 §3).
    const EXTS: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];
    let mut candidates = vec![joined.to_string()];
    candidates.extend(EXTS.iter().map(|e| format!("{joined}.{e}")));
    candidates.extend(EXTS.iter().map(|e| format!("{joined}/index.{e}")));
    candidates.into_iter().find(|c| files.contains(c))
}

// ── TypeScript path aliases (ADR-2611221645 §4) ──────────
//
// The same resolution `hexa-analysis` makes (`path_normalizer::TsPaths`), so
// `graph consumers` counts the files that import through an alias. hexa-graph
// does not depend on hexa-analysis, so the pure resolver is repeated here;
// adr_2611221645_spec::s25 holds the two to the same edges.

/// The root `tsconfig.json`'s `compilerOptions.paths`, `extends` followed,
/// each target project-relative: (pattern, targets in order).
fn load_ts_paths(root: &Path) -> Vec<(String, Vec<String>)> {
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let (mut chain, mut budget) = (Vec::new(), TSCONFIG_READ_BUDGET);
    let (paths, base) = tsconfig_settings("tsconfig.json", &read, &mut chain, &mut budget);
    let Some((map, decl_dir)) = paths else { return Vec::new() };
    let base_dir = base.unwrap_or(decl_dir);
    map.into_iter()
        .map(|(pattern, targets)| {
            let targets = targets.iter().filter(|t| !t.starts_with('/')).filter_map(|t| normalize_join(&base_dir, t)).collect();
            (pattern, targets)
        })
        .collect()
}

type TsPathsMap = Vec<(String, Vec<String>)>;

/// How many tsconfig files one load may read (as `path_normalizer`).
const TSCONFIG_READ_BUDGET: usize = 256;

/// `chain` is the `extends` path from the root to `rel`: only a file already
/// on it is a cycle, so a base shared by two branches is read for both.
fn tsconfig_settings(
    rel: &str,
    read: &dyn Fn(&str) -> Option<String>,
    chain: &mut Vec<String>,
    budget: &mut usize,
) -> (Option<(TsPathsMap, String)>, Option<String>) {
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
) -> (Option<(TsPathsMap, String)>, Option<String>) {
    let Some(text) = read(rel) else { return (None, None) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)) else { return (None, None) };
    let dir = parent_dir(rel);
    let (mut paths, mut base) = (None, None);
    let extends: Vec<&str> = match v.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.as_str()],
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect(),
        _ => Vec::new(),
    };
    for ext in extends {
        // A package-name extends is not followed (ADR-2611221645 §2).
        if !(ext.starts_with("./") || ext.starts_with("../")) {
            continue;
        }
        let Some(target) = normalize_join(&dir, ext) else { continue };
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
        base = normalize_join(&dir, b);
    }
    if let Some(map) = opts.and_then(|o| o.get("paths")).and_then(|p| p.as_object()) {
        let map = map
            .iter()
            .map(|(k, t)| (k.clone(), t.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()))
            .collect();
        paths = Some((map, dir));
    }
    (paths, base)
}

/// The project file a bare TypeScript specifier names through an alias: an
/// exact pattern first, else the longest matching prefix; its targets in
/// order, the first that names a file.
fn resolve_ts_alias(paths: &[(String, Vec<String>)], spec: &str, files: &HashSet<String>) -> Option<String> {
    let mut best: Option<(&Vec<String>, &str, usize)> = None;
    for (pattern, targets) in paths {
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
    targets.iter().find_map(|t| ts_file_in(&t.replacen('*', capture, 1), files))
}

/// `tsconfig.json` text as plain JSON: comments outside strings removed, a
/// trailing comma before `}` or `]` dropped.
fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let skip_comment = |j: &mut usize| -> bool {
        if chars.get(*j) == Some(&'/') && chars.get(*j + 1) == Some(&'/') {
            while *j < chars.len() && chars[*j] != '\n' {
                *j += 1;
            }
            true
        } else if chars.get(*j) == Some(&'/') && chars.get(*j + 1) == Some(&'*') {
            *j += 2;
            while *j < chars.len() && !(chars[*j] == '*' && chars.get(*j + 1) == Some(&'/')) {
                *j += 1;
            }
            *j += 2;
            true
        } else {
            false
        }
    };
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
        if skip_comment(&mut i) {
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            loop {
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if !skip_comment(&mut j) {
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

fn resolve_rust_mod(from_rel: &str, modname: &str, files: &HashSet<String>) -> Option<String> {
    let parent = parent_dir(from_rel);
    let stem = file_stem(from_rel);
    // `mod.rs`/`lib.rs`/`main.rs` host modules as siblings; otherwise modules live
    // in a subdirectory named after the file stem.
    let base = if stem == "mod" || stem == "lib" || stem == "main" {
        parent
    } else if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{parent}/{stem}")
    };
    let candidates = [
        format!("{base}/{modname}.rs"),
        format!("{base}/{modname}/mod.rs"),
    ];
    candidates.into_iter().find(|c| files.contains(c))
}

fn parent_dir(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

fn file_stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

/// Lexically join `base` with a relative `rel` (handling `.` and `..`).
///
/// Returns `None` when `rel` climbs above `base`: the target then lies outside
/// the graph root, so there is no in-root file it could name. Clamping at the
/// root instead would fabricate an edge to whatever path the escaping suffix
/// happens to collide with.
fn normalize_join(base: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn files(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    /// The defect ADR-2609122048 records: a cross-crate `use` names the crate,
    /// so before this resolved, every crate root looked unimported.
    #[test]
    fn a_cross_crate_use_reaches_the_crate_root() {
        let f = files(&["hexa-core/src/lib.rs", "hexa-cli/src/commands/analyze.rs"]);
        let crates = crate_roots(Path::new("hexa"), &f);
        let got = resolve_import(
            "hexa-cli/src/commands/analyze.rs",
            "hexa_core::domain::Layer",
            &f,
            &crates,
        );
        assert_eq!(got.as_deref(), Some("hexa-core/src/lib.rs"));
    }

    /// Two directories can spell one crate ident. `files` is a `HashSet`, and
    /// std randomizes its iteration order per instance, so the pre-fix
    /// "last writer wins" moved the edge between builds of an unchanged repo.
    /// Rebuild the set many times: every build must name the same file.
    #[test]
    fn a_duplicated_crate_ident_resolves_the_same_way_every_time() {
        for _ in 0..64 {
            let f = files(&[
                "services/api/src/lib.rs",
                "tools/api/src/lib.rs",
                "app/src/main.rs",
            ]);
            let crates = crate_roots(Path::new("hexa"), &f);
            assert_eq!(
                crates.get("api").map(String::as_str),
                Some("services/api/src/lib.rs")
            );
            assert_eq!(
                resolve_import("app/src/main.rs", "api::Foo", &f, &crates).as_deref(),
                Some("services/api/src/lib.rs")
            );
        }
    }

    /// `foo-bar` and `foo_bar` both normalize to the ident `foo_bar`. The
    /// directory that already spells the ident wins, on every build.
    #[test]
    fn an_underscore_directory_wins_the_ident_over_a_hyphen_directory() {
        for _ in 0..64 {
            let f = files(&[
                "foo-bar/src/lib.rs",
                "foo_bar/src/lib.rs",
                "app/src/main.rs",
            ]);
            let crates = crate_roots(Path::new("hexa"), &f);
            assert_eq!(
                crates.get("foo_bar").map(String::as_str),
                Some("foo_bar/src/lib.rs")
            );
        }
    }

    #[test]
    fn a_crate_path_reaches_the_deepest_module_that_exists() {
        let f = files(&[
            "hexa-cli/src/lib.rs",
            "hexa-cli/src/commands/mod.rs",
            "hexa-cli/src/commands/analyze.rs",
            "hexa-cli/src/main.rs",
        ]);
        let crates = crate_roots(Path::new("hexa"), &f);
        let got = resolve_import(
            "hexa-cli/src/main.rs",
            "crate::commands::analyze::run",
            &f,
            &crates,
        );
        assert_eq!(got.as_deref(), Some("hexa-cli/src/commands/analyze.rs"));
    }

    #[test]
    fn super_resolves_against_the_parent_module() {
        let f = files(&[
            "a/src/commands/mod.rs",
            "a/src/commands/plan/mod.rs",
            "a/src/commands/analyze.rs",
        ]);
        let crates = crate_roots(Path::new("hexa"), &f);
        let got = resolve_import(
            "a/src/commands/plan/mod.rs",
            "super::analyze::Thing",
            &f,
            &crates,
        );
        assert_eq!(got.as_deref(), Some("a/src/commands/analyze.rs"));
    }

    #[test]
    fn a_third_party_crate_resolves_to_nothing() {
        let f = files(&["hexa-core/src/lib.rs"]);
        let crates = crate_roots(Path::new("hexa"), &f);
        assert_eq!(
            resolve_import("hexa-core/src/lib.rs", "serde::Deserialize", &f, &crates),
            None
        );
        assert_eq!(
            resolve_import(
                "hexa-core/src/lib.rs",
                "std::collections::HashMap",
                &f,
                &crates
            ),
            None
        );
    }

    /// The layout `hexa new` scaffolds: the crate sits at the graph root, so
    /// every path starts `src/` with no leading crate directory. Before the
    /// fix, `crate_src_dir` looked for `/src/` and found nothing, so no
    /// `crate::` import in a scaffolded project ever produced an edge.
    #[test]
    fn a_crate_path_resolves_in_a_crate_at_the_graph_root() {
        let f = files(&[
            "src/lib.rs",
            "src/domain/mod.rs",
            "src/ports/mod.rs",
            "src/usecases/mod.rs",
            "src/adapters/secondary/mod.rs",
        ]);
        let crates = crate_roots(Path::new("/tmp/counter-app"), &f);
        for from in [
            "src/ports/mod.rs",
            "src/usecases/mod.rs",
            "src/adapters/secondary/mod.rs",
        ] {
            assert_eq!(
                resolve_import(from, "crate::domain::Count", &f, &crates).as_deref(),
                Some("src/domain/mod.rs"),
                "crate::domain from {from}"
            );
        }
    }

    /// A crate at the graph root is still a crate root, so an integration test
    /// naming it by crate ident reaches `src/lib.rs`.
    #[test]
    fn a_crate_at_the_graph_root_is_a_crate_root() {
        let f = files(&["src/lib.rs", "src/domain/mod.rs", "tests/counter.rs"]);
        let crates = crate_roots(Path::new("/tmp/counter-app"), &f);
        assert_eq!(
            crates.get("counter_app").map(String::as_str),
            Some("src/lib.rs")
        );
        assert_eq!(
            resolve_import(
                "tests/counter.rs",
                "counter_app::domain::Count",
                &f,
                &crates
            )
            .as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn a_file_never_imports_itself() {
        let f = files(&["hexa-core/src/lib.rs", "hexa-core/src/domain.rs"]);
        let crates = crate_roots(Path::new("hexa"), &f);
        assert_eq!(
            resolve_import(
                "hexa-core/src/lib.rs",
                "hexa_core::domain::Layer",
                &f,
                &crates
            ),
            None
        );
    }
}
