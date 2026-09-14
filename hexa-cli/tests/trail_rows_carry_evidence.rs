//! The verbs that read a trail and check a playbook (ADR-2609140928, -0929).
//!
//! A trail nothing can read is a file, not a record. A playbook that can be
//! routed to but never printed or validated is an asset nobody can audit.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn hexa_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(hexa_bin()).current_dir(dir).args(args).output().expect("run hexa")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Absence is a result. A directory with no trails is not an error.
#[test]
fn no_trail_is_a_result_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = run(dir.path(), &["trail", "list"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("no run has left a trail"), "{}", text(&out));
}

#[test]
fn a_trail_is_listed_and_read_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trails = dir.path().join("docs/trails");
    std::fs::create_dir_all(&trails).expect("mkdir");
    std::fs::write(
        trails.join("run-1.tsv"),
        "2026-09-14T12:00:00Z\t1\trepo_read src/lib.rs\tcargo_check, repo_grep\tok: 40 lines\n\
         2026-09-14T12:00:09Z\t2\tpropose_edit src/lib.rs\trepo_read, repo_grep\tgate passed, committed abc1234\n",
    )
    .expect("seed trail");

    let listed = text(&run(dir.path(), &["trail", "list"]));
    assert!(listed.contains("run-1"), "{listed}");
    assert!(listed.contains("2 decision(s)"), "{listed}");

    let shown = text(&run(dir.path(), &["trail", "show", "run-1"]));
    assert!(shown.contains("repo_read src/lib.rs"), "the choice must survive:\n{shown}");
    assert!(shown.contains("cargo_check, repo_grep"), "so must the alternatives:\n{shown}");
    assert!(shown.contains("gate passed, committed abc1234"), "and the evidence:\n{shown}");
}

#[test]
fn asking_for_a_trail_that_is_not_there_is_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = run(dir.path(), &["trail", "show", "nope"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("no trail for"), "{}", text(&out));
}

/// ADR-2609140929 phase 1: the shipped playbooks can finally be seen.
#[test]
fn every_shipped_playbook_can_be_listed_and_printed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let listed = text(&run(dir.path(), &["playbook", "list"]));
    for name in ["bug-fix", "feature", "refactor", "investigation"] {
        assert!(listed.contains(name), "`{name}` is missing from the list:\n{listed}");
    }
    let shown = text(&run(dir.path(), &["playbook", "show", "bug-fix"]));
    assert!(shown.contains("playbook: bug-fix"), "{shown}");
    assert!(shown.contains("hexa analyze"), "the last step must survive printing:\n{shown}");
}

#[test]
fn an_unknown_playbook_names_the_ones_that_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = run(dir.path(), &["playbook", "show", "nonsense"]);
    assert!(out.status.success());
    let t = text(&out);
    assert!(t.contains("bug-fix"), "it must say what there is:\n{t}");
}

/// The validator must refuse, or it validates nothing.
#[test]
fn check_accepts_a_shipped_playbook_and_refuses_a_broken_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("root").to_path_buf();
    let good = run(&root, &["playbook", "check", "hexa-cli/assets/playbooks/bug-fix.json"]);
    assert!(good.status.success(), "a shipped playbook must pass:\n{}", text(&good));

    let dir = tempfile::tempdir().expect("tempdir");
    let bad = dir.path().join("bad.json");
    // Three steps, real verbs, but nothing proves anything and it does not end
    // at the grade. A validator that accepts this accepts everything.
    std::fs::write(
        &bad,
        serde_json::json!({
            "name": "bad",
            "summary": "a playbook that proves nothing",
            "triggers": ["bad"],
            "steps": [
                {"title": "look", "run": "hexa graph build .", "done_when": "done"},
                {"title": "look again", "run": "hexa graph consumers <p>", "done_when": "done"},
                {"title": "stop", "run": "hexa status", "done_when": "done"}
            ]
        })
        .to_string(),
    )
    .expect("write");

    let out = run(dir.path(), &["playbook", "check", bad.to_str().unwrap()]);
    assert!(!out.status.success(), "a playbook with no proof step must be refused:\n{}", text(&out));
    let t = text(&out);
    assert!(t.contains("no proof step"), "{t}");
    assert!(t.contains("not the architecture grade"), "{t}");
}

/// Decision 7: too little history is a result.
#[test]
fn learning_from_nothing_says_so_and_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = run(dir.path(), &["playbook", "learn"]);
    assert!(out.status.success(), "{}", text(&out));
    let t = text(&out);
    assert!(t.contains("recorded run(s)"), "{t}");
    assert!(t.contains("are needed"), "it must say how many it needs:\n{t}");
}
