//! A scenario's test was red before the code (ADR-2610221540 §5, §8 `--prove`).
//!
//! "Written first and shown failing" was a sentence in most ADRs here, and
//! nothing checked it. A test written after the code, from the code, passes
//! the moment it exists. It mirrors the implementation instead of the
//! decision. `hexa spec verify --prove` checks out each scenario's `red`
//! commit in a scratch worktree and requires the test to exist and fail there.
//!
//! Ids are assembled at run time: the citation checker scans source.

use std::path::Path;
use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(hexa())
        .args(args)
        .current_dir(root)
        .env_remove("HEXA_NO_SKIP")
        .output()
        .expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn git(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn id() -> String {
    format!("ADR-{}", "2701010000")
}

/// A crate whose rule is `double`, and a test of it. `implemented` decides
/// whether `double` is right yet.
fn write_crate(root: &Path, implemented: bool) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    let body = if implemented { "n * 2" } else { "n" };
    std::fs::write(root.join("src/lib.rs"), format!("pub fn double(n: u32) -> u32 {{ {body} }}\n")).unwrap();
    std::fs::write(root.join("tests/rule.rs"), "#[test]\nfn doubles() { assert_eq!(fx::double(2), 4); }\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
}

fn write_adr(root: &Path) {
    let f = id();
    std::fs::create_dir_all(root.join("docs/adrs")).unwrap();
    std::fs::write(
        root.join("docs/adrs").join(format!("{f}-a-decision.md")),
        format!(
            "---\nid: {f}\nstatus: accepted\ndate: 2027-01-01\n---\n# {f}: A decision\n\n\
             **Status:** Accepted\n\n## Decision\n\n1. **It doubles.**\n\n## References\n\nnone\n"
        ),
    )
    .unwrap();
}

/// The spec, with each scenario's `red` commit, accepted and sealed.
fn spec(root: &Path, red: &str) {
    std::fs::create_dir_all(root.join("docs/specs")).unwrap();
    let s = |sid: &str, kind: &str| {
        format!(
            "{{\"id\":\"{sid}\",\"decision\":[\"§1\"],\"kind\":\"{kind}\",\"description\":\"d\",\
             \"given\":\"g\",\"when\":\"w\",\"then\":\"t\",\"test\":\"tests/rule.rs::doubles\",\"red\":\"{red}\"}}"
        )
    };
    std::fs::write(
        root.join("docs/specs").join(format!("{}.json", id())),
        format!(
            "{{\"adr\":\"{}\",\"feature\":\"f\",\"specs\":[{},{}]}}",
            id(),
            s("S1", "example"),
            s("S2", "negative")
        ),
    )
    .unwrap();
    for args in [vec!["spec", "accept", &id(), "--by", "a reviewer"][..].to_vec(), vec!["spec", "seal", &id()]] {
        let (code, out) = run(root, &args);
        assert_eq!(code, 0, "{args:?}:\n{out}");
    }
}

#[test]
fn a_test_red_before_the_code_proves() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write_adr(r);
    write_crate(r, false);
    git(r, &["init", "-q"]);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "test(rule): it doubles (failing)"]);
    let red = git(r, &["rev-parse", "--short", "HEAD"]);
    write_crate(r, true);
    spec(r, &red);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "feat(rule): it doubles"]);

    let (code, out) = run(r, &["spec", "verify", "--prove", &id()]);
    assert_eq!(code, 0, "red at its commit, green now (control):\n{out}");
    assert!(out.contains("red at"), "the proof is reported:\n{out}");
}

#[test]
fn a_test_green_at_its_red_commit_is_a_mirror() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write_adr(r);
    write_crate(r, true);
    git(r, &["init", "-q"]);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "the code and its test, together"]);
    let red = git(r, &["rev-parse", "--short", "HEAD"]);
    spec(r, &red);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "the spec"]);

    let (code, out) = run(r, &["spec", "verify", "--prove", &id()]);
    assert_ne!(code, 0, "a test that passes at its red commit was not written first:\n{out}");
    assert!(out.contains("mirror"), "{out}");
}

#[test]
fn a_test_absent_at_its_red_commit_proves_nothing() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write_adr(r);
    std::fs::write(r.join("README"), "before anything\n").unwrap();
    git(r, &["init", "-q"]);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "before the test existed"]);
    let red = git(r, &["rev-parse", "--short", "HEAD"]);
    write_crate(r, true);
    spec(r, &red);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "test and code"]);

    let (code, out) = run(r, &["spec", "verify", "--prove", &id()]);
    assert_ne!(code, 0, "a red commit where the test does not exist is not a red test:\n{out}");
    assert!(out.contains("not at"), "{out}");
}

#[test]
fn a_tested_scenario_with_no_red_commit_is_unproven_under_prove() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write_adr(r);
    write_crate(r, true);
    git(r, &["init", "-q"]);
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "init"]);
    spec(r, "");
    let p = r.join("docs/specs").join(format!("{}.json", id()));
    let text = std::fs::read_to_string(&p).unwrap().replace("\"red\": \"\",", "").replace(",\n      \"red\": \"\"", "");
    std::fs::write(&p, text).unwrap();

    let (code, out) = run(r, &["spec", "verify", "--prove", &id()]);
    assert_ne!(code, 0, "--prove asks every tested scenario to show its red commit:\n{out}");
    assert!(out.contains("no red commit"), "{out}");
}
