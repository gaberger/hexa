//! A grade is a claim about code that was read. No code read, no grade.
//!
//! `hexa analyze /path-that-does-not-exist` printed **A+ — score 100/100** and
//! exited 0. So did `--grade A`, `--strict` and `--exit-code`. An empty but
//! existing directory did the same. `canonicalize()` fell back to the literal
//! path on failure, nothing was scanned, and a score computed from zero
//! findings is a perfect one.
//!
//! This is the failure this project names as the worst kind: a gate that
//! degrades silently is worse than no gate, and a tool that reports "nothing
//! found" must prove it looked (ADR-2609122048). A CI job running
//! `hexa analyze . --grade A` from the wrong working directory, or after a
//! checkout that produced nothing, passed with full marks.

use std::path::Path;
use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(hexa()).args(args).output().expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// A path that cannot exist, so the test does not depend on the filesystem.
const MISSING: &str = "/nonexistent-path-hexa-gate-xyz";

// ── 1. A path that does not exist ────────────────────────────────────────────

#[test]
fn a_path_that_does_not_exist_is_an_error() {
    let (code, out) = run(&["analyze", MISSING]);
    assert_eq!(code, 1, "analysing nothing is not success:\n{out}");
    assert!(out.contains(MISSING), "the message names the path:\n{out}");
}

#[test]
fn a_path_that_does_not_exist_prints_no_grade() {
    let (_, out) = run(&["analyze", MISSING]);
    assert!(
        !out.contains("A+") && !out.contains("100/100"),
        "a grade for a directory that is not there is the whole bug:\n{out}"
    );
}

#[test]
fn the_grade_floor_fails_on_a_path_that_does_not_exist() {
    let (code, out) = run(&["analyze", MISSING, "--grade", "A"]);
    assert_eq!(code, 1, "--grade A passed against nothing:\n{out}");
}

#[test]
fn strict_and_exit_code_fail_on_a_path_that_does_not_exist() {
    for flag in ["--strict", "--exit-code"] {
        let (code, out) = run(&["analyze", MISSING, flag]);
        assert_eq!(code, 1, "{flag} passed against nothing:\n{out}");
    }
}

#[test]
fn json_mode_fails_on_a_path_that_does_not_exist() {
    let (code, out) = run(&["analyze", MISSING, "--json"]);
    assert_eq!(code, 1, "--json reported a clean tree that does not exist:\n{out}");
}

// ── 2. A directory that exists but holds no source ───────────────────────────

fn empty_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn a_directory_with_no_source_prints_no_grade() {
    let d = empty_dir();
    let (_, out) = run(&["analyze", d.path().to_str().unwrap()]);
    assert!(
        !out.contains("A+") && !out.contains("100/100"),
        "zero files scanned cannot produce a perfect score:\n{out}"
    );
}

#[test]
fn a_directory_with_no_source_is_an_error() {
    let d = empty_dir();
    let (code, out) = run(&["analyze", d.path().to_str().unwrap()]);
    assert_eq!(code, 1, "a vacuous analysis is a failed analysis:\n{out}");
    assert!(
        out.to_lowercase().contains("no source") || out.to_lowercase().contains("nothing to grade"),
        "and it says why, rather than printing an empty report:\n{out}"
    );
}

#[test]
fn the_grade_floor_fails_on_a_directory_with_no_source() {
    let d = empty_dir();
    let (code, out) = run(&["analyze", d.path().to_str().unwrap(), "--grade", "A"]);
    assert_eq!(code, 1, "--grade A passed on an empty directory:\n{out}");
}

// ── 3. Must not regress: a real tree still grades ────────────────────────────

#[test]
fn a_real_tree_still_grades() {
    // The control. Refusing an empty scan must not start refusing real ones.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let (code, out) = run(&["analyze", root.to_str().unwrap()]);
    assert_eq!(code, 0, "hexa's own tree still analyses:\n{out}");
    assert!(out.contains("A+"), "and still grades A+:\n{out}");
}

#[test]
fn a_small_real_project_still_grades() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::create_dir_all(r.join("src/domain")).unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(r.join("src/domain/thing.rs"), "pub fn thing() -> u8 { 1 }\n").unwrap();
    let (code, out) = run(&["analyze", r.to_str().unwrap()]);
    assert_eq!(code, 0, "one real source file is enough to analyse:\n{out}");
    assert!(out.contains("score"), "and a score is printed:\n{out}");
}
