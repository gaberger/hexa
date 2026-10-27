//! A run restores the files it was not given (parley K17).
//!
//! A `claude -p` delegate ran `cargo fmt` over a whole crate, and the run left
//! 29 files modified that it was never given: committed only its target, and
//! left the rest dirty in the tree. Written before the fix. The frontier is a
//! fake.
//!
//! - In an isolated run (`factory`), only the run writes the tree, so every file
//!   that was clean at the start, is not `--file` or `--also`, and is changed
//!   now is the run's: it is restored *before* the gate, so the gate judges
//!   exactly what is committed.
//! - On the operator's tree, such a file may be the operator's own concurrent
//!   edit, so it is left as it is, and the result names it.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::direct_exec::DirectTask;
use hexa_exec::direct_react::claude_attempts;
use hexa_exec::ports::{Frontier, FrontierError};

/// Writes `writes` into the working tree, then answers.
struct FakeDelegate {
    writes: Vec<(&'static str, &'static str)>,
}

#[async_trait]
impl Frontier for FakeDelegate {
    async fn run(&self, _prompt: &str, cwd: &Path, _t: Duration, _s: &str) -> Result<String, FrontierError> {
        for (rel, body) in &self.writes {
            let p = cwd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        Ok("done".to_string())
    }
}

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
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "init"]);
    tmp
}

fn task(evidence: &str, also: &[&str]) -> DirectTask {
    DirectTask {
        instruction: "add pub mod names".into(),
        file: "src/domain/mod.rs".into(),
        evidence: evidence.into(),
        isolate: Some(false),
        also: also.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn deps(writes: Vec<(&'static str, &'static str)>) -> hexa_exec::direct_exec::ExecDeps {
    let mut d = hexa_exec::default_deps();
    d.frontier = Arc::new(FakeDelegate { writes });
    d
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

/// `cargo fmt` over the crate: the target plus two files it was not given,
/// and one new untracked file.
fn formatter() -> Vec<(&'static str, &'static str)> {
    vec![
        ("src/domain/mod.rs", "pub mod names;\n"),
        ("src/domain/existing.rs", "// committed, reformatted\n"),
        ("src/lib.rs", "// lib, reformatted\n"),
        ("src/domain/scratch.rs", "// new, unlisted\n"),
    ]
}

#[tokio::test]
async fn an_isolated_run_restores_every_file_it_was_not_given() {
    let tmp = repo();
    let dir = tmp.path();
    let (r, _, _) = claude_attempts(&deps(formatter()), &task("true", &[]), dir, true).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(git(dir, &["show", "--name-only", "--format=", "HEAD"]).trim(), "src/domain/mod.rs");
    assert_eq!(read(dir, "src/domain/existing.rs"), "// committed\n");
    assert_eq!(read(dir, "src/lib.rs"), "// lib\n");
    assert!(!dir.join("src/domain/scratch.rs").exists(), "a new unlisted file is removed");
    assert_eq!(git(dir, &["status", "--porcelain", "-uall"]), "", "the tree is clean");
    assert!(r.left_modified.is_empty(), "{:?}", r.left_modified);
}

#[tokio::test]
async fn an_isolated_run_restores_before_the_gate_so_a_needed_stray_fails_it_by_name() {
    let tmp = repo();
    let dir = tmp.path();
    // The gate passes only if the run's edit to a file it was not given stays.
    let t = task("grep -q reformatted src/lib.rs", &[]);
    let (r, _, _) = claude_attempts(&deps(formatter()), &t, dir, true).await;

    assert!(!r.ok, "a gate that needed an unlisted file must not pass");
    let err = r.error.clone().unwrap_or_default();
    assert!(err.contains("src/lib.rs"), "the error names the restored file: {err}");
    assert!(err.contains("--also"), "the error says how to give it: {err}");
    assert_eq!(git(dir, &["rev-list", "--count", "HEAD"]).trim(), "1", "nothing committed");
    assert_eq!(git(dir, &["status", "--porcelain", "-uall"]), "", "the tree is clean");
}

#[tokio::test]
async fn an_isolated_run_keeps_its_also_files_and_pre_run_edits() {
    let tmp = repo();
    let dir = tmp.path();
    // Dirty before the run: not the run's to restore.
    std::fs::write(dir.join("src/lib.rs"), "// lib, operator wip\n").unwrap();
    let writes = vec![
        ("src/domain/mod.rs", "pub mod names;\n"),
        ("src/domain/names.rs", "pub struct Name;\n"),
        ("src/domain/existing.rs", "// committed, reformatted\n"),
    ];
    let t = task("test -f src/domain/names.rs", &["src/domain/names.rs"]);
    let (r, _, _) = claude_attempts(&deps(writes), &t, dir, true).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(read(dir, "src/domain/names.rs"), "pub struct Name;\n");
    assert_eq!(read(dir, "src/domain/existing.rs"), "// committed\n");
    assert_eq!(read(dir, "src/lib.rs"), "// lib, operator wip\n");
}

#[tokio::test]
async fn on_the_operator_tree_a_run_leaves_unlisted_changes_and_names_them() {
    let tmp = repo();
    let dir = tmp.path();
    let (r, _, _) = claude_attempts(&deps(formatter()), &task("true", &[]), dir, false).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(git(dir, &["show", "--name-only", "--format=", "HEAD"]).trim(), "src/domain/mod.rs");
    assert_eq!(read(dir, "src/domain/existing.rs"), "// committed, reformatted\n", "never touched");
    let mut left = r.left_modified.clone();
    left.sort();
    assert_eq!(left, vec!["src/domain/existing.rs", "src/domain/scratch.rs", "src/lib.rs"]);
}
