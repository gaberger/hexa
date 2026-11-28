//! Claude Code names a subagent by its definition's frontmatter `name:`, not
//! by the file's name. The pre-agent hook finds a definition the same way: the
//! project's `.claude/agents/*.md` whose `name:` equals the subagent type, and
//! only when no project file claims it, the home folder's. A file with no
//! `name:` matches nothing. Two files in one folder claiming a name is no
//! definition. A `tools` token with unbalanced brackets or text after `)` is
//! not a tool hexa knows, and tool names match case-sensitively; an unknown
//! tool writes (ADR-2611221646 §5).

use std::io::Write;
use std::path::{Path, PathBuf};
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

    fn project_agents(&self) -> PathBuf {
        self.proj.path().join(".claude/agents")
    }

    fn home_agents(&self) -> PathBuf {
        self.home.path().join(".claude/agents")
    }

    fn write(dir: &Path, file_stem: &str, frontmatter: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{file_stem}.md")), format!("---\n{frontmatter}\n---\n\nAn agent.\n")).unwrap();
    }

    fn pre_agent(&self, kind: &str) -> Output {
        let input = serde_json::json!({
            "session_id": "agent-by-name-test",
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
fn a_project_definition_is_found_by_its_frontmatter_name() {
    let fx = Fx::new();
    assert!(fx.blocked("zz-no-definition"), "baseline: an undefined agent is blocked");
    Fx::write(&fx.project_agents(), "some-file", "name: reviewer\ntools: Read, Grep");
    assert!(!fx.blocked("reviewer"), "a file whose name: is reviewer defines reviewer, whatever the file is called");
}

#[test]
fn a_home_definition_is_found_by_its_frontmatter_name() {
    let fx = Fx::new();
    Fx::write(&fx.home_agents(), "home-file", "name: auditor\ntools: Read");
    assert!(!fx.blocked("auditor"), "a home file whose name: is auditor defines auditor");
}

#[test]
fn a_project_definition_is_final() {
    let fx = Fx::new();
    Fx::write(&fx.project_agents(), "writer-file", "name: reviewer\ntools: Read, Write");
    Fx::write(&fx.home_agents(), "reviewer", "name: reviewer\ntools: Read");
    assert!(fx.blocked("reviewer"), "the project's reviewer writes; home is not consulted");
}

#[test]
fn a_file_named_for_the_agent_without_that_name_does_not_define_it() {
    let fx = Fx::new();
    Fx::write(&fx.project_agents(), "reviewer", "name: someone-else\ntools: Read");
    assert!(fx.blocked("reviewer"));
    assert!(!fx.blocked("someone-else"), "the file defines the agent its name: names");
}

#[test]
fn a_definition_without_a_name_never_counts() {
    let fx = Fx::new();
    Fx::write(&fx.project_agents(), "nameless", "tools: Read, Grep");
    assert!(fx.blocked("nameless"), "a project file with no name: matches nothing");
    Fx::write(&fx.home_agents(), "homeless", "tools: Read, Grep");
    assert!(fx.blocked("homeless"), "a home file with no name: matches nothing");
}

#[test]
fn two_files_claiming_one_name_are_no_definition() {
    let fx = Fx::new();
    Fx::write(&fx.project_agents(), "dup", "name: dup\ntools: Read");
    Fx::write(&fx.project_agents(), "dup-copy", "name: dup\ntools: Read, Grep");
    assert!(fx.blocked("dup"), "two project files claiming dup: no definition");

    Fx::write(&fx.home_agents(), "twin", "name: twin\ntools: Read");
    Fx::write(&fx.home_agents(), "twin-copy", "name: twin\ntools: Grep");
    assert!(fx.blocked("twin"), "two home files claiming twin: no definition");
}

#[test]
fn malformed_tool_tokens_are_assumed_to_write() {
    let fx = Fx::new();
    let refused = [
        ("m1", "tools: 'Read(, Write'"),
        ("m2", "tools: 'Read(x)Write'"),
        ("m3", "tools: 'Read(x)Write, Grep'"),
        ("m4", "tools: 'Read), Write'"),
        ("m5", "tools: 'Read((x), Grep'"),
    ];
    for (name, fm) in refused {
        Fx::write(&fx.project_agents(), name, &format!("name: {name}\n{fm}"));
        assert!(fx.blocked(name), "{name} ({fm}) is malformed and must be blocked");
    }
    Fx::write(&fx.project_agents(), "ok", "name: ok\ntools: 'WebFetch(domain:x.com), Read'");
    assert!(!fx.blocked("ok"), "a balanced specifier still only reads");
}

#[test]
fn tool_names_match_case_sensitively() {
    let fx = Fx::new();
    for (name, fm) in [("c1", "tools: read"), ("c2", "tools: Read, grep"), ("c3", "tools: GLOB"), ("c4", "tools: webfetch")] {
        Fx::write(&fx.project_agents(), name, &format!("name: {name}\n{fm}"));
        assert!(fx.blocked(name), "{name} ({fm}) names no tool hexa knows and must be blocked");
    }
}
