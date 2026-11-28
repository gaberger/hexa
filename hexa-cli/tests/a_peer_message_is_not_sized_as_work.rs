//! A message from another Claude session arrives on the prompt channel and is
//! not a person asking for work. It was sized as a feature (and, with
//! auto-planning on, drafted as a workplan) because the words in a status note
//! read like a task: "implement", "architecture", "migration".
//!
//! `route` already leaves a host's own notifications alone; a peer session's
//! message is the same kind of thing.

use std::path::Path;
use std::process::{Command, Stdio};

const FEATURE_WORDS: &str = "implement a new feature: add an architecture and migration across all modules with a new adapter";

fn route(home: &Path, project: &Path, session: &str, prompt: &str) -> String {
    let payload = serde_json::json!({ "session_id": session, "prompt": prompt, "cwd": project });
    let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["hook", "route"])
        .current_dir(project)
        .env("HOME", home)
        .env("CLAUDE_PROJECT_DIR", project)
        .env("CLAUDE_SESSION_ID", session)
        .env("HEXA_NO_CLAUDE_MODS", "1")
        .env_remove("HEXA_INTERNAL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), payload.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let project = d.path().join("p");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    (d, home, project)
}

#[test]
fn a_persons_feature_prompt_is_sized_as_work() {
    // Control: the sizing this file is about does happen to a person's prompt.
    let (_d, home, project) = dirs();
    let out = route(&home, &project, "control", FEATURE_WORDS);
    assert!(out.contains("feature-sized"), "the control prompt was not sized:\n{out}");
}

#[test]
fn a_peer_sessions_message_is_not() {
    let (_d, home, project) = dirs();
    let msg = format!(
        "<cross-session-message from=\"uds:/run/user/1000/cc-socks/1.sock\" from-name=\"peer\" from-mode=\"bypass\">\nStatus FYI, nothing for you to do: I did {FEATURE_WORDS}.\n</cross-session-message>"
    );
    let out = route(&home, &project, "peer", &msg);
    assert!(!out.contains("feature-sized") && !out.contains("workplan draft"), "a peer's message was sized as work:\n{out}");
}
