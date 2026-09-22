//! An expected refusal prints a sentence, not a stack trace.
//!
//! `main` returned `anyhow::Result<()>`, so Rust's `Termination` printed the
//! error with `Debug` — and anyhow's `Debug` appends a backtrace whenever
//! `RUST_BACKTRACE` is set. Rust developers commonly export that globally, and
//! hexa's users are Rust developers, so an ordinary "write the ADR first"
//! refusal arrived looking like a crash.
//!
//! The message and the exit code were always right. Only the presentation was
//! wrong, and a tool that looks like it crashed when it is working teaches
//! people to distrust it.

use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

/// Run with `RUST_BACKTRACE` forced on — the condition that produced the noise.
fn run_with_backtrace(dir: &std::path::Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(hexa())
        .args(args)
        .current_dir(dir)
        .env("RUST_BACKTRACE", "1")
        .output()
        .expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// An ADR id that is guaranteed not to exist, assembled at run time.
///
/// Written whole, it would read as a citation to `hexa adr doctor`, which scans
/// source for `ADR-…` and requires every one to resolve. It is right to: a
/// dangling citation is a real defect. This is a fixture, not a reference, so
/// it is built from parts rather than weakening the check that catches the
/// real thing.
fn absent_adr() -> String {
    format!("ADR-{}", "2699999999")
}

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(d.path().join(".hexa")).unwrap();
    std::fs::create_dir_all(d.path().join("docs/adrs")).unwrap();
    d
}

#[test]
fn a_missing_adr_refusal_prints_no_backtrace() {
    let d = project();
    let id = absent_adr();
    let (code, out) = run_with_backtrace(d.path(), &["loop", "adr", &id]);
    assert_eq!(code, 1, "the refusal still fails:\n{out}");
    assert!(out.contains(&id), "and still says what is wrong:\n{out}");
    assert!(
        !out.contains("Stack backtrace") && !out.contains("libc_start"),
        "an expected refusal is not a crash:\n{out}"
    );
}

#[test]
fn an_analysis_of_nothing_prints_no_backtrace() {
    // The second error path, so this pins the shared exit rather than one verb.
    let d = project();
    let (code, out) = run_with_backtrace(d.path(), &["analyze", "/nonexistent-path-hexa-xyz"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        !out.contains("Stack backtrace") && !out.contains("libc_start"),
        "every expected refusal takes the same exit:\n{out}"
    );
}

#[test]
fn the_message_survives_without_the_backtrace() {
    let d = project();
    let (_, out) = run_with_backtrace(d.path(), &["loop", "adr", &absent_adr()]);
    assert!(
        out.contains("docs/adrs"),
        "the sentence that tells the reader what to do must not be lost:\n{out}"
    );
}

#[test]
fn success_is_unaffected() {
    let d = project();
    let out = Command::new(hexa())
        .args(["loop"])
        .current_dir(d.path())
        .env("RUST_BACKTRACE", "1")
        .output()
        .expect("run hexa");
    assert_eq!(out.status.code().unwrap_or(-1), 0, "a normal run still succeeds");
}
