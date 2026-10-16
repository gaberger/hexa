//! hexa's own bookkeeping is not part of a fix.
//!
//! Observed 2026-10-16 hardening hexa's own adversarial.rs: 10 findings
//! confirmed, 10 fixes passed `cargo test --workspace`, 0 committed. Each was
//! "left in the tree for review" because it "touched files that had
//! uncommitted changes before the pass: .hexa/loop.json". No fix touched it:
//! `hexa harden`'s progress reporter rewrites `.hexa/loop.json` (`updated`,
//! `running`) while every fix runs, so the file differed from the fix's
//! snapshot, counted as the fix's own path, and — dirty before the pass —
//! held every fix back (ADR-2610141733 §6).
//!
//! The reporter here does what the CLI's does: every progress line rewrites
//! `.hexa/loop.json`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::adversarial::{Progress, Reporter};
use hexa_exec::ports::{Frontier, FrontierError};

const TARGET: &str = "src/target.txt";
const LOOP: &str = ".hexa/loop.json";

struct FakeFrontier {
    bugs: usize,
    handed_out: AtomicBool,
}

#[async_trait]
impl Frontier for FakeFrontier {
    async fn run(&self, prompt: &str, cwd: &Path, _t: Duration, _s: &str) -> Result<String, FrontierError> {
        if prompt.contains("adversarial code reviewer") {
            if self.handed_out.swap(true, Ordering::SeqCst) {
                return Ok(r#"{"findings":[]}"#.to_string());
            }
            let findings: Vec<String> = (1..=self.bugs)
                .map(|n| format!(r#"{{"title":"bug-{n}","location":"{TARGET}:{n}","description":"defect {n}","lens":"correctness"}}"#))
                .collect();
            return Ok(format!(r#"{{"findings":[{}]}}"#, findings.join(",")));
        }
        if prompt.contains("SKEPTICALLY verify") {
            return Ok(r#"{"is_real":true,"reasoning":"it is"}"#.to_string());
        }
        if prompt.contains("Fix this CONFIRMED bug") {
            let n = (1..=self.bugs).find(|n| prompt.contains(&format!("title: bug-{n}\n"))).unwrap();
            std::fs::write(cwd.join(format!("src/fix_{n}.txt")), format!("fix {n}\n")).unwrap();
            // A real fix takes minutes, and every 30 s the phase heartbeat
            // ("still N bugs to fix") goes through the reporter, which
            // rewrites the loop state — inside the fix's snapshot window.
            // Waiting out a real heartbeat would cost 30 s a fix; the write
            // is what matters, so it is made here, mid-fix, as the CLI's
            // reporter makes it.
            std::fs::write(
                cwd.join(LOOP),
                format!("{{\"sessions\":{{\"s\":{{\"gate\":\"true\",\"running\":\"fix\",\"updated\":\"heartbeat {n}\"}}}}}}\n"),
            )
            .unwrap();
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
    std::fs::create_dir_all(dir.join(".hexa")).unwrap();
    std::fs::write(dir.join(TARGET), "original\n").unwrap();
    std::fs::write(dir.join(LOOP), "{\"sessions\":{}}\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-qm", "init"]);
    // The CLI records the gate before the pass starts: dirty before the pass.
    std::fs::write(dir.join(LOOP), "{\"sessions\":{\"s\":{\"gate\":\"true\"}}}\n").unwrap();
    (tmp, dir)
}

/// A reporter that rewrites `.hexa/loop.json` on every progress line, as
/// `hexa harden`'s does.
fn loop_writing_reporter(dir: &Path) -> Reporter {
    let dir = dir.to_path_buf();
    let n = Arc::new(AtomicUsize::new(0));
    Arc::new(move |p: Progress| {
        let i = n.fetch_add(1, Ordering::SeqCst);
        let _ = std::fs::write(
            dir.join(LOOP),
            format!("{{\"sessions\":{{\"s\":{{\"gate\":\"true\",\"running\":\"{}\",\"updated\":{i}}}}}}}\n", p.phase),
        );
    })
}

#[tokio::test]
async fn fixes_commit_while_the_reporter_rewrites_loop_state() {
    let (_tmp, dir) = repo();
    let report = hexa_exec::adversarial::run_review_with(
        Arc::new(FakeFrontier { bugs: 2, handed_out: AtomicBool::new(false) }),
        TARGET,
        "true",
        &dir,
        loop_writing_reporter(&dir),
    )
    .await;

    assert_eq!(report.fixed, vec!["bug-1", "bug-2"], "notes: {:?}", report.notes);
    assert!(report.uncommitted.is_empty(), "nothing held back: {:?}", report.notes);
    let committed = git(&dir, &["log", "--format=", "--name-only", "HEAD~2..HEAD"]);
    assert!(!committed.contains(LOOP), "hexa's loop state is never part of a fix commit: {committed}");
    assert!(
        git(&dir, &["status", "--porcelain"]).contains(LOOP),
        "the loop state stays the operator's, uncommitted"
    );
}

/// A fix that really does touch a file the operator had changed is still
/// held back: ignoring hexa's own state must not widen into ignoring theirs.
#[tokio::test]
async fn an_operator_dirty_file_still_holds_a_fix_back() {
    let (_tmp, dir) = repo();
    std::fs::write(dir.join("src/fix_1.txt"), "operator's draft\n").unwrap();
    let report = hexa_exec::adversarial::run_review_with(
        Arc::new(FakeFrontier { bugs: 1, handed_out: AtomicBool::new(false) }),
        TARGET,
        "true",
        &dir,
        loop_writing_reporter(&dir),
    )
    .await;

    assert!(report.fixed.is_empty(), "{:?}", report.fixed);
    assert_eq!(report.uncommitted, vec!["bug-1"], "notes: {:?}", report.notes);
    let note = report.notes.iter().find(|n| n.contains("bug-1")).cloned().unwrap_or_default();
    assert!(note.contains("src/fix_1.txt") && !note.contains(LOOP), "names the operator's file only: {note}");
}
