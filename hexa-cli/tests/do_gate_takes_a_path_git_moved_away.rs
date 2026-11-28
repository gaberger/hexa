//! `hexa do gate --files` takes a path that a `git mv` or `git rm` moved away.
//!
//! A rename is two paths: the new one, and the old one that is gone from the
//! tree and, once staged, gone from the index too. The gate refused the old
//! path as "no such file in the tree or in git", so a rename committed the new
//! copy and left the old one in HEAD. A path HEAD holds is a deletion to commit.

use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn gate(root: &Path, files: &[&str]) -> (i32, String) {
    let home = root.join("..").join("home");
    std::fs::create_dir_all(&home).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["do", "gate", "--dry-run", "--files"])
        .args(files)
        .args(["-e", "bash ci/green.sh", "-m", "a rename"])
        .current_dir(root)
        .env("HOME", &home)
        .output()
        .unwrap();
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().join("repo");
    std::fs::create_dir_all(r.join("src")).unwrap();
    std::fs::create_dir_all(r.join("ci")).unwrap();
    std::fs::write(r.join("src/old.txt"), "one\n").unwrap();
    std::fs::write(r.join("ci/green.sh"), "echo 'test result: ok. 1 passed; 0 failed'\n").unwrap();
    git(&r, &["init", "-q", "-b", "main"]);
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-qm", "init"]);
    d
}

#[test]
fn a_renamed_file_names_both_paths() {
    let d = repo();
    let r = d.path().join("repo");
    git(&r, &["mv", "src/old.txt", "src/new.txt"]);
    let (code, out) = gate(&r, &["src/old.txt", "src/new.txt"]);
    assert!(!out.contains("no such file"), "the moved-away path was refused:\n{out}");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("src/old.txt") && out.contains("src/new.txt"), "both paths are in the commit:\n{out}");
}

#[test]
fn the_gate_commits_the_rename_whole() {
    // Not a dry run: the old path must leave HEAD, not just be accepted.
    let d = repo();
    let r = d.path().join("repo");
    git(&r, &["mv", "src/old.txt", "src/new.txt"]);
    let home = d.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["do", "gate", "--files", "src/old.txt", "src/new.txt", "-e", "bash ci/green.sh", "-m", "rename old to new", "--no-land"])
        .current_dir(&r)
        .env("HOME", &home)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    let out = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "{out}");
    let tree = Command::new("git").args(["ls-tree", "-r", "--name-only", "HEAD"]).current_dir(&r).output().unwrap();
    let tree = String::from_utf8_lossy(&tree.stdout).to_string();
    assert!(tree.contains("src/new.txt"), "the new copy is committed:\n{tree}");
    assert!(!tree.contains("src/old.txt"), "the old copy is gone from HEAD:\n{tree}");
}

#[test]
fn a_path_that_never_existed_is_still_refused() {
    // Control: the refusal this file is not about still holds.
    let d = repo();
    let r = d.path().join("repo");
    let (code, out) = gate(&r, &["src/never-was.txt"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("no such file"), "{out}");
}
