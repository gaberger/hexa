//! End-to-end engine tests over a temp-dir fixture.

use std::fs;

use hexa_graph::model::{EdgeKind, NodeKind};
use hexa_graph::query;
use hexa_graph::semantic::NoopSemanticExtractor;
use hexa_graph::{build, BuildOpts, Mode};

fn write(dir: &std::path::Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

async fn build_fixture(dir: &std::path::Path) -> hexa_graph::model::KnowledgeGraph {
    write(dir, "src/a.ts", "export function alpha() { return 1; }\n");
    write(
        dir,
        "src/b.ts",
        "import { alpha } from './a.js';\nexport class Beta { run() { return alpha(); } }\n",
    );
    write(dir, "src/lib.rs", "pub mod thing;\npub fn root_fn() {}\n");
    write(dir, "src/thing.rs", "pub struct Widget;\n");
    write(
        dir,
        "README.md",
        "# Project Title\n\nSome prose about Beta and Widget.\n",
    );

    let opts = BuildOpts {
        project_id: "test".into(),
        mode: Mode::Ast,
        include_docs: true,
        ..Default::default()
    };
    build(dir, opts, &NoopSemanticExtractor).await.unwrap()
}

#[tokio::test]
async fn builds_nodes_edges_and_communities() {
    let tmp = tempfile::tempdir().unwrap();
    let g = build_fixture(tmp.path()).await;

    // File nodes exist.
    assert!(g.node("file:src/a.ts").is_some());
    assert!(g.node("file:src/b.ts").is_some());
    assert!(g.node("file:README.md").is_some());

    // Entities extracted with correct kinds.
    let alpha = g.node("function:src/a.ts:alpha").expect("alpha fn");
    assert_eq!(alpha.kind, NodeKind::Function);
    let beta = g.node("class:src/b.ts:Beta").expect("Beta class");
    assert_eq!(beta.kind, NodeKind::Class);
    assert!(g.node("struct:src/thing.rs:Widget").is_some());
    assert!(g.node("function:src/lib.rs:root_fn").is_some());

    // Doc concept from heading.
    assert!(g.node_by_label("Project Title").is_some());

    // Defines edge: file → entity.
    assert!(g
        .edges
        .iter()
        .any(|e| e.kind == EdgeKind::Defines && e.dst == "function:src/a.ts:alpha"));

    // Imports edge: b.ts → a.ts (TS relative import resolved).
    assert!(
        g.edges.iter().any(|e| e.kind == EdgeKind::Imports
            && e.src == "file:src/b.ts"
            && e.dst == "file:src/a.ts"),
        "expected b.ts -> a.ts import edge"
    );

    // Declares edge: lib.rs → thing.rs (Rust `mod thing;` resolved). A module
    // declaration wires the module in; it is not an import of its items.
    assert!(
        g.edges.iter().any(|e| e.kind == EdgeKind::Declares
            && e.src == "file:src/lib.rs"
            && e.dst == "file:src/thing.rs"),
        "expected lib.rs -> thing.rs mod edge"
    );

    // Reference edge: b.ts → alpha (imported symbol matched an entity).
    assert!(
        g.edges.iter().any(|e| e.kind == EdgeKind::References
            && e.src == "file:src/b.ts"
            && e.dst == "function:src/a.ts:alpha"),
        "expected b.ts -> alpha reference edge"
    );

    // Communities populated and every node assigned.
    assert!(!g.communities.is_empty());
    assert_eq!(g.meta.node_count, g.nodes.len());
}

#[tokio::test]
async fn shortest_path_and_query() {
    let tmp = tempfile::tempdir().unwrap();
    let g = build_fixture(tmp.path()).await;

    // Path from b.ts to alpha exists (via the reference edge).
    let path = query::shortest_path(&g, "file:src/b.ts", "function:src/a.ts:alpha");
    assert!(path.is_some(), "expected a path b.ts -> alpha");

    // Query by label term returns the matching entity.
    let hits = query::query(&g, "alpha", 5);
    assert!(hits.iter().any(|h| h.label == "alpha"));

    // Explain returns neighbours.
    let ex = query::explain(&g, "function:src/a.ts:alpha").unwrap();
    assert_eq!(ex.label, "alpha");
    assert!(!ex.neighbors.is_empty());
}

#[tokio::test]
async fn context_bundle_has_neighbourhood() {
    use hexa_graph::context::{context_for, render_markdown, ContextOpts};
    let tmp = tempfile::tempdir().unwrap();
    let g = build_fixture(tmp.path()).await;

    // Context for a.ts: defines `alpha`, and is used by b.ts.
    let ctx = context_for(&g, "src/a.ts", ContextOpts::default()).expect("context for a.ts");
    assert_eq!(ctx.kind, "file");
    assert!(ctx.defines.iter().any(|d| d.label == "alpha"));
    assert!(
        ctx.used_by
            .iter()
            .any(|u| u.file == "src/b.ts" && u.entity == "alpha"),
        "a.ts should be used_by b.ts via alpha"
    );
    assert!(ctx.imported_by.iter().any(|f| f == "src/b.ts"));

    // An entity target resolves to its declaring file.
    let via_entity = context_for(&g, "Beta", ContextOpts::default()).expect("context via entity");
    assert_eq!(via_entity.label, "src/b.ts");

    // Markdown render is non-empty and mentions the file.
    let md = render_markdown(&ctx);
    assert!(md.contains("src/a.ts"));
    assert!(md.contains("Defines"));
}

#[tokio::test]
async fn rank_lessons_prefers_neighbourhood_mentions() {
    use hexa_graph::context::{context_for, rank_lessons, ContextOpts};
    let tmp = tempfile::tempdir().unwrap();
    let g = build_fixture(tmp.path()).await;
    let ctx = context_for(&g, "src/a.ts", ContextOpts::default()).expect("context");

    let lessons = vec![
        (
            "lesson:alpha".into(),
            "the alpha function in src/a.ts needs care".into(),
        ),
        (
            "lesson:unrelated".into(),
            "tune the postgres connection pool size".into(),
        ),
        ("lesson:beta".into(), "Beta class consumes alpha".into()),
    ];
    let ranked = rank_lessons(&ctx, &lessons, 6);
    // The unrelated lesson (no anchor mentions) is filtered out.
    assert!(ranked.iter().all(|l| l.key != "lesson:unrelated"));
    // The lesson mentioning the file + symbol ranks first and scores highest.
    assert!(!ranked.is_empty());
    assert_eq!(ranked[0].key, "lesson:alpha");
    assert!(ranked[0].score >= ranked.last().unwrap().score);
}

#[tokio::test]
async fn build_is_deterministic() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let g1 = build_fixture(tmp1.path()).await;
    let g2 = build_fixture(tmp2.path()).await;
    // Same structure → identical community assignment and serialization.
    assert_eq!(g1.to_json().unwrap(), g2.to_json().unwrap());
}

