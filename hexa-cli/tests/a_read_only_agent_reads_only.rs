//! A subagent runs in place without a worktree only when its definition's
//! `tools` names nothing but tools that read: Read, Grep, Glob, LS,
//! NotebookRead, WebFetch, WebSearch. Anything else, `*`, a list hexa cannot
//! read, or a definition written for another agent is treated as an agent
//! that writes, and is blocked on a feature branch.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@x", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

struct Fx {
    proj: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl Fx {
    fn new() -> Fx {
        let proj = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let p = proj.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("README.md"), "demo\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "init"]);
        git(p, &["checkout", "-q", "-b", "feat/work"]);
        Fx { proj, home }
    }

    fn define(&self, file_stem: &str, frontmatter: &str) {
        let dir = self.proj.path().join(".claude/agents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{file_stem}.md")), format!("---\n{frontmatter}\n---\n\nAn agent.\n")).unwrap();
    }

    fn pre_agent(&self, kind: &str) -> Output {
        let input = serde_json::json!({
            "session_id": "read-only-agent-test",
            "cwd": self.proj.path().to_string_lossy(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Agent",
            "tool_input": {"subagent_type": kind, "description": "d", "prompt": "p"}
        })
        .to_string();
        let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["hook", "pre-agent"])
            .current_dir(self.proj.path())
            .env("HOME", self.home.path())
            .env("HEXA_HOME", self.home.path().join(".hexa"))
            .env("CLAUDE_PROJECT_DIR", self.proj.path())
            .env_remove("HEXA_INTERNAL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn blocked(&self, kind: &str) -> bool {
        let out = self.pre_agent(kind);
        let stdout = String::from_utf8_lossy(&out.stdout);
        out.status.code() == Some(2) || stdout.contains("\"deny\"") || stdout.contains("\"block\"")
    }
}

#[test]
fn the_baseline_editor_is_blocked() {
    let fx = Fx::new();
    assert!(fx.blocked("zz-no-definition"), "an agent with no definition must be blocked on a feature branch");
}

#[test]
fn only_read_tools_run_in_place() {
    let fx = Fx::new();
    let allowed = [
        ("a1", "tools: Read, Grep, Glob, LS, NotebookRead, WebFetch, WebSearch"),
        ("a2", "tools: Read,Grep"),
        ("a3", "tools: Read Grep Glob"),
        ("a4", "tools:\n  - Read\n  - Grep"),
        ("a5", "tools: WebFetch(domain:example.com), Read"),
    ];
    for (name, fm) in allowed {
        fx.define(name, &format!("name: {name}\n{fm}"));
        assert!(!fx.blocked(name), "{name} ({fm}) only reads and should run in place");
    }
}

#[test]
fn any_other_tool_is_assumed_to_write() {
    let fx = Fx::new();
    let refused = [
        ("r1", "tools: '*'"),
        ("r2", "tools: Read, *"),
        ("r3", "tools: Read, mcp__github__create_issue"),
        ("r4", "tools: Read, PowerShell"),
        ("r5", "tools: Read, TodoWrite"),
        ("r6", "tools: Read edit"),
        ("r7", "tools: Read, bash"),
        ("r8", "tools:\n  - Read\n  - 7"),
        ("r9", "tools:\n  - Read\n  - [Write]"),
        ("r10", "tools: Read, Task"),
        ("r11", "tools:\n  - Read Write"),
        ("r12", "tools: read, grep"),
    ];
    for (name, fm) in refused {
        fx.define(name, &format!("name: {name}\n{fm}"));
        assert!(fx.blocked(name), "{name} ({fm}) is not known to only read and must be blocked");
    }
}

#[test]
fn a_definition_named_for_another_agent_does_not_count() {
    let fx = Fx::new();
    fx.define("impostor", "name: someone-else\ntools: Read, Grep");
    assert!(fx.blocked("impostor"), "a definition whose name: is another agent is not this agent's definition");
    fx.define("honest", "name: honest\ntools: Read, Grep");
    assert!(!fx.blocked("honest"));
    fx.define("nameless", "tools: Read, Grep");
    assert!(fx.blocked("nameless"), "a definition with no name: matches no agent, whatever its file is called");
}
