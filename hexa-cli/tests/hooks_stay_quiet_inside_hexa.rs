//! A claude that hexa itself starts carries HEXA_INTERNAL, and the project's
//! hooks stay quiet inside it. Before this, `hexa harden` ran its three
//! reviewer prompts through `claude -p`, the `route` hook inside that claude
//! sized "You are an adversarial code reviewer…" as feature-sized work, and
//! drafted a workplan for it under docs/workplans/drafts/. Found by a user.

use std::io::Write;
use std::process::{Command, Stdio};

fn hexa() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hexa"))
}

fn route(project: &std::path::Path, internal: bool) -> String {
    let mut cmd = hexa();
    cmd.args(["hook", "route"])
        .current_dir(project)
        .env("CLAUDE_PROJECT_DIR", project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if internal {
        cmd.env("HEXA_INTERNAL", "1");
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"session_id":"quiet-test","prompt":"You are an adversarial code reviewer. Implement a full audit of the HTTP adapter with retries and tests."}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn inside_hexa_the_route_hook_says_nothing_and_drafts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("p");
    assert!(hexa().args(["init", project.to_str().unwrap(), "--scaffold", "--lang", "rust"]).output().unwrap().status.success());
    let drafts = project.join("docs/workplans/drafts");

    let quiet = route(&project, true);
    assert!(quiet.trim().is_empty(), "hooks spoke inside hexa's own claude:\n{quiet}");
    let drafted = std::fs::read_dir(&drafts).map(|d| d.count()).unwrap_or(0);
    assert_eq!(drafted, 0, "a workplan was drafted from hexa's own prompt");

    // The same prompt from a person is sized and answered.
    let loud = route(&project, false);
    assert!(loud.contains("[HEX]"), "a person's prompt got no routing:\n{loud}");
    let _ = std::fs::remove_file(dirs::home_dir().unwrap().join(".hexa/sessions/agent-quiet-test.json"));
}