#[tokio::test]
async fn excluded_dir_name_at_the_root_still_walks() {
    // The root itself is named `build` — one of EXCLUDE_DIRS. walkdir applies
    // filter_entry to the root, so rejecting it there ended the walk immediately
    // and produced an empty graph.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("build");
    fs::create_dir_all(&root).unwrap();
    let g = build_fixture(&root).await;

    assert!(
        g.nodes.iter().any(|n| n.file == "src/a.ts"),
        "root named `build` yielded {} nodes",
        g.nodes.len()
    );
    // A nested excluded directory is still pruned.
    write(&root, "target/gen.rs", "pub fn generated() {}\n");
    let g = build_fixture(&root).await;
    assert!(!g.nodes.iter().any(|n| n.file.starts_with("target/")));
}

#[tokio::test]
async fn a_scaffolded_root_crate_keeps_its_crate_import_edges() {
    // The layout `hexa new` scaffolds for Rust: one crate at the graph root, so
    // every path starts `src/` with no crate directory in front of it. Import
    // resolution looked for the substring `/src/`, with a leading slash, found
    // nothing, and dropped every `use crate::…` edge. `graph consumers` then
    // printed SAFE TO REMOVE for a file the build depends on — the ADR-2609122048
    // excision-oracle failure, still live for the root-crate case.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("counter-app");
    fs::create_dir_all(&root).unwrap();

    write(
        &root,
        "src/domain/user.rs",
        "pub struct User {\n    pub id: u32,\n}\n",
    );
    write(
        &root,
        "src/usecases/create_user.rs",
        "use crate::domain::user::User;\npub fn create_user(id: u32) -> User {\n    User { id }\n}\n",
    );
    write(&root, "src/lib.rs", "pub mod domain;\npub mod usecases;\n");

    let opts = BuildOpts {
        project_id: "root-crate".into(),
        mode: Mode::Ast,
        ..Default::default()
    };
    let g = build(&root, opts, &NoopSemanticExtractor).await.unwrap();

    let ctx = hexa_graph::context::context_for(
        &g,
        "src/domain/user.rs",
        hexa_graph::context::ContextOpts::default(),
    )
    .expect("context for src/domain/user.rs");

    // What `hexa graph consumers` reads to decide safe_to_remove.
    assert!(
        ctx.imported_by
            .iter()
            .any(|f| f == "src/usecases/create_user.rs"),
        "create_user.rs imports crate::domain::user, imported_by was {:?}",
        ctx.imported_by
    );
    assert!(
        ctx.used_by.iter().any(|u| u.entity == "User"),
        "create_user.rs names User, used_by was {:?}",
        ctx.used_by
    );

    // An integration test names the root crate by its ident; that reaches `src/lib.rs`.
    write(
        &root,
        "tests/counter.rs",
        "use counter_app::usecases::create_user::create_user;\n#[test]\nfn works() {\n    let _ = create_user(1);\n}\n",
    );
    let opts = BuildOpts {
        project_id: "root-crate".into(),
        mode: Mode::Ast,
        ..Default::default()
    };
    let g = build(&root, opts, &NoopSemanticExtractor).await.unwrap();
    let lib = hexa_graph::context::context_for(
        &g,
        "src/lib.rs",
        hexa_graph::context::ContextOpts::default(),
    )
    .expect("context for src/lib.rs");
    assert!(
        lib.imported_by.iter().any(|f| f == "tests/counter.rs"),
        "tests/counter.rs names the root crate, imported_by was {:?}",
        lib.imported_by
    );
}

