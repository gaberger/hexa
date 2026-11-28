//! A Go file's same-package links must be real uses of the package-level name
//! (ADR-2611221643 §3). A name that only looks like one — a type from another
//! package, a struct literal's field key, a method, or a name in a different
//! package that shares the directory — is not a consumer.

use std::fs;
use std::path::Path;

use hexa_graph::context::{context_for, ContextOpts};
use hexa_graph::model::KnowledgeGraph;
use hexa_graph::semantic::NoopSemanticExtractor;
use hexa_graph::{build, BuildOpts};

fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

async fn graph_of(dir: &Path) -> KnowledgeGraph {
    let opts = BuildOpts { project_id: "t".into(), ..Default::default() };
    build(dir, opts, &NoopSemanticExtractor).await.unwrap()
}

/// The files that use `entity`, defined in `file`.
async fn users_of(dir: &Path, file: &str, entity: &str) -> Vec<String> {
    let g = graph_of(dir).await;
    let ctx = context_for(&g, file, ContextOpts::default()).unwrap();
    ctx.used_by
        .iter()
        .filter(|u| u.entity == entity)
        .map(|u| u.file.clone())
        .collect()
}

fn module(dir: &Path) {
    write(dir, "go.mod", "module example.com/p\n\ngo 1.21\n");
}

#[tokio::test]
async fn a_qualified_type_is_not_a_use_of_a_same_package_type() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "handler.go", "package p\n\ntype Handler struct{}\n");
    write(
        tmp.path(),
        "server.go",
        "package p\n\nimport \"net/http\"\n\nvar H http.Handler\n",
    );
    let users = users_of(tmp.path(), "handler.go", "Handler").await;
    assert!(!users.iter().any(|f| f == "server.go"), "http.Handler is not p.Handler: {users:?}");
}

#[tokio::test]
async fn an_unqualified_type_is_a_use() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "handler.go", "package p\n\ntype Handler struct{}\n");
    write(tmp.path(), "server.go", "package p\n\nvar H Handler\n");
    let users = users_of(tmp.path(), "handler.go", "Handler").await;
    assert!(users.iter().any(|f| f == "server.go"), "{users:?}");
}

#[tokio::test]
async fn a_struct_literal_key_is_not_a_use_of_a_sibling_name() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "name.go", "package p\n\nconst Name = 7\n");
    write(
        tmp.path(),
        "t.go",
        "package p\n\ntype T struct{ Name int }\n\nvar V = T{Name: 1}\n",
    );
    let users = users_of(tmp.path(), "name.go", "Name").await;
    assert!(!users.iter().any(|f| f == "t.go"), "a field key is not the const: {users:?}");
}

#[tokio::test]
async fn a_struct_literal_value_and_a_map_key_are_uses() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "name.go", "package p\n\nconst Name = 7\nconst Key = \"k\"\n");
    write(
        tmp.path(),
        "t.go",
        "package p\n\ntype T struct{ N int }\n\nvar V = T{N: Name}\nvar M = map[string]int{Key: 1}\n",
    );
    let name_users = users_of(tmp.path(), "name.go", "Name").await;
    assert!(name_users.iter().any(|f| f == "t.go"), "{name_users:?}");
    let key_users = users_of(tmp.path(), "name.go", "Key").await;
    assert!(key_users.iter().any(|f| f == "t.go"), "a map key is an expression: {key_users:?}");
}

#[tokio::test]
async fn a_bare_name_is_not_a_use_of_a_method() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "t.go", "package p\n\ntype T struct{}\n\nfunc (T) Close() {}\n");
    write(
        tmp.path(),
        "u.go",
        "package p\n\nfunc Use() { Close := 1; _ = Close }\n",
    );
    let users = users_of(tmp.path(), "t.go", "Close").await;
    assert!(!users.iter().any(|f| f == "u.go"), "a bare name never denotes a method: {users:?}");
}

#[tokio::test]
async fn an_external_test_package_is_not_linked_to_the_package() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "lib.go", "package p\n\nfunc Helper() int { return 1 }\n");
    write(
        tmp.path(),
        "lib_ext_test.go",
        "package p_test\n\nfunc Helper() int { return 2 }\n\nfunc use() int { return Helper() }\n",
    );
    write(
        tmp.path(),
        "gen.go",
        "//go:build ignore\n\npackage main\n\nfunc main() { Helper() }\n\nfunc Helper() {}\n",
    );
    let users = users_of(tmp.path(), "lib.go", "Helper").await;
    assert!(!users.iter().any(|f| f == "lib_ext_test.go"), "p_test is another package: {users:?}");
    assert!(!users.iter().any(|f| f == "gen.go"), "package main is another package: {users:?}");
}

#[tokio::test]
async fn an_internal_test_file_is_a_consumer() {
    let tmp = tempfile::tempdir().unwrap();
    module(tmp.path());
    write(tmp.path(), "lib.go", "package p\n\nfunc Helper() int { return 1 }\n");
    write(
        tmp.path(),
        "lib_test.go",
        "package p\n\nimport \"testing\"\n\nfunc TestHelper(t *testing.T) { _ = Helper() }\n",
    );
    let users = users_of(tmp.path(), "lib.go", "Helper").await;
    assert!(users.iter().any(|f| f == "lib_test.go"), "{users:?}");
}
