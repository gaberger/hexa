//! The two history operations harden's prefix rebuild needs (ADR-2610160756,
//! Implementation): reset the branch to a commit hexa made, and cherry-pick a
//! fix onto it. Both work on the current branch, and neither may touch the
//! operator's uncommitted files (ADR-2610141733 §6 holds them back).
//!
//! Written before the code, from the ADR.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, path: &str, body: &str, subject: &str) -> String {
    std::fs::write(dir.join(path), body).unwrap();
    git(dir, &["add", "--", path]);
    git(dir, &["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-qm", subject]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A repository with `base` then fixes a, b and c, each in its own file, and
/// the operator's uncommitted work: a modified tracked file and an untracked one.
fn repo() -> (tempfile::TempDir, PathBuf, [String; 4]) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    git(&dir, &["init", "-q"]);
    let base = commit(&dir, "README.md", "readme\n", "base");
    let a = commit(&dir, "a.txt", "a\n", "fix a");
    let b = commit(&dir, "b.txt", "b\n", "fix b");
    let c = commit(&dir, "c.txt", "c\n", "fix c");
    std::fs::write(dir.join("README.md"), "readme\noperator edit\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "operator notes\n").unwrap();
    (tmp, dir, [base, a, b, c])
}

fn operator_work_is_intact(dir: &Path) {
    assert_eq!(std::fs::read_to_string(dir.join("README.md")).unwrap(), "readme\noperator edit\n");
    assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "operator notes\n");
}

#[test]
fn reset_moves_the_branch_and_keeps_the_operators_files() {
    let (_tmp, dir, [_, a, _, _]) = repo();
    let branch = git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]);

    hexa_git::history::reset_to(&dir, &a).expect("reset to a commit hexa made");

    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), a);
    assert_eq!(git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]), branch, "still on the current branch");
    assert!(dir.join("a.txt").exists());
    assert!(!dir.join("b.txt").exists() && !dir.join("c.txt").exists(), "later fixes leave the tree");
    operator_work_is_intact(&dir);
}

#[test]
fn cherry_pick_applies_a_dropped_fix_on_top_and_keeps_its_message() {
    let (_tmp, dir, [_, a, _, c]) = repo();
    hexa_git::history::reset_to(&dir, &a).unwrap();

    hexa_git::history::cherry_pick(&dir, &c).expect("c applies on a");

    assert_eq!(git(&dir, &["log", "--format=%s", "-n", "2"]), "fix c\nfix a");
    assert_eq!(git(&dir, &["rev-parse", "HEAD~1"]), a, "on top of the reset point");
    assert!(dir.join("c.txt").exists() && !dir.join("b.txt").exists());
    operator_work_is_intact(&dir);
}

#[test]
fn a_cherry_pick_is_committed_by_hexa_not_by_whoever_is_configured() {
    // CI containers have no user.name, and an operator's identity must not be
    // borrowed: hexa commits as itself (ADR-2606071323 §4).
    let (_tmp, dir, [_, a, _, c]) = repo();
    hexa_git::history::reset_to(&dir, &a).unwrap();

    hexa_git::history::cherry_pick(&dir, &c).expect("c applies on a");

    assert_eq!(git(&dir, &["log", "--format=%cn <%ce>", "-n", "1"]), "hexa-factory <factory@hexa.local>");
}

#[test]
fn a_cherry_pick_that_conflicts_fails_and_leaves_the_tree_as_it_was() {
    let (_tmp, dir, [_, a, _, _]) = repo();
    // A fix that edits a.txt, made on top of a, then a different edit to a.txt.
    git(&dir, &["stash", "-u", "-q"]);
    let edit1 = commit(&dir, "a.txt", "a\nfirst\n", "edit 1");
    hexa_git::history::reset_to(&dir, &a).unwrap();
    commit(&dir, "a.txt", "a\nsecond\n", "edit 2");
    git(&dir, &["stash", "pop", "-q"]);
    let head = git(&dir, &["rev-parse", "HEAD"]);

    let err = hexa_git::history::cherry_pick(&dir, &edit1).expect_err("the edits conflict");

    assert!(!err.is_empty(), "the error says why");
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), head, "HEAD did not move");
    assert!(!dir.join(".git/CHERRY_PICK_HEAD").exists(), "no cherry-pick is left in progress");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "a\nsecond\n");
    operator_work_is_intact(&dir);
}