#[tokio::test]
async fn a_python_module_imported_in_any_form_has_importers() {
    // ADR-2611211324 put `.py` files in the graph. A Python import is written
    // `pkg.mod` or `.mod`, which no Rust or TypeScript rule resolves, so every
    // Python module looked unimported and `graph consumers` printed SAFE TO
    // REMOVE for a module the program imports — the excision-oracle failure.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("pyapp");
    fs::create_dir_all(&root).unwrap();
    write(&root, "pkg/__init__.py", "");
    write(&root, "pkg/mod.py", "def f():\n    return 1\n");
    write(&root, "main.py", "import pkg.mod\nprint(pkg.mod.f())\n");
    write(&root, "a.py", "from pkg import mod\n");
    write(&root, "pkg/b.py", "from . import mod\n");
    write(&root, "pkg/c.py", "from .mod import f\n");
    write(&root, "pkg/d.py", "import os\nfrom collections import OrderedDict\n");
    write(&root, "pkg/other.py", "X = 1\n");
    write(&root, "e.py", "from pkg import other, mod\n");
    // A package whose subpackage has no `__init__.py` (a namespace package).
    write(&root, "ns/__init__.py", "");
    write(&root, "ns/sub/leaf.py", "Z = 1\n");
    write(&root, "f.py", "import ns.sub.leaf\n");

    let opts = BuildOpts { project_id: "pyapp".into(), mode: Mode::Ast, ..Default::default() };
    let g = build(&root, opts, &NoopSemanticExtractor).await.unwrap();
    let ctx = hexa_graph::context::context_for(&g, "pkg/mod.py", hexa_graph::context::ContextOpts::default())
        .expect("context for pkg/mod.py");

    let mut got: Vec<&str> = ctx.imported_by.iter().map(String::as_str).collect();
    got.sort();
    assert_eq!(got, ["a.py", "e.py", "main.py", "pkg/b.py", "pkg/c.py"], "imported_by: a `from` import of two names reaches both");
    // Importing a submodule runs its package's `__init__.py` first.
    let init = hexa_graph::context::context_for(&g, "pkg/__init__.py", hexa_graph::context::ContextOpts::default()).unwrap();
    assert!(init.imported_by.iter().any(|f| f == "main.py"), "`import pkg.mod` runs pkg/__init__.py: {:?}", init.imported_by);
    // A sibling import inside the package does not import the package itself.
    assert!(!init.imported_by.iter().any(|f| f.starts_with("pkg/")), "no loop from pkg's own modules: {:?}", init.imported_by);
    // The walk goes past a directory with no `__init__.py`.
    let ns = hexa_graph::context::context_for(&g, "ns/__init__.py", hexa_graph::context::ContextOpts::default()).unwrap();
    assert!(ns.imported_by.iter().any(|f| f == "f.py"), "`import ns.sub.leaf` runs ns/__init__.py: {:?}", ns.imported_by);
    // The standard library is not a project file: no edge from pkg/d.py.
    let d = hexa_graph::context::context_for(&g, "pkg/d.py", hexa_graph::context::ContextOpts::default()).unwrap();
    assert!(d.imports.is_empty(), "stdlib imports are not edges: {:?}", d.imports);
}
