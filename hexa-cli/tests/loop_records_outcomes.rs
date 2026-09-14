//! The loop recorded intentions and never recorded outcomes.
//!
//! It knew which gate must pass and never whether it had. So `hexa bro` said
//! "hexa has not recorded a result for it" on every single run — honest, and
//! useless. A gate that nothing ever runs is a note, not a gate.
//!
//! `hexa loop check` runs the recorded gate and writes down what it did.
//! `hexa analyze` writes down the grade it took. Both record what the result
//! was *about* — the gate string, the commit — because a result that outlives
//! its subject is a rumour.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn hexa_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

/// A project with a recorded gate and nothing else.
fn project(gate: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".hexa")).expect("mkdir .hexa");
    std::fs::write(
        dir.path().join(".hexa/loop.json"),
        serde_json::json!({ "gate": gate, "stage": "build" }).to_string(),
    )
    .expect("seed loop.json");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(hexa_bin()).current_dir(dir).args(args).output().expect("run hexa")
}

fn state(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join(".hexa/loop.json")).expect("read loop.json");
    serde_json::from_str(&text).expect("parse loop.json")
}

#[test]
fn a_passing_gate_is_written_down() {
    let dir = project("true");
    let out = run(dir.path(), &["loop", "check"]);
    assert!(out.status.success(), "a passing gate must exit 0");

    let st = state(dir.path());
    assert_eq!(st["gate_result"], serde_json::json!(true), "{st}");
    assert!(st["gate_result_at"].is_string(), "a result with no date is a rumour: {st}");
    assert_eq!(st["gate_result_for"], serde_json::json!("true"), "the result must name its gate: {st}");
}

#[test]
fn a_failing_gate_is_written_down_and_exits_non_zero() {
    let dir = project("false");
    let out = run(dir.path(), &["loop", "check"]);
    assert!(!out.status.success(), "a failing gate must exit non-zero, so a script can use it");
    assert_eq!(state(dir.path())["gate_result"], serde_json::json!(false));
}

#[test]
fn bro_reads_the_result_back_instead_of_shrugging() {
    let dir = project("true");
    let before = String::from_utf8_lossy(&run(dir.path(), &["bro"]).stdout).to_string();
    assert!(before.contains("No result is recorded"), "{before}");

    run(dir.path(), &["loop", "check"]);
    let after = String::from_utf8_lossy(&run(dir.path(), &["bro"]).stdout).to_string();
    assert!(after.contains("It passed,"), "the recorded result must surface:\n{after}");
    assert!(!after.contains("No result is recorded"), "{after}");
}

/// A result is about one gate. Change the gate and it is about nothing.
#[test]
fn a_result_for_a_different_gate_is_called_stale() {
    let dir = project("true");
    run(dir.path(), &["loop", "check"]);
    run(dir.path(), &["loop", "gate", "cargo test --workspace"]);

    let report = String::from_utf8_lossy(&run(dir.path(), &["bro"]).stdout).to_string();
    assert!(
        report.contains("The gate has changed since"),
        "a result for a gate that no longer exists must be called out:\n{report}"
    );
}

/// No gate recorded is a result, not an error.
#[test]
fn nothing_to_check_is_not_a_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".hexa")).expect("mkdir");
    std::fs::write(dir.path().join(".hexa/loop.json"), "{}").expect("seed");
    let out = run(dir.path(), &["loop", "check"]);
    assert!(out.status.success(), "an absent gate is a result, not an error");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("no gate recorded"), "{text}");
}

/// And `hexa loop check` runs the gate rather than believing the last answer.
#[test]
fn check_re_runs_the_gate_every_time() {
    let dir = project("false");
    run(dir.path(), &["loop", "check"]);
    assert_eq!(state(dir.path())["gate_result"], serde_json::json!(false));

    // The gate now passes. A cached verdict would still say false.
    run(dir.path(), &["loop", "gate", "true"]);
    run(dir.path(), &["loop", "check"]);
    assert_eq!(
        state(dir.path())["gate_result"],
        serde_json::json!(true),
        "the verdict was cached rather than re-taken"
    );
}
