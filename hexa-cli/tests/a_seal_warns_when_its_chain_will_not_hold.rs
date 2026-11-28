//! `hexa spec seal` over a test file edited since it was first committed says so.
//!
//! The seal's chain starts at the blob of the commit that first added the file
//! (ADR-2611081151 §1). A hand-written test is usually edited between that
//! commit and the red commit it is bound at, and sealed at the edited text. No
//! amendment records that step, so `hexa spec verify` reports the chain broken
//! ("amendment 0 ... does not continue the chain from the anchored hash") long
//! after the seal was made. The verb that makes the seal names it then.
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
    format!("ADR-{}", "2701040000")
}

/// An accepted spec naming tests/rule.rs. The test is committed once, or committed and then edited.
fn accepted_repo(edit_after_first_commit: bool) -> tempfile::TempDir {
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
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "the test, first written"]);
    if edit_after_first_commit {
        std::fs::write(r.join("tests/rule.rs"), "#[test]\nfn doubles() { assert_eq!(2 * 2, 4); }\n").unwrap();
        git(r, &["commit", "-qam", "the test, finished"]);
    }
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
    git(r, &["add", "docs"]);
    git(r, &["commit", "-qm", "the spec, as written"]);
    let (code, out) = run(r, &["spec", "accept", &f, "--by", "a reviewer"]);
    assert_eq!(code, 0, "accept:\n{out}");
    d
}

#[test]
fn sealing_a_file_edited_since_it_was_first_committed_says_verify_will_object() {
    let d = accepted_repo(true);
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_eq!(code, 0, "the seal is still made:\n{out}");
    assert!(out.contains("tests/rule.rs"), "names the file:\n{out}");
    assert!(out.contains("edited since it was first committed"), "says why:\n{out}");
    // And it is true: verify does object to that chain.
    let (_, v) = run(d.path(), &["spec", "verify", &id()]);
    assert!(v.contains("seal rewritten") || v.contains("seal broken"), "verify agrees with the warning:\n{v}");
}

#[test]
fn sealing_a_file_committed_once_is_silent_about_it() {
    // Control: a file sealed at the text it was first committed with has nothing to warn about.
    let d = accepted_repo(false);
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("edited since it was first committed"), "control:\n{out}");
}
