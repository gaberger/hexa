//! A Rust file reaches another module in more ways than a top-level `use`,
//! and the grade sees all of them.
//!
//! Three were invisible, and hexa's own tree hid crossings behind each:
//!
//! 1. `super::x` resolved to the literal path `super/x`, which is in no layer.
//!    It only ever matched when the text happened to contain `/domain/`.
//! 2. An inline path — `crate::store::save(..)`, `hexa_exec::local_store::x()`
//!    — with no `use` line was never an edge.
//! 3. A `use` inside a function body was never read; only top-level ones were.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn violations(root: &Path) -> Vec<(String, u64)> {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}:\n{text}"));
    v["boundary_violations"]
        .as_array()
        .unwrap_or_else(|| panic!("{text}"))
        .iter()
        .map(|x| (x["from_file"].as_str().unwrap_or("").to_string(), x["line"].as_u64().unwrap_or(0)))
        .collect()
}

/// A clean crate; `order.rs` in the domain is the file each case edits.
fn project(order_rs: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(r, "src/lib.rs", "pub mod domain;\npub mod ports;\npub mod adapters;\n");
    write(r, "src/domain/mod.rs", "pub mod order;\n");
    write(r, "src/domain/order.rs", order_rs);
    write(r, "src/ports/mod.rs", "pub trait StorePort {}\n");
    write(r, "src/adapters/mod.rs", "pub mod secondary;\n");
    write(r, "src/adapters/secondary/mod.rs", "pub mod db;\n");
    write(r, "src/adapters/secondary/db.rs", "use crate::ports::StorePort;\npub struct Db;\nimpl StorePort for Db {}\npub fn save() {}\n");
    d
}

fn in_order(v: &[(String, u64)]) -> Vec<u64> {
    v.iter().filter(|(f, _)| f == "src/domain/order.rs").map(|(_, l)| *l).collect()
}

#[test]
fn a_super_path_into_an_adapter_is_a_violation() {
    // The target is declared, not named for its layer, so only resolving
    // `super::super::store` to `src/store` finds it. (Aimed at the literal
    // `super/super/adapters/secondary/…`, the old resolution matched by
    // substring and passed this without resolving anything.)
    let d = project("use super::super::store::Disk;\npub struct Order { pub d: Disk }\n");
    write(d.path(), "src/store.rs", "pub struct Disk;\n");
    write(d.path(), ".hexa/project.json", r#"{"analyze":{"layers":{"src/store.rs":"adapters/secondary"}}}"#);
    assert_eq!(in_order(&violations(d.path())), [1]);
}

#[test]
fn an_inline_path_into_an_adapter_is_a_violation() {
    let d = project("pub struct Order;\npub fn place() {\n    crate::adapters::secondary::db::save();\n}\n");
    assert_eq!(in_order(&violations(d.path())), [3]);
}

#[test]
fn a_use_inside_a_function_is_read() {
    let d = project("pub struct Order;\npub fn place() {\n    use crate::adapters::secondary::db::save;\n    save();\n}\n");
    assert_eq!(in_order(&violations(d.path())), [3]);
}

#[test]
fn one_path_used_twice_is_one_finding() {
    // Ten points a violation: the same crossing on two lines is one fact.
    let d = project(
        "pub fn a() { crate::adapters::secondary::db::save(); }\npub fn b() { crate::adapters::secondary::db::save(); }\n",
    );
    assert_eq!(in_order(&violations(d.path())).len(), 1);
}

#[test]
fn a_crossing_inside_test_code_is_not_a_violation() {
    // The control: a test double is what ports and adapters exist to allow.
    let d = project(
        "pub struct Order;\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        crate::adapters::secondary::db::save();\n        use super::super::super::adapters::secondary::db::Db;\n        let _ = Db;\n    }\n}\n",
    );
    assert!(in_order(&violations(d.path())).is_empty());
}

#[test]
fn type_paths_and_local_paths_are_not_violations() {
    // The control: `Vec::new`, `Self::new`, `Ordering::Less` and a path into
    // the domain itself are not crossings.
    let d = project(
        "use std::cmp::Ordering;\npub struct Order { pub v: Vec<u8> }\nimpl Order {\n    pub fn new() -> Self { Self { v: Vec::new() } }\n    pub fn cmp_(&self) -> Ordering { Ordering::Less }\n    pub fn me() -> Self { super::order::Order::new() }\n}\n",
    );
    assert!(violations(d.path()).is_empty(), "{:?}", violations(d.path()));
}
