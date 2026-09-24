//! A project whose code is not laid out in `domain/`, `ports/`, `adapters/`
//! folders declares its layers in `.hexa/project.json`, and the grade reads
//! that declaration.
//!
//! Before this, a file the built-in patterns did not recognise was Unknown,
//! and an edge touching an Unknown file was never checked. hexa graded
//! itself A+ with 118 of its 144 Rust files unclassified: 0 violations over
//! roughly a fifth of its own code.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// `src/engine/` is pure logic that reaches straight into `src/store/`, which
/// talks to the disk. Neither folder name means anything to the built-in
/// patterns.
fn fixture(layers: Option<&str>) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(r, "src/lib.rs", "pub mod engine;\npub mod store;\n");
    write(r, "src/engine/mod.rs", "pub mod core;\npub mod io;\n");
    write(r, "src/engine/core.rs", "use crate::store::disk::Disk;\npub struct Engine { pub d: Disk }\n");
    write(r, "src/engine/io.rs", "pub struct Io;\n");
    write(r, "src/store/mod.rs", "pub mod disk;\n");
    write(r, "src/store/disk.rs", "pub struct Disk;\n");
    if let Some(l) = layers {
        write(r, ".hexa/project.json", &format!("{{\"analyze\":{{\"layers\":{l}}}}}"));
    }
    d
}

fn analyze(root: &Path) -> (Option<i32>, serde_json::Value, String) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let v = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code(), v, format!("{stdout}{stderr}"))
}

fn layer_of(v: &serde_json::Value, lang: &str, layer: &str) -> u64 {
    v["layer_inventory"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["language"] == lang && r["layer"] == layer)
        .map(|r| r["files"].as_u64().unwrap_or(0))
        .sum()
}

const DECLARED: &str = r#"{"src/engine": "domain", "src/store": "adapters/secondary"}"#;

#[test]
fn undeclared_the_edge_is_not_checked() {
    // The control: the same code, no declaration, no finding — this is the
    // blind spot the declaration closes.
    let d = fixture(None);
    let (_, v, all) = analyze(d.path());
    let n = v["boundary_violations"].as_array().map_or(0, |a| a.len());
    assert_eq!(n, 0, "{all}");
}

#[test]
fn a_declared_domain_reaching_an_adapter_is_a_violation() {
    let d = fixture(Some(DECLARED));
    let (_, v, all) = analyze(d.path());
    let hits: Vec<_> = v["boundary_violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| x["from_file"] == "src/engine/core.rs")
        .collect();
    assert_eq!(hits.len(), 1, "domain → secondary adapter must be reported:\n{all}");
    assert!(v["score"].as_u64().unwrap_or(100) < 100, "and it must cost grade:\n{all}");
}

#[test]
fn the_inventory_reads_the_same_declaration() {
    let d = fixture(Some(DECLARED));
    let (_, v, all) = analyze(d.path());
    // engine/{mod,core,io}.rs and store/{mod,disk}.rs.
    assert_eq!(layer_of(&v, "rust", "domain"), 3, "{all}");
    assert_eq!(layer_of(&v, "rust", "adapters/secondary"), 2, "{all}");
}

#[test]
fn the_longest_declared_prefix_wins() {
    let d = fixture(Some(
        r#"{"src/engine": "domain", "src/engine/io.rs": "adapters/secondary", "src/store": "adapters/secondary"}"#,
    ));
    let (_, v, all) = analyze(d.path());
    assert_eq!(layer_of(&v, "rust", "domain"), 2, "{all}");
    assert_eq!(layer_of(&v, "rust", "adapters/secondary"), 3, "{all}");
}

#[test]
fn a_prefix_matches_whole_path_segments_only() {
    // `src/eng` must not claim `src/engine/…`.
    let d = fixture(Some(r#"{"src/eng": "domain"}"#));
    let (_, v, all) = analyze(d.path());
    assert_eq!(layer_of(&v, "rust", "domain"), 0, "{all}");
}

#[test]
fn a_misspelt_layer_is_an_error_not_a_skip() {
    // A declaration that is silently dropped is a gate that degrades
    // silently: the grade would read as covering code it does not check.
    let d = fixture(Some(r#"{"src/engine": "domian"}"#));
    let (code, _, all) = analyze(d.path());
    assert_ne!(code, Some(0), "analyze must fail:\n{all}");
    assert!(all.contains("domian"), "and name the bad entry:\n{all}");
}

#[test]
fn every_file_in_hexa_itself_has_a_layer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let (_, v, all) = analyze(root);
    let unknown: u64 = ["rust", "go", "typescript"].iter().map(|l| layer_of(&v, l, "unknown")).sum();
    let total: u64 = v["layer_inventory"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| r["files"].as_u64().unwrap_or(0))
        .sum();
    assert!(total > 100, "the inventory did not read hexa: {all}");
    assert_eq!(unknown, 0, "unlabelled files in hexa's own tree:\n{}", v["layer_inventory"]);
}
