//! A Go composite literal's bare-identifier key is a struct field name only
//! when the literal's type is a struct. For a named type that is not known to
//! be a struct (`type M map[string]int; var x = M{Key: 1}`), the key is an
//! expression and may be a use of a same-package name. The deletion oracle
//! over-reports when unsure (ADR-2609122048), so such a key is linked; the
//! keys of an inline `struct{...}{F: 1}` literal, and of a type the package
//! declares as a struct, may stay unlinked.

use std::fs;
use std::path::Path;

use hexa_graph::context::{context_for, ContextOpts};
use hexa_graph::semantic::NoopSemanticExtractor;
use hexa_graph::{build, BuildOpts};

fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

async fn users_of(dir: &Path, file: &str, entity: &str) -> Vec<String> {
    let opts = BuildOpts { project_id: "t".into(), ..Default::default() };
    let g = build(dir, opts, &NoopSemanticExtractor).await.unwrap();
    let ctx = context_for(&g, file, ContextOpts::default()).unwrap();
    ctx.used_by.iter().filter(|u| u.entity == entity).map(|u| u.file.clone()).collect()
}

fn project(dir: &Path, key_go: &str, use_go: &str) {
    write(dir, "go.mod", "module example.com/p\n\ngo 1.21\n");
    write(dir, "key.go", key_go);
    write(dir, "use.go", use_go);
}

#[tokio::test]
async fn a_key_of_a_named_map_type_literal_is_a_use() {
    let tmp = tempfile::tempdir().unwrap();
    project(
        tmp.path(),
        "package p\n\nconst Key = \"k\"\n",
        "package p\n\ntype M map[string]int\n\nvar X = M{Key: 1}\n",
    );
    let users = users_of(tmp.path(), "key.go", "Key").await;
    assert!(users.iter().any(|f| f == "use.go"), "M is a map type, so Key is an expression: {users:?}");
}

#[tokio::test]
async fn a_key_of_a_named_type_declared_in_a_third_file_is_a_use() {
    let tmp = tempfile::tempdir().unwrap();
    project(tmp.path(), "package p\n\nconst Key = \"k\"\n", "package p\n\nvar X = M{Key: 1}\n");
    write(tmp.path(), "m.go", "package p\n\ntype M map[string]int\n");
    let users = users_of(tmp.path(), "key.go", "Key").await;
    assert!(users.iter().any(|f| f == "use.go"), "{users:?}");
}

#[tokio::test]
async fn a_key_of_an_elided_named_map_element_is_a_use() {
    let tmp = tempfile::tempdir().unwrap();
    project(
        tmp.path(),
        "package p\n\nconst Key = \"k\"\n",
        "package p\n\ntype M map[string]int\n\nvar X = []M{{Key: 1}}\n",
    );
    let users = users_of(tmp.path(), "key.go", "Key").await;
    assert!(users.iter().any(|f| f == "use.go"), "{users:?}");
}

#[tokio::test]
async fn a_key_of_an_inline_struct_literal_is_not_a_use() {
    let tmp = tempfile::tempdir().unwrap();
    project(
        tmp.path(),
        "package p\n\nconst Key = \"k\"\n",
        "package p\n\nvar X = struct{ Key int }{Key: 1}\n",
    );
    let users = users_of(tmp.path(), "key.go", "Key").await;
    assert!(!users.iter().any(|f| f == "use.go"), "a field of an inline struct: {users:?}");
}

#[tokio::test]
async fn a_key_of_a_struct_declared_in_another_file_of_the_package_is_not_a_use() {
    let tmp = tempfile::tempdir().unwrap();
    project(tmp.path(), "package p\n\nconst Key = \"k\"\n", "package p\n\nvar X = T{Key: 1}\n");
    write(tmp.path(), "t.go", "package p\n\ntype T struct{ Key int }\n");
    let users = users_of(tmp.path(), "key.go", "Key").await;
    assert!(!users.iter().any(|f| f == "use.go"), "T is a struct of the package: {users:?}");
}
