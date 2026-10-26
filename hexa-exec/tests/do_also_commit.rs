//! `hexa do run --also <PATH>`: the files a run may create or edit beside its
//! target are committed WITH it, and nothing else is.
//!
//! The bug: `commit()` staged the target plus the files that were already
//! dirty when the run started. A file the run CREATED — a new module declared
//! from the target, a new test file — was in no pathspec, so it was left
//! untracked and swept into some later run's commit. No commit reproduced the
//! green state the evidence had seen. Written before the fix.

use std::path::Path;
use std::process::Command;

use hexa_exec::direct_exec::{commit_with_also, validate_also_paths, AlsoSnapshot, DirectTask};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::create_dir_all(dir.join("src/domain")).unwrap();
    std::fs::write(dir.join("src/domain/mod.rs"), "// domain\n").unwrap();
    std::fs::write(dir.join("src/domain/existing.rs"), "// committed\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "init"]);
    tmp
}

fn committed_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        git(dir, &["show", "--name-only", "--format=", "HEAD"]).lines().map(str::to_string).collect();
    v.sort();
    v
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|p| p.to_string()).collect()
}

#[test]
fn a_task_names_no_also_files_by_default() {
    let t = DirectTask { instruction: "x".into(), file: "a.rs".into(), evidence: "true".into(), ..Default::default() };
    assert!(t.also.is_empty());
    let parsed: DirectTask =
        serde_json::from_value(serde_json::json!({ "instruction": "x", "file": "a.rs", "evidence": "true" })).unwrap();
    assert!(parsed.also.is_empty(), "an absent `also` deserializes to none");
}

#[tokio::test]
async fn a_new_file_named_with_also_is_in_the_runs_commit() {
    let tmp = repo();
    let dir = tmp.path();
    std::fs::write(dir.join("src/domain/mod.rs"), "pub mod names;\n").unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("src/domain/names.rs"), "pub struct SecretKey(pub String);\n").unwrap();
    std::fs::write(dir.join("tests/domain_names.rs"), "#[test] fn t() {}\n").unwrap();

    commit_with_also(
        dir,
        "src/domain/mod.rs",
        "add names",
        false,
        &[],
        &s(&["src/domain/names.rs", "tests/domain_names.rs"]),
    )
    .await
    .unwrap();

    assert_eq!(
        committed_files(dir),
        s(&["src/domain/mod.rs", "src/domain/names.rs", "tests/domain_names.rs"]),
        "the target and every --also file land in ONE commit"
    );
    assert!(git(dir, &["status", "--porcelain", "-uall"]).trim().is_empty(), "nothing is left behind");
}

#[tokio::test]
async fn a_new_file_not_named_with_also_is_not_committed() {
    let tmp = repo();
    let dir = tmp.path();
    std::fs::write(dir.join("src/domain/mod.rs"), "pub mod names;\n").unwrap();
    std::fs::write(dir.join("src/domain/names.rs"), "pub struct SecretKey(pub String);\n").unwrap();
    std::fs::write(dir.join("src/domain/stray.rs"), "// nobody asked for this\n").unwrap();

    commit_with_also(dir, "src/domain/mod.rs", "add names", false, &[], &s(&["src/domain/names.rs"]))
        .await
        .unwrap();

    assert_eq!(committed_files(dir), s(&["src/domain/mod.rs", "src/domain/names.rs"]));
    assert!(
        git(dir, &["status", "--porcelain", "-uall"]).contains("?? src/domain/stray.rs"),
        "an unlisted new file stays untracked — the concurrency guard holds"
    );
}

#[tokio::test]
async fn an_also_file_the_run_did_not_create_is_skipped_not_an_error() {
    let tmp = repo();
    let dir = tmp.path();
    std::fs::write(dir.join("src/domain/mod.rs"), "// edited\n").unwrap();

    commit_with_also(dir, "src/domain/mod.rs", "edit", false, &[], &s(&["src/domain/never_made.rs"]))
        .await
        .expect("a listed path that does not exist after the run is permission, not a requirement");

    assert_eq!(committed_files(dir), s(&["src/domain/mod.rs"]));
}

#[test]
fn also_paths_must_stay_inside_the_repository() {
    for bad in ["../outside.rs", "src/../../outside.rs", "src/a/../b.rs", "/etc/passwd", "", "./"] {
        assert!(validate_also_paths(&s(&[bad])).is_err(), "`{bad}` must be refused");
    }
}

#[test]
fn also_paths_may_not_reach_into_git() {
    for bad in [".git", ".git/config", ".git/hooks/pre-commit", "sub/.git/config"] {
        let err = validate_also_paths(&s(&[bad])).expect_err(bad);
        assert!(err.contains(".git"), "the refusal names why: {err}");
    }
}

#[test]
fn ordinary_repo_relative_paths_are_accepted() {
    validate_also_paths(&s(&["src/domain/names.rs", "tests/domain_names.rs", "./README.md"])).unwrap();
    validate_also_paths(&[]).unwrap();
}

#[test]
fn a_snapshot_restores_edited_also_files_and_deletes_created_ones() {
    let tmp = repo();
    let dir = tmp.path();
    // A pre-run uncommitted edit is the state to restore, not HEAD.
    std::fs::write(dir.join("src/domain/existing.rs"), "// operator's pre-run edit\n").unwrap();
    let also = s(&["src/domain/existing.rs", "src/domain/new.rs", "tests/new_dir/t.rs"]);

    let snap = AlsoSnapshot::capture(dir, &also).unwrap();
    std::fs::write(dir.join("src/domain/existing.rs"), "// the failed run's edit\n").unwrap();
    std::fs::write(dir.join("src/domain/new.rs"), "// the failed run's new file\n").unwrap();
    std::fs::create_dir_all(dir.join("tests/new_dir")).unwrap();
    std::fs::write(dir.join("tests/new_dir/t.rs"), "// another\n").unwrap();
    snap.restore();

    assert_eq!(std::fs::read_to_string(dir.join("src/domain/existing.rs")).unwrap(), "// operator's pre-run edit\n");
    assert!(!dir.join("src/domain/new.rs").exists(), "a file that did not exist before the run is deleted");
    assert!(!dir.join("tests/new_dir/t.rs").exists());
}

#[test]
fn a_snapshot_refuses_paths_outside_the_repository() {
    let tmp = repo();
    assert!(AlsoSnapshot::capture(tmp.path(), &s(&["../escape.rs"])).is_err());
    assert!(AlsoSnapshot::capture(tmp.path(), &s(&[".git/config"])).is_err());
}
