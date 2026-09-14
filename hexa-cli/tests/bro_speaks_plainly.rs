//! `hexa bro`'s writing has a gate (ADR-2609140925, decision 4).
//!
//! This is the one surface in hexa whose job is to be understood by someone
//! who just walked back in. That makes its register load-bearing, and
//! load-bearing things get tested here rather than reviewed.
//!
//! Two rules, both mechanical. No sentence runs past 25 words. Every hexa verb
//! it names exists. A third rule is implied by the first two and matters most:
//! the extractor must find something, or a report that says nothing would pass
//! both checks perfectly.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/register.rs"]
mod register;
use register::assert_plain;

fn hexa_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").to_path_buf()
}

/// Seed loop state through the binary rather than by writing the file.
///
/// Loop state is session-scoped and written whole; a test that hand-writes it
/// is pinned to a shape it does not own, and breaks the moment that shape
/// changes for good reasons elsewhere.
fn seed(dir: &Path, args: &[&str]) {
    let out = Command::new(hexa_bin()).current_dir(dir).args(args).output().expect("seed");
    assert!(out.status.success(), "seeding failed: hexa {args:?}");
}

/// Run `hexa bro` in `dir` and return (exit ok, stdout).
fn bro(dir: &Path) -> (bool, String) {
    let out = Command::new(hexa_bin()).current_dir(dir).arg("bro").output().expect("run hexa bro");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string())
}

/// Every backticked `hexa …` chain in a report, as argument lists.
fn backticked_verbs(report: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut rest = report;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let span = &after[..close];
        rest = &after[close + 1..];
        let mut tokens = span.split_whitespace();
        if tokens.next() != Some("hexa") {
            continue;
        }
        let mut chain = Vec::new();
        for t in tokens {
            let checkable = !t.is_empty()
                && !t.starts_with('-')
                && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !checkable {
                break;
            }
            chain.push(t.to_string());
        }
        if !chain.is_empty() {
            out.push(chain);
        }
    }
    out
}

fn resolves(chain: &[String]) -> bool {
    Command::new(hexa_bin())
        .args(chain)
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A project with a decision, a gate and a checklist recorded.
///
/// Seeded rather than read from the tree this happens to run in. The first
/// version graded the workspace root, which passes only while that root has
/// loop state — in a fresh worktree there is none, the report is four lines
/// long, and the sentence floor fires on a report that is correct.
#[test]
fn the_report_reads_plainly_in_a_live_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".hexa")).expect("mkdir .hexa");
    seed(dir.path(), &["loop", "stage", "build"]);
    seed(dir.path(), &["loop", "gate", "cargo test --workspace"]);
    seed(dir.path(), &["loop", "task", "add", "the first step"]);

    let (ok, report) = bro(dir.path());
    assert!(ok, "hexa bro exited non-zero:\n{report}");
    assert_plain("live project", &report, 6);
}

/// And in a directory hexa has never touched, which is decision 5.
#[test]
fn an_untouched_directory_is_a_result_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (ok, report) = bro(dir.path());
    assert!(ok, "hexa bro exited non-zero in an empty directory:\n{report}");
    assert!(
        report.contains("no record of any work here"),
        "the empty case must say so plainly:\n{report}"
    );
    assert_plain("untouched directory", &report, 3);
}

/// Decision 4: every verb it names exists.
///
/// The states are seeded rather than read from this repository. A report names
/// different verbs depending on what is recorded, so testing against hexa's own
/// live loop state makes the vacuity floor a coin toss: the run that records
/// everything names two verbs and the run that records nothing names five.
/// A loop file with nothing in it fires every "not recorded" branch at once,
/// which is exactly the surface this test exists to check.
#[test]
fn every_verb_the_report_names_exists() {
    let blank = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(blank.path().join(".hexa")).expect("mkdir .hexa");
    seed(blank.path(), &["loop", "stage", "build"]);

    let untouched = tempfile::tempdir().expect("tempdir");

    let mut chains: Vec<Vec<String>> = Vec::new();
    for dir in [blank.path(), untouched.path(), workspace_root().as_path()] {
        chains.extend(backticked_verbs(&bro(dir).1));
    }

    assert!(
        chains.len() >= 5,
        "found only {} backticked hexa command(s); the extractor is broken",
        chains.len()
    );
    let dead: Vec<String> = chains
        .iter()
        .filter(|c| !resolves(c))
        .map(|c| format!("hexa {}", c.join(" ")))
        .collect();
    assert!(dead.is_empty(), "hexa bro names {} dead verb(s):\n  {}", dead.len(), dead.join("\n  "));
}

/// A report with nothing recorded still tells the operator what to do.
///
/// This is the state a new project is in, and the one where a status tool is
/// least useful and most needed.
#[test]
fn an_empty_record_still_names_the_way_forward() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".hexa")).expect("mkdir .hexa");
    // An entry that exists and records nothing else. `loop stage` would record
    // a stage, which is the opposite of what this test is about.
    seed(dir.path(), &["loop", "task", "add", "something to do"]);

    let (ok, report) = bro(dir.path());
    assert!(ok, "hexa bro exited non-zero on an empty record:\n{report}");
    for missing in ["No decision is recorded", "No gate is recorded", "No stage is recorded"] {
        assert!(report.contains(missing), "the empty record must say `{missing}`:\n{report}");
    }
    assert_plain("empty record", &report, 5);
}

/// Decision 2: it reads, and it changes nothing.
///
/// The surface an operator runs when they are lost must be safe to run when
/// they are lost. A status report that mutates state is a trap.
#[test]
fn the_report_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join(".hexa")).expect("mkdir .hexa");
    seed(root, &["loop", "stage", "build"]);
    seed(root, &["loop", "gate", "true"]);
    let before = std::fs::read_to_string(root.join(".hexa/loop.json")).expect("read");

    let (ok, _) = bro(root);
    assert!(ok);

    let after = std::fs::read_to_string(root.join(".hexa/loop.json")).expect("read");
    assert_eq!(before, after, "hexa bro rewrote the loop state it was only meant to read");
    let mut entries: Vec<String> = std::fs::read_dir(root)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    entries.sort();
    assert_eq!(entries, vec![".hexa".to_string()], "hexa bro created files: {entries:?}");
}

/// Decision 3: an unrecorded gate result is never shown as green.
#[test]
fn an_unrecorded_gate_result_is_never_implied_to_be_green() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join(".hexa")).expect("mkdir .hexa");
    seed(root, &["loop", "stage", "build"]);
    seed(root, &["loop", "gate", "cargo test"]);

    let (ok, report) = bro(root);
    assert!(ok);
    assert!(
        report.contains("No result is recorded"),
        "a gate with no recorded result must say so:\n{report}"
    );
    // And it must not imply one either way.
    assert!(!report.contains("It passed"), "an unrecorded gate read as passing:\n{report}");
    assert!(!report.contains("It failed"), "an unrecorded gate read as failing:\n{report}");
}
