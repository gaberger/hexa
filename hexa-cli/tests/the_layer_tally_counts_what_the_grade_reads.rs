//! The "Hex layers" tally of `hexa analyze` counts the files the grade reads
//! (ADR-2611221646 §3). It walked the scan directory skipping a fixed list of
//! folders, so a file left out of the grade by `analyze.exclude`, or by a
//! built-in pattern, was still counted under its layer: on one Python
//! repository it printed "Use Cases (97 files)" when 70 files were graded.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A Rust project with three graded use-case files and four more under
/// `src/usecases/legacy/`, which `analyze.exclude` leaves out of the grade;
/// one graded secondary adapter and one under `__tests__/`, which a built-in
/// pattern leaves out.
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(r, ".hexa/project.json", "{\"analyze\":{\"exclude\":[\"src/usecases/legacy\"]}}\n");
    write(r, "src/lib.rs", "pub mod usecases;\npub mod adapters;\n");
    for name in ["a", "b", "c"] {
        write(r, &format!("src/usecases/{name}.rs"), &format!("pub fn {name}() {{}}\n"));
    }
    for name in ["old1", "old2", "old3", "old4"] {
        write(r, &format!("src/usecases/legacy/{name}.rs"), &format!("pub fn {name}() {{}}\n"));
    }
    write(r, "src/adapters/secondary/store.rs", "pub struct Store;\n");
    write(r, "src/adapters/secondary/__tests__/store_check.rs", "pub fn check() {}\n");
    d
}

fn hex_layers_block(root: &Path) -> String {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", "."])
        .current_dir(root)
        .env("HOME", home.path())
        .env("NO_COLOR", "1")
        .env("CLICOLOR_FORCE", "0")
        .output()
        .unwrap();
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let start = all.find("Hex layers:").unwrap_or_else(|| panic!("no Hex layers block in:\n{all}"));
    let block: String = all[start..].lines().take(8).collect::<Vec<_>>().join("\n");
    block
}

#[test]
fn a_file_analyze_exclude_leaves_out_is_not_in_the_tally() {
    let d = fixture();
    let block = hex_layers_block(d.path());
    assert!(
        block.contains("Use Cases (3 files)"),
        "the four files under the excluded src/usecases/legacy were counted:\n{block}"
    );
}

#[test]
fn a_file_a_built_in_pattern_leaves_out_is_not_in_the_tally() {
    let d = fixture();
    let block = hex_layers_block(d.path());
    assert!(
        block.contains("Secondary Adapters (1 files)"),
        "the adapter file under __tests__/ was counted:\n{block}"
    );
}
