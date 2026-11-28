//! `hexa spec seal` over a test file no commit holds says so.
//!
//! A seal over an uncommitted file exits 0 and prints "sealed", and `hexa spec
//! verify --ci` then fails the same seal as unanchored: history cannot anchor
//! a file it does not hold. The verb that created the state names it.
//!
//! Ids are assembled at run time: the citation checker scans source.

use std::path::Path;
use std::process::Command;

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hexa")).args(args).current_dir(root).output().expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn id() -> String {
    format!("ADR-{}", "2701020000")
}

/// An accepted spec naming tests/rule.rs; the test file is committed only when asked.
fn accepted_repo(commit_the_test: bool) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let f = id();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("docs/specs")).unwrap();
    std::fs::create_dir_all(r.join("tests")).unwrap();
    std::fs::write(
        r.join("docs/adrs").join(format!("{f}-a-decision.md")),
        format!("---\nid: {f}\nstatus: accepted\ndate: 2027-01-01\n---\n# {f}: A decision\n\n**Status:** Accepted\n\n## Decision\n\n1. **It doubles.**\n\n## References\n\nnone\n"),
    )
    .unwrap();
    std::fs::write(r.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
    std::fs::write(r.join("tests/rule.rs"), "#[test]\nfn doubles() {}\n").unwrap();
    let s = |sid: &str, kind: &str| {
        format!(
            "{{\"id\":\"{sid}\",\"decision\":[\"§1\"],\"kind\":\"{kind}\",\"description\":\"d\",\"given\":\"g\",\"when\":\"w\",\"then\":\"t\",\"test\":\"tests/rule.rs::doubles\"}}"
        )
    };
    std::fs::write(
        r.join("docs/specs").join(format!("{f}.json")),
        format!("{{\"adr\":\"{f}\",\"feature\":\"f\",\"specs\":[{},{}]}}", s("S1", "example"), s("S2", "negative")),
    )
    .unwrap();
    git(r, &["init", "-q"]);
    git(r, &["add", "docs", "Cargo.toml"]);
    if commit_the_test {
        git(r, &["add", "tests"]);
    }
    git(r, &["commit", "-qm", "the spec, as written"]);
    let (code, out) = run(r, &["spec", "accept", &f, "--by", "a reviewer"]);
    assert_eq!(code, 0, "accept:\n{out}");
    d
}

#[test]
fn sealing_a_file_no_commit_holds_says_it_will_not_anchor() {
    let d = accepted_repo(false);
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_eq!(code, 0, "the seal is still made:\n{out}");
    assert!(out.contains("tests/rule.rs"), "names the file:\n{out}");
    assert!(out.contains("not committed") && out.contains("unanchored"), "says what verify will report:\n{out}");
}

#[test]
fn sealing_a_committed_file_is_silent_about_anchoring() {
    let d = accepted_repo(true);
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("not committed") && !out.contains("unanchored"), "control:\n{out}");
}
