//! A grade cannot exceed the share of the code it could place in a layer,
//! and A+ needs all of it (ADR-2609241707).
//!
//! hexa graded itself A+ with 118 of 144 Rust files unclassified: every
//! import touching them was skipped, and nothing said how much was skipped.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn analyze(root: &Path) -> serde_json::Value {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}:\n{text}"))
}

/// A clean four-layer crate: every file in a layer the classifier knows.
fn clean() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"clean\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(r, "src/lib.rs", "pub mod domain;\npub mod ports;\npub mod usecases;\npub mod adapters;\n");
    write(r, "src/domain/mod.rs", "pub struct Order { pub id: u64 }\n");
    write(r, "src/ports/mod.rs", "use crate::domain::Order;\npub trait StorePort { fn save(&self, o: &Order); }\n");
    write(r, "src/usecases/mod.rs", "use crate::ports::StorePort;\npub fn place(s: &dyn StorePort) { let _ = s; }\n");
    write(r, "src/adapters/mod.rs", "pub mod secondary;\n");
    write(r, "src/adapters/secondary/mod.rs", "use crate::ports::StorePort;\npub struct Mem;\nimpl StorePort for Mem { fn save(&self, _o: &crate::ports::Order) {} }\n");
    d
}

fn score(v: &serde_json::Value) -> u64 {
    v["score"].as_u64().unwrap_or_else(|| panic!("no score: {v}"))
}

fn unclassified(v: &serde_json::Value) -> Vec<String> {
    v["coverage"]["unclassified"]
        .as_array()
        .unwrap_or_else(|| panic!("no coverage.unclassified: {}", v["coverage"]))
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect()
}

#[test]
fn a_fully_classified_project_can_grade_a_plus() {
    // The control: the ceiling must not cost a project that is all in view.
    let d = clean();
    let v = analyze(d.path());
    assert!(unclassified(&v).is_empty(), "{}", v["coverage"]);
    assert!(score(&v) >= 95, "a clean, fully classified project is A+: {}", v["score_components"]);
}

#[test]
fn one_unclassified_file_withholds_a_plus_and_is_named() {
    let d = clean();
    write(d.path(), "src/helpers/text.rs", "pub fn trim(s: &str) -> &str { s.trim() }\n");
    let v = analyze(d.path());
    assert_eq!(unclassified(&v), ["src/helpers/text.rs"]);
    assert!(score(&v) <= 94, "A+ needs every file in a layer, got {}", score(&v));
}

#[test]
fn the_score_cannot_exceed_the_share_classified() {
    let d = clean();
    // Six classified files (lib.rs is the composition root); add six more
    // the classifier cannot place: half the tree.
    for n in 0..6 {
        write(d.path(), &format!("src/misc/m{n}.rs"), "pub fn f() {}\n");
    }
    let v = analyze(d.path());
    assert_eq!(unclassified(&v).len(), 6);
    assert!(score(&v) <= 50, "half the code in view caps the grade at 50, got {}", score(&v));
}

#[test]
fn declaring_the_layer_restores_the_grade() {
    let d = clean();
    write(d.path(), "src/helpers/text.rs", "pub fn trim(s: &str) -> &str { s.trim() }\n");
    write(d.path(), ".hexa/project.json", r#"{"analyze":{"layers":{"src/helpers":"domain"}}}"#);
    let v = analyze(d.path());
    assert!(unclassified(&v).is_empty(), "{}", v["coverage"]);
    assert!(score(&v) >= 95, "{}", v["score_components"]);
}

#[test]
fn a_build_script_is_not_architecture() {
    let d = clean();
    write(d.path(), "build.rs", "fn main() {}\n");
    let v = analyze(d.path());
    assert!(unclassified(&v).is_empty(), "build.rs is not a layer and not a gap: {}", v["coverage"]);
}

#[test]
fn hexa_itself_is_fully_classified() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let v = analyze(root);
    assert!(unclassified(&v).is_empty(), "{}", v["coverage"]);
    assert!(v["coverage"]["total"].as_u64().unwrap_or(0) > 100, "{}", v["coverage"]);
}
