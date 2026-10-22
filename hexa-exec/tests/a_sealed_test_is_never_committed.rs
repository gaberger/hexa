//! No agent commit carries an edit to a sealed test (ADR-2610221540 §6).
//!
//! A spec's tests are sealed once a person has accepted the spec. The code
//! side may only make them pass, never change what passing means. The agent
//! may edit any file it can reach. The `claude -p` delegate edits files itself
//! and bypasses the tool library (ADR-2610141733). So the guard sits where
//! every agent's work becomes a commit, and it judges the whole tree: a sealed
//! test loosened and left uncommitted still made the gate pass.
//!
//! The frontier here is a fake. It confirms two bugs. It fixes the first by
//! appending to the target and loosening a sealed test, and the second by
//! appending to the target only.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::ports::{Frontier, FrontierError};
use sha2::{Digest, Sha256};

const TARGET: &str = "src/target.txt";
const SEALED: &str = "tests/sealed.rs";
const SEALED_BODY: &str = "#[test]\nfn the_rule_holds() { assert_eq!(2 + 2, 4); }\n";

struct FakeFrontier {
    handed_out: AtomicBool,
}

#[async_trait]
impl Frontier for FakeFrontier {
    async fn run(&self, prompt: &str, cwd: &Path, _t: Duration, _s: &str) -> Result<String, FrontierError> {
        if prompt.contains("adversarial code reviewer") {
            if self.handed_out.swap(true, Ordering::SeqCst) {
                return Ok(r#"{"findings":[]}"#.to_string());
            }
            return Ok(format!(
                r#"{{"findings":[{{"title":"bug-1","location":"{TARGET}:1","description":"d1","lens":"correctness"}},{{"title":"bug-2","location":"{TARGET}:2","description":"d2","lens":"correctness"}}]}}"#
            ));
        }
        if prompt.contains("SKEPTICALLY verify") {
            return Ok(r#"{"is_real":true,"reasoning":"it is"}"#.to_string());
        }
        if prompt.contains("Fix this CONFIRMED bug") {
            let n = if prompt.contains("title: bug-1\n") { 1 } else { 2 };
            let target = cwd.join(TARGET);
            let body = std::fs::read_to_string(&target).unwrap() + &format!("fix {n}\n");
            std::fs::write(&target, body).unwrap();
            if n == 1 {
                // Loosen the sealed rule so the fix "passes".
                std::fs::write(cwd.join(SEALED), "#[test]\nfn the_rule_holds() {}\n").unwrap();
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

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A repository with a target, a sealed test, and the spec that seals it.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    git(&dir, &["init", "-q"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::create_dir_all(dir.join("docs/specs")).unwrap();
    std::fs::write(dir.join(TARGET), "original\n").unwrap();
    std::fs::write(dir.join(SEALED), SEALED_BODY).unwrap();
    let adr = format!("ADR-{}", "2701010000");
    std::fs::write(
        dir.join("docs/specs").join(format!("{adr}.json")),
        format!(
            r#"{{"adr":"{adr}","feature":"f","sealed":{{"{SEALED}":"{}"}},"specs":[]}}"#,
            sha256(SEALED_BODY.as_bytes())
        ),
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-qm", "init"]);
    (tmp, dir)
}

#[test]
fn an_intact_tree_has_no_broken_seal() {
    let (_tmp, dir) = repo();
    assert!(hexa_exec::direct_exec::broken_seals(&dir).is_empty());
}

#[test]
fn an_edited_or_missing_sealed_test_is_a_broken_seal() {
    let (_tmp, dir) = repo();
    std::fs::write(dir.join(SEALED), "// loosened\n").unwrap();
    assert_eq!(hexa_exec::direct_exec::broken_seals(&dir), vec![SEALED.to_string()]);
    std::fs::remove_file(dir.join(SEALED)).unwrap();
    assert_eq!(hexa_exec::direct_exec::broken_seals(&dir), vec![SEALED.to_string()], "a deleted test is the loosest edit of all");
}

#[tokio::test]
async fn harden_reverts_a_fix_that_edits_a_sealed_test_and_commits_the_rest() {
    let (_tmp, dir) = repo();
    let report = hexa_exec::adversarial::run_review(
        Arc::new(FakeFrontier { handed_out: AtomicBool::new(false) }),
        TARGET,
        "true",
        &dir,
    )
    .await;

    assert_eq!(report.confirmed.len(), 2, "notes: {:?}", report.notes);
    assert_eq!(report.fixed, vec!["bug-2"], "only the fix that left the seal alone counts; notes: {:?}", report.notes);
    assert_eq!(std::fs::read_to_string(dir.join(SEALED)).unwrap(), SEALED_BODY, "the sealed test is restored");
    assert!(
        report.notes.iter().any(|n| n.contains("bug-1") && n.contains("sealed")),
        "the refusal names the fix and the seal: {:?}",
        report.notes
    );
    let log = git(&dir, &["log", "--format=%s", "HEAD"]);
    assert!(!log.contains("bug-1"), "no commit carries the loosened test:\n{log}");
    assert!(log.contains("bug-2"), "{log}");
    let shown = git(&dir, &["log", "-p", "--format=", "HEAD", "--", SEALED]);
    assert!(!shown.contains("fn the_rule_holds() {}"), "no commit ever carried the loosened body");
}
