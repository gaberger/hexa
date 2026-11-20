//! ADR-2610281700: `hexa do apply`, a gated change without a model. Driven
//! with the real binary in a fresh git repository, with no model configured
//! anywhere and a completion command that fails if it is ever called.
//!
//! Written before the code, from the ADR; not derived from it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::io::Write;

const BASE: &str = "one\ntwo\nthree\n";

struct Repo {
    _d: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git").args(args).current_dir(root).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

fn repo() -> Repo {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("repo");
    let home = d.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("notes.txt"), BASE).unwrap();
    std::fs::write(root.join("other.txt"), "x\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "start"]);
    Repo { _d: d, root, home }
}

impl Repo {
    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"]).trim().to_string()
    }
    fn status(&self) -> String {
        git(&self.root, &["status", "--porcelain", "--untracked-files=all"])
    }
    fn apply(&self, args: &[&str], patch: &str) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
        c.args(["do", "apply"])
            .args(args)
            .current_dir(&self.root)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &self.home)
            .env("GIT_AUTHOR_NAME", "hexa").env("GIT_AUTHOR_EMAIL", "hexa@test")
            .env("GIT_COMMITTER_NAME", "hexa").env("GIT_COMMITTER_EMAIL", "hexa@test")
            // No model anywhere; any completion would fail.
            .env("HEXA_COMPLETION_COMMAND", "/bin/false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut ch = c.spawn().unwrap();
        ch.stdin.take().unwrap().write_all(patch.as_bytes()).unwrap();
        ch.wait_with_output().unwrap()
    }
}

/// A diff changing `two` to `TWO` in notes.txt: a mid-file edit.
const MID: &str = "--- a/notes.txt\n+++ b/notes.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
/// A diff creating new.txt.
const NEW: &str = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+fresh\n";
/// A diff touching other.txt.
const OTHER: &str = "--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-x\n+y\n";

#[test]
fn a_green_apply_commits_exactly_the_change() {
    let r = repo();
    let before = r.head();
    let o = r.apply(&["Capitalise two", "--file", "notes.txt", "--evidence", "grep -qx TWO notes.txt"], MID);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::read_to_string(r.root.join("notes.txt")).unwrap(), "one\nTWO\nthree\n", "a mid-file edit, nothing appended");
    assert_ne!(r.head(), before);
    assert_eq!(git(&r.root, &["log", "-1", "--format=%s"]).trim(), "feat(apply): Capitalise two");
    assert_eq!(git(&r.root, &["show", "--name-only", "--format=", "HEAD"]).trim(), "notes.txt");
    assert_eq!(r.status(), "");
}

#[test]
fn a_red_apply_restores_the_tree_exactly() {
    let r = repo();
    let before = r.head();
    let patch = format!("{MID}{NEW}");
    let o = r.apply(&["Try it", "--file", "notes.txt", "--also", "new.txt", "--evidence", "false"], &patch);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(r.root.join("notes.txt")).unwrap(), BASE);
    assert!(!r.root.join("new.txt").exists(), "a created file survived the revert");
    assert_eq!(r.head(), before);
    assert_eq!(r.status(), "");
}

#[test]
fn a_created_also_file_is_committed_on_green() {
    let r = repo();
    let o = r.apply(&["Add new", "--file", "notes.txt", "--also", "new.txt", "--evidence", "test -f new.txt"], NEW);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(git(&r.root, &["show", "--name-only", "--format=", "HEAD"]).trim(), "new.txt");
    assert_eq!(r.status(), "");
}

#[test]
fn a_diff_touching_an_unlisted_file_is_refused_and_changes_nothing() {
    let r = repo();
    let before = r.head();
    let patch = format!("{MID}{OTHER}");
    let o = r.apply(&["Sneak", "--file", "notes.txt", "--evidence", "true"], &patch);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("other.txt"), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::read_to_string(r.root.join("notes.txt")).unwrap(), BASE);
    assert_eq!(r.head(), before);
    assert_eq!(r.status(), "");
}

#[test]
fn a_diff_that_does_not_apply_is_refused_before_any_evidence() {
    let r = repo();
    let stale = "--- a/notes.txt\n+++ b/notes.txt\n@@ -1,3 +1,3 @@\n one\n-zwei\n+TWO\n three\n";
    let o = r.apply(&["Stale", "--file", "notes.txt", "--evidence", "touch ran-evidence"], stale);
    assert_eq!(o.status.code(), Some(2));
    assert!(!r.root.join("ran-evidence").exists(), "evidence ran for a refused diff");
    assert_eq!(r.status(), "");
}

#[test]
fn a_listed_file_with_uncommitted_changes_is_refused() {
    let r = repo();
    std::fs::write(r.root.join("notes.txt"), "one\ntwo\nthree\nmine\n").unwrap();
    let o = r.apply(&["Clobber", "--file", "notes.txt", "--evidence", "true"], MID);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(r.root.join("notes.txt")).unwrap(), "one\ntwo\nthree\nmine\n", "the user's change was touched");
}

#[test]
fn the_patch_can_come_from_a_file_and_json_is_one_result() {
    let r = repo();
    let p = r.home.join("p.diff");
    std::fs::write(&p, MID).unwrap();
    let o = r.apply(&["From a file", "--file", "notes.txt", "--patch", p.to_str().unwrap(), "--evidence", "true", "--json"], "");
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "{out}");
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["commit"].as_str().unwrap(), r.head());
    assert_eq!(v["attempts"], 1);
    assert_eq!(v["code"], serde_json::Value::Null);
    // And red, as JSON.
    let o = r.apply(&["Red", "--file", "notes.txt", "--patch", p.to_str().unwrap(), "--evidence", "false", "--json"], "");
    let v: serde_json::Value = serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim()).unwrap();
    assert_eq!(v["code"], "gate_failed");
}

#[test]
fn it_is_listed_in_do_runs() {
    let r = repo();
    let o = r.apply(&["Logged", "--file", "notes.txt", "--evidence", "true"], MID);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let runs = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["do", "runs"])
        .current_dir(&r.root)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &r.home)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&runs.stdout).contains("Logged"), "{}", String::from_utf8_lossy(&runs.stdout));
}
