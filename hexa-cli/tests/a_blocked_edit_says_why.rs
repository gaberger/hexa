//! When the pre-edit hook blocks an edit, the reason reaches the person.
//!
//! Claude Code shows a blocking hook's stderr and nothing else. The hook
//! printed its reason to stdout, so a blocked edit arrived as "No stderr
//! output" — a gate that refused without saying why. It also refused an edit
//! to a shell profile outside the project, because a prompt about the project
//! had been sized as feature work. Found by a user.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const SESSION: &str = "blocked-edit-test";

/// `hexa hook <event>` with `payload` on stdin, under a private HOME so the
/// session state it writes is this test's alone.
fn hook(home: &Path, project: &Path, event: &str, payload: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["hook", event])
        .current_dir(project)
        .env("HOME", home)
        .env("CLAUDE_PROJECT_DIR", project)
        .env("CLAUDE_CODE_SESSION_ID", SESSION)
        .env_remove("HEXA_SESSION_ID")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("HEXA_INTERNAL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn edit(home: &Path, project: &Path, file: &Path) -> Output {
    let payload = serde_json::json!({
        "session_id": SESSION,
        "tool_name": "Edit",
        "tool_input": { "file_path": file.to_str().unwrap() },
    });
    hook(home, project, "pre-edit", &payload.to_string())
}

/// A scaffolded project, and a session whose last prompt was sized as work
/// with a shape and which has recorded no gate.
fn sized_session() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = dir.path().join("p");
    std::fs::create_dir_all(&home).unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["init", project.to_str().unwrap(), "--scaffold", "--lang", "rust"])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));

    let prompt = serde_json::json!({
        "session_id": SESSION,
        "prompt": "Implement a full audit of the HTTP adapter with retries and tests.",
    });
    hook(&home, &project, "route", &prompt.to_string());

    // Not vacuous: the precondition this file is about has to hold.
    let state = std::fs::read_to_string(home.join(format!(".hexa/sessions/agent-{SESSION}.json")))
        .expect("the route hook recorded the session");
    assert!(
        state.contains("\"T2\"") || state.contains("\"T3\""),
        "the prompt was not sized as work with a shape:\n{state}"
    );
    (dir, home, project)
}

#[test]
fn a_blocked_edit_puts_its_reason_on_stderr() {
    let (_dir, home, project) = sized_session();
    let out = edit(&home, &project, &project.join("src/lib.rs"));
    assert_eq!(out.status.code(), Some(2), "an ungated edit in the project is blocked");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No gate recorded"), "the reason must be on stderr, got: [{stderr}]");
}

#[test]
fn a_file_outside_the_project_is_not_held_to_its_gate() {
    let (dir, home, project) = sized_session();
    let out = edit(&home, &project, &dir.path().join("home/.zshenv"));
    assert_eq!(
        out.status.code(),
        Some(0),
        "the project's gate does not govern a file outside it: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_recorded_gate_lets_the_edit_through() {
    // The control: the block is for the missing gate, not for every edit.
    let (_dir, home, project) = sized_session();
    let gate = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["loop", "gate", "cargo test"])
        .current_dir(&project)
        .env("HOME", &home)
        .env("CLAUDE_CODE_SESSION_ID", SESSION)
        .env_remove("HEXA_SESSION_ID")
        .output()
        .unwrap();
    assert!(gate.status.success(), "{}", String::from_utf8_lossy(&gate.stderr));
    let out = edit(&home, &project, &project.join("src/lib.rs"));
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
}
