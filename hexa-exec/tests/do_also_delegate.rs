//! The frontier delegate (`claude -p`) is told which files beside its target it
//! may create, the run commits exactly those, and a failed run takes them back.
//!
//! The prompt said "Edit ONLY the file `<file>`", so a task that needed a new
//! module or a new test file was told not to make it; and on a failure only
//! the target was reverted. Written before the fix. The frontier is a fake.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::direct_exec::DirectTask;
use hexa_exec::direct_react::claude_attempts;
use hexa_exec::ports::{Frontier, FrontierError};

/// Writes `writes` into the working tree, records the prompt, then answers
/// (or times out).
struct FakeDelegate {
    writes: Vec<(&'static str, &'static str)>,
    time_out: bool,
    prompt: Mutex<Option<String>>,
}

impl FakeDelegate {
    fn new(writes: Vec<(&'static str, &'static str)>, time_out: bool) -> Arc<Self> {
        Arc::new(Self { writes, time_out, prompt: Mutex::new(None) })
    }
}

#[async_trait]
impl Frontier for FakeDelegate {
    async fn run(&self, prompt: &str, cwd: &Path, timeout: Duration, _s: &str) -> Result<String, FrontierError> {
        *self.prompt.lock().unwrap() = Some(prompt.to_string());
        for (rel, body) in &self.writes {
            let p = cwd.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        if self.time_out {
            return Err(FrontierError::Timeout(timeout));
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

fn deps(fake: Arc<FakeDelegate>) -> hexa_exec::direct_exec::ExecDeps {
    let mut d = hexa_exec::default_deps();
    d.frontier = fake;
    d
}

fn committed_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        git(dir, &["show", "--name-only", "--format=", "HEAD"]).lines().map(str::to_string).collect();
    v.sort();
    v
}

#[tokio::test]
async fn the_delegate_prompt_names_the_also_files() {
    let tmp = repo();
    let fake = FakeDelegate::new(vec![], false);
    let t = task("false", &["src/domain/names.rs", "tests/domain_names.rs"]);
    let _ = claude_attempts(&deps(fake.clone()), &t, tmp.path(), false).await;

    let prompt = fake.prompt.lock().unwrap().clone().expect("the delegate was asked");
    assert!(prompt.contains("`src/domain/mod.rs`"), "{prompt}");
    assert!(
        prompt.contains("you may create or edit only these additional files: `src/domain/names.rs`, `tests/domain_names.rs`"),
        "{prompt}"
    );
    assert!(!prompt.contains("Edit ONLY the file"), "the prompt must not forbid the --also files: {prompt}");
}

#[tokio::test]
async fn without_also_the_delegate_is_still_held_to_its_one_file() {
    let tmp = repo();
    let fake = FakeDelegate::new(vec![], false);
    let _ = claude_attempts(&deps(fake.clone()), &task("false", &[]), tmp.path(), false).await;
    let prompt = fake.prompt.lock().unwrap().clone().unwrap();
    assert!(prompt.contains("`src/domain/mod.rs`"), "{prompt}");
    assert!(!prompt.contains("additional files"), "{prompt}");
}

#[tokio::test]
async fn a_passing_delegate_commits_target_and_also_files_together_and_nothing_else() {
    let tmp = repo();
    let dir = tmp.path();
    let fake = FakeDelegate::new(
        vec![
            ("src/domain/mod.rs", "pub mod names;\n"),
            ("src/domain/names.rs", "pub struct SecretKey(pub String);\n"),
            ("tests/domain_names.rs", "#[test] fn t() {}\n"),
            ("src/domain/stray.rs", "// unlisted\n"),
        ],
        false,
    );
    let t = task("test -f src/domain/names.rs && test -f tests/domain_names.rs", &["src/domain/names.rs", "tests/domain_names.rs"]);
    let (r, _, _) = claude_attempts(&deps(fake), &t, dir, false).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(
        committed_files(dir),
        vec!["src/domain/mod.rs", "src/domain/names.rs", "tests/domain_names.rs"],
        "one commit reproduces the green state"
    );
    assert!(git(dir, &["status", "--porcelain", "-uall"]).contains("?? src/domain/stray.rs"), "unlisted file not swept in");
}

#[tokio::test]
async fn a_failing_delegate_restores_its_also_files() {
    let tmp = repo();
    let dir = tmp.path();
    let head = git(dir, &["rev-parse", "HEAD"]);
    let fake = FakeDelegate::new(
        vec![
            ("src/domain/mod.rs", "pub mod names;\n"),
            ("src/domain/existing.rs", "// clobbered\n"),
            ("src/domain/names.rs", "broken\n"),
        ],
        false,
    );
    let (r, _, _) =
        claude_attempts(&deps(fake), &task("false", &["src/domain/existing.rs", "src/domain/names.rs"]), dir, false).await;

    assert!(!r.ok);
    assert_eq!(git(dir, &["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(std::fs::read_to_string(dir.join("src/domain/mod.rs")).unwrap(), "// domain\n");
    assert_eq!(std::fs::read_to_string(dir.join("src/domain/existing.rs")).unwrap(), "// committed\n");
    assert!(!dir.join("src/domain/names.rs").exists(), "a file the failed run created is removed");
}

#[tokio::test]
async fn a_timed_out_delegate_restores_its_also_files() {
    let tmp = repo();
    let dir = tmp.path();
    let fake = FakeDelegate::new(
        vec![("src/domain/existing.rs", "// half-written\n"), ("tests/domain_names.rs", "half\n")],
        true,
    );
    let (r, _, _) =
        claude_attempts(&deps(fake), &task("true", &["src/domain/existing.rs", "tests/domain_names.rs"]), dir, false).await;

    assert!(!r.ok);
    assert!(r.error.unwrap_or_default().contains("timed out"));
    assert_eq!(std::fs::read_to_string(dir.join("src/domain/existing.rs")).unwrap(), "// committed\n");
    assert!(!dir.join("tests/domain_names.rs").exists());
}

#[tokio::test]
async fn a_delegate_given_an_escaping_also_path_is_refused_before_it_runs() {
    let tmp = repo();
    let fake = FakeDelegate::new(vec![], false);
    let (r, _, _) = claude_attempts(&deps(fake.clone()), &task("true", &["../outside.rs"]), tmp.path(), false).await;
    assert!(!r.ok);
    assert!(fake.prompt.lock().unwrap().is_none(), "the delegate must not be started");
}
