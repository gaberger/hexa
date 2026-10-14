//! `hexa harden` commits every fix that passes the gate, and counts what it
//! committed.
//!
//! Observed 2026-10-14 in weave: a pass confirmed 15 bugs, committed one of
//! them, and left the rest uncommitted in the worktree; another pass on
//! `src/domain/log-chain.ts` said "1 fixed" while its one commit changed code
//! for four findings. Both come from the fix loop deciding "did the agent
//! edit?" by comparing the SET of dirty paths before and after: once the first
//! fix has dirtied the target, every later fix to the same file leaves the set
//! unchanged and is reported as "NO EDIT", while its change stays in the tree.
//! A fix that failed the gate also stayed in the tree, so the final gate
//! failed and nothing at all was committed.
//!
//! The frontier here is a fake: it answers the hunt with three findings,
//! confirms each, and "fixes" each by appending to the target and writing a
//! regression-test file — what a real agent does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::ports::{Frontier, FrontierError};

const TARGET: &str = "src/target.txt";

/// What the fake agent does when asked to fix bug `n`.
#[derive(Clone, Copy, PartialEq)]
enum Fix {
    /// Append a line to the target and write a test file.
    Good,
    /// Append a line that breaks the gate.
    Breaks,
    /// Append a line and commit it itself, as a frontier agent may.
    CommitsItself,
}

struct FakeFrontier {
    fixes: Vec<Fix>,
    handed_out: AtomicBool,
}

impl FakeFrontier {
    fn new(fixes: Vec<Fix>) -> Arc<Self> {
        Arc::new(Self { fixes, handed_out: AtomicBool::new(false) })
    }
}

#[async_trait]
impl Frontier for FakeFrontier {
    async fn run(&self, prompt: &str, cwd: &Path, _t: Duration, _s: &str) -> Result<String, FrontierError> {
        if prompt.contains("adversarial code reviewer") {
            // One lens reports every finding; the others report none.
            if self.handed_out.swap(true, Ordering::SeqCst) {
                return Ok(r#"{"findings":[]}"#.to_string());
            }
            let findings: Vec<String> = (1..=self.fixes.len())
                .map(|n| format!(r#"{{"title":"bug-{n}","location":"{TARGET}:{n}","description":"defect {n}","lens":"correctness"}}"#))
                .collect();
            return Ok(format!(r#"{{"findings":[{}]}}"#, findings.join(",")));
        }
        if prompt.contains("SKEPTICALLY verify") {
            return Ok(r#"{"is_real":true,"reasoning":"it is"}"#.to_string());
        }
        if prompt.contains("Fix this CONFIRMED bug") {
            let n = (1..=self.fixes.len())
                .find(|n| prompt.contains(&format!("title: bug-{n}\n")))
                .expect("the fix prompt names its bug");
            let target = cwd.join(TARGET);
            let mut body = std::fs::read_to_string(&target).unwrap();
            match self.fixes[n - 1] {
                Fix::Good | Fix::CommitsItself => body.push_str(&format!("fix {n}\n")),
                Fix::Breaks => body.push_str(&format!("fix {n} BROKEN\n")),
            }
            std::fs::write(&target, body).unwrap();
            std::fs::create_dir_all(cwd.join("tests")).unwrap();
            std::fs::write(cwd.join(format!("tests/bug_{n}.txt")), format!("regression test for bug {n}\n")).unwrap();
            if self.fixes[n - 1] == Fix::CommitsItself {
                git(cwd, &["add", "-A"]);
                git(cwd, &["-c", "user.name=agent", "-c", "user.email=agent@x", "commit", "-qm", "agent commit"]);
            }
            return Ok(format!("fixed bug-{n}"));
        }
        Ok(String::new())
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    git(&dir, &["init", "-q"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join(TARGET), "original\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-qm", "init"]);
    (tmp, dir)
}

/// The gate: the target holds no line marked BROKEN.
const GATE: &str = "! grep -q BROKEN src/target.txt";

fn commits_since_init(dir: &Path) -> Vec<String> {
    git(dir, &["log", "--format=%s", "HEAD"])
        .lines()
        .filter(|l| *l != "init")
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn three_fixes_to_one_file_are_three_fixes_and_three_commits() {
    let (_tmp, dir) = repo();
    let report = hexa_exec::adversarial::run_review(
        FakeFrontier::new(vec![Fix::Good, Fix::Good, Fix::Good]),
        TARGET,
        GATE,
        &dir,
    )
    .await;

    assert_eq!(report.confirmed.len(), 3, "notes: {:?}", report.notes);
    assert_eq!(report.fixed, vec!["bug-1", "bug-2", "bug-3"], "every gated fix counts; notes: {:?}", report.notes);
    let commits = commits_since_init(&dir);
    assert_eq!(commits.len(), 3, "one commit per fix: {commits:?}");
    assert_eq!(git(&dir, &["status", "--porcelain"]), "", "nothing is left uncommitted");
    let body = std::fs::read_to_string(dir.join(TARGET)).unwrap();
    assert_eq!(body, "original\nfix 1\nfix 2\nfix 3\n");
    assert!(report.verdict_line().contains("3 fixed"), "{}", report.verdict_line());
}

#[tokio::test]
async fn a_fix_that_fails_the_gate_is_reverted_and_the_others_still_commit() {
    let (_tmp, dir) = repo();
    let report = hexa_exec::adversarial::run_review(
        FakeFrontier::new(vec![Fix::Good, Fix::Breaks, Fix::Good]),
        TARGET,
        GATE,
        &dir,
    )
    .await;

    assert_eq!(report.fixed, vec!["bug-1", "bug-3"], "notes: {:?}", report.notes);
    assert_eq!(commits_since_init(&dir).len(), 2);
    assert_eq!(git(&dir, &["status", "--porcelain"]), "", "the failed fix is not left in the tree");
    let body = std::fs::read_to_string(dir.join(TARGET)).unwrap();
    assert_eq!(body, "original\nfix 1\nfix 3\n");
    assert!(!dir.join("tests/bug_2.txt").exists(), "the failed fix's test goes with it");
    assert!(report.notes.iter().any(|n| n.contains("bug-2")), "the failure is said: {:?}", report.notes);
    assert_eq!(report.gate_line(), "PASS");
}

#[tokio::test]
async fn a_fix_the_agent_committed_itself_is_still_a_gated_fix() {
    let (_tmp, dir) = repo();
    let report = hexa_exec::adversarial::run_review(
        FakeFrontier::new(vec![Fix::CommitsItself, Fix::Good]),
        TARGET,
        GATE,
        &dir,
    )
    .await;

    assert_eq!(report.fixed, vec!["bug-1", "bug-2"], "notes: {:?}", report.notes);
    let authors = git(&dir, &["log", "--format=%an", "HEAD"]);
    assert!(!authors.contains("agent"), "the agent's own commit is folded into the gated one: {authors}");
    assert_eq!(git(&dir, &["status", "--porcelain"]), "");
}
