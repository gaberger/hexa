//! What `hexa do` may commit (issues #36 and #37).
//!
//! Written before the fix, against a real temp git repository and the real
//! `claude_attempts`; only the frontier agent is a fake.
//!
//! - #36: a repo-root manifest made the "package" prefix `""`, which every path
//!   starts with, so untracked files that were there before the run (a runner
//!   token, a draft) were committed with the target. Only `--file`, `--also`
//!   and pre-run dirty files that are TRACKED may be committed, and a credential
//!   is never committed, even when named.
//! - #37: a `--file` that did not exist before the run is removed when it fails.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::direct_exec::DirectTask;
use hexa_exec::direct_react::claude_attempts;
use hexa_exec::ports::{Frontier, FrontierError};

struct FakeDelegate {
    writes: Vec<(String, &'static str)>,
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

/// A repository with a ROOT `package.json`, the shape that made the prefix "".
fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("package.json"), "{}\n").unwrap();
    std::fs::write(dir.join("src/lib.ts"), "// lib\n").unwrap();
    std::fs::write(dir.join("src/helper.ts"), "// helper\n").unwrap();
    std::fs::write(dir.join(".mcp.json"), "{}\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "init"]);
    tmp
}

fn task(file: &str, evidence: &str, also: &[&str]) -> DirectTask {
    DirectTask {
        instruction: "change it".into(),
        file: file.into(),
        evidence: evidence.into(),
        isolate: Some(false),
        also: also.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn deps(writes: Vec<(&str, &'static str)>) -> hexa_exec::direct_exec::ExecDeps {
    let mut d = hexa_exec::default_deps();
    d.frontier = Arc::new(FakeDelegate { writes: writes.into_iter().map(|(p, b)| (p.to_string(), b)).collect() });
    d
}

fn committed(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        git(dir, &["show", "--name-only", "--format=", "HEAD"]).lines().map(str::to_string).collect();
    v.sort();
    v
}

#[tokio::test]
async fn a_root_manifest_does_not_sweep_pre_run_untracked_files_into_the_commit() {
    let tmp = repo();
    let dir = tmp.path();
    // Present before the run: a token, a draft, and the operator's tracked edit.
    std::fs::write(dir.join(".runner"), "token\n").unwrap();
    std::fs::create_dir_all(dir.join("docs/drafts")).unwrap();
    std::fs::write(dir.join("docs/drafts/a.md"), "draft\n").unwrap();
    std::fs::write(dir.join("src/helper.ts"), "// helper, operator edit\n").unwrap();

    let t = task("src/lib.ts", "true", &[]);
    let (r, _, _) = claude_attempts(&deps(vec![("src/lib.ts", "// lib v2\n")]), &t, dir, false).await;

    assert!(r.ok, "{:?}", r.error);
    // The tracked, in-package pre-run edit still rides along (supporting file) ...
    assert_eq!(committed(dir), vec!["src/helper.ts", "src/lib.ts"]);
    // ... the untracked ones do not.
    let st = git(dir, &["status", "--porcelain", "-uall"]);
    assert!(st.contains("?? .runner"), "the token must stay untracked: {st}");
    assert!(st.contains("?? docs/drafts/a.md"), "an untracked draft must stay untracked: {st}");
}

#[tokio::test]
async fn a_tracked_credential_edited_before_the_run_is_never_committed() {
    let tmp = repo();
    let dir = tmp.path();
    std::fs::write(dir.join(".mcp.json"), "{\"secret\":1}\n").unwrap();

    let t = task("src/lib.ts", "true", &[]);
    let (r, _, _) = claude_attempts(&deps(vec![("src/lib.ts", "// lib v2\n")]), &t, dir, false).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(committed(dir), vec!["src/lib.ts"]);
    assert!(git(dir, &["status", "--porcelain"]).contains(" M .mcp.json"));
}

#[tokio::test]
async fn naming_a_credential_with_also_is_refused_by_name() {
    let tmp = repo();
    let dir = tmp.path();
    let t = task("src/lib.ts", "true", &[".runner"]);
    let (r, _, _) = claude_attempts(&deps(vec![("src/lib.ts", "// lib v2\n"), (".runner", "token\n")]), &t, dir, false).await;

    assert!(!r.ok, "a run that names a credential must not commit");
    let err = r.error.clone().unwrap_or_default();
    assert!(err.contains(".runner"), "the refusal names the file: {err}");
    assert_eq!(git(dir, &["rev-list", "--count", "HEAD"]).trim(), "1", "nothing committed");
    assert!(!git(dir, &["ls-files"]).contains(".runner"), "the token is not tracked");
}

#[tokio::test]
async fn a_credential_pattern_is_refused_wherever_it_sits() {
    for name in ["config/.env.local", "certs/server.pem", "ci/deploy-token.txt"] {
        let tmp = repo();
        let dir = tmp.path();
        let t = task(name, "true", &[]);
        let (r, _, _) = claude_attempts(&deps(vec![(name, "x\n")]), &t, dir, false).await;
        assert!(!r.ok, "{name} must be refused");
        assert!(r.error.clone().unwrap_or_default().contains(name), "{name}: {:?}", r.error);
        assert_eq!(git(dir, &["rev-list", "--count", "HEAD"]).trim(), "1", "{name} was committed");
    }
}

#[tokio::test]
async fn a_new_file_is_removed_again_when_the_evidence_fails() {
    let tmp = repo();
    let dir = tmp.path();
    let t = task("src/new.ts", "false", &[]);
    let (r, _, _) = claude_attempts(&deps(vec![]), &t, dir, false).await;

    assert!(!r.ok);
    assert!(!dir.join("src/new.ts").exists(), "the file did not exist before the run and must not be left behind");
    assert_eq!(git(dir, &["status", "--porcelain", "-uall"]), "");
}

#[tokio::test]
async fn a_new_file_the_delegate_wrote_is_removed_when_the_evidence_fails() {
    let tmp = repo();
    let dir = tmp.path();
    let t = task("src/new.ts", "false", &[]);
    let (r, _, _) = claude_attempts(&deps(vec![("src/new.ts", "// made\n")]), &t, dir, false).await;

    assert!(!r.ok);
    assert!(!dir.join("src/new.ts").exists());
}

#[tokio::test]
async fn an_existing_file_is_still_restored_to_its_bytes_on_failure() {
    let tmp = repo();
    let dir = tmp.path();
    let t = task("src/lib.ts", "false", &[]);
    let (r, _, _) = claude_attempts(&deps(vec![("src/lib.ts", "// broken\n")]), &t, dir, false).await;

    assert!(!r.ok);
    assert_eq!(std::fs::read_to_string(dir.join("src/lib.ts")).unwrap(), "// lib\n");
}
