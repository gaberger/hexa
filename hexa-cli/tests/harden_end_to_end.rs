//! `hexa harden` driven end to end against doubles (ADR-2609131948).
//!
//! `adversarial.rs` is well covered where it is pure and untested where it
//! decides. Four defects shipped from that gap in one day, each a claim the
//! report had no right to make. Every case here is one of them.
//!
//! The harness is spawned as a child process so its `HOME`,
//! `HEXA_CLAUDE_BINARY` and project root can be set without touching this
//! process's environment, which ADR-2609131749 forbids.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A local model endpoint serving one canned body to every request.
struct ModelDouble {
    url: String,
}

/// Serve `body` as an OpenAI chat-completions reply until the process ends.
fn model_double(body: &'static str) -> ModelDouble {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            // Drain the headers, then the body if one is announced.
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            if len > 0 {
                let mut buf = vec![0u8; len];
                let _ = reader.read_exact(&mut buf);
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    ModelDouble { url }
}

/// An OpenAI reply carrying `content`.
fn reply_with(content: &str) -> String {
    format!(
        r#"{{"id":"c","object":"chat.completion","model":"m","choices":[{{"index":0,"message":{{"role":"assistant","content":{}}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}}}"#,
        serde_json::to_string(content).unwrap()
    )
}

/// The reply a reasoning model sends when its budget runs out first: no
/// content, and a length stop. This is the shape gpt-oss-120b returned.
const TRUNCATED_REPLY: &str = r#"{"id":"c","object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":null,"reasoning":"thinking…"},"finish_reason":"length"}],"usage":{"prompt_tokens":1,"completion_tokens":64,"total_tokens":65}}"#;

/// A project with one target file, a gate that passes, and a registry
/// pointing at `model_url`.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, model_url: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("hexa-e2e-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".hexa")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("home/.hexa")).unwrap();
        std::fs::write(dir.join("src/target.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();
        std::fs::write(
            dir.join(".hexa/project.json"),
            r#"{"name":"e2e","inference":{"tier_models":{"t2.5":"double-model"}}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("home/.hexa/inference-servers.json"),
            format!(
                r#"{{"endpoints":[{{"id":"double","url":"{model_url}","provider":"openai-compat","model":"double-model","models":"[\"double-model\"]","apiKeyRef":"","requiresAuth":false,"status":"healthy"}}]}}"#
            ),
        )
        .unwrap();
        // A git repo, so the pass can read the tree and decide about committing.
        let git = |args: &[&str]| {
            Command::new("git").args(args).current_dir(&dir).output().expect("git");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "e2e@test"]);
        git(&["config", "user.name", "e2e"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "fixture"]);
        Fixture { dir }
    }

    /// A fake `claude` that prints `output` and exits 0 — which is what the
    /// real one does when it declines.
    fn frontier_says(&self, output: &str) -> PathBuf {
        let path = self.dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\ncat <<'EOF_BODY'\n{output}\nEOF_BODY\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn harden(&self, claude: &Path) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["harden", "src/target.rs", "--gate", "true"])
            .current_dir(&self.dir)
            .env("HOME", self.dir.join("home"))
            .env("HEXA_PROJECT_ROOT", &self.dir)
            .env("HEXA_CLAUDE_BINARY", claude)
            .env("HEXA_REVIEW_MAX_TOKENS", "256")
            .output()
            .expect("run hexa harden");
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        (out.status.success(), text)
    }

    fn commits(&self) -> usize {
        let out = Command::new("git").args(["rev-list", "--count", "HEAD"]).current_dir(&self.dir).output().expect("git");
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const SPEND_LIMIT: &str = "You've hit your monthly spend limit. Switch to another model, or manage usage credits.";

/// The false-clean defect: every lens refused, and the pass reported a clean
/// file and committed. It must now say nothing was reviewed, commit nothing,
/// and fail.
#[test]
fn a_pass_where_nothing_answered_reports_nothing_reviewed_and_does_not_commit() {
    let model = model_double(Box::leak(reply_with("I cannot help with that.").into_boxed_str()));
    let f = Fixture::new("silent", &model.url);
    let before = f.commits();
    let (ok, text) = f.harden(&f.frontier_says(SPEND_LIMIT));

    assert!(text.contains("nothing was reviewed"), "{text}");
    assert!(!text.contains("confirmed real"), "no clean-sounding count: {text}");
    assert!(text.contains("spend limit"), "the refusal is quoted: {text}");
    assert_eq!(f.commits(), before, "a pass that reviewed nothing commits nothing");
    assert!(!ok, "and does not exit 0");
}

/// The fallback: the frontier declines, the local model answers, and the
/// report names which reviewer produced the findings.
#[test]
fn a_frontier_that_declines_falls_back_and_the_report_names_the_local_reviewer() {
    let findings = r#"{"findings":[{"title":"add overflows","location":"add","description":"i32 addition wraps in release","lens":"correctness"}]}"#;
    let model = model_double(Box::leak(reply_with(findings).into_boxed_str()));
    let f = Fixture::new("fallback", &model.url);
    let (_, text) = f.harden(&f.frontier_says(SPEND_LIMIT));

    assert!(text.contains("reviewed by double-model"), "names the reviewer: {text}");
    assert!(!text.contains("nothing was reviewed"), "{text}");
    assert!(text.contains("candidate"), "{text}");
}

/// The truncation defect: a reasoning model that spends its budget thinking
/// returns no content, which became `empty reply`. It must name the budget.
#[test]
fn a_truncated_reply_is_reported_as_a_budget_failure_not_an_empty_answer() {
    let model = model_double(TRUNCATED_REPLY);
    let f = Fixture::new("truncated", &model.url);
    let (ok, text) = f.harden(&f.frontier_says(SPEND_LIMIT));

    assert!(text.contains("256"), "names the budget it hit: {text}");
    assert!(text.contains("HEXA_REVIEW_MAX_TOKENS"), "names the knob: {text}");
    assert!(!text.contains("empty reply"), "the symptom replaced by the cause: {text}");
    assert!(!ok, "nothing was reviewed, so the pass fails");
}

/// The frontier answering is still the ordinary path, and it is named too.
#[test]
fn a_frontier_that_answers_is_used_and_named() {
    let model = model_double(Box::leak(reply_with("unused").into_boxed_str()));
    let f = Fixture::new("frontier", &model.url);
    let findings = r#"{"findings":[]}"#;
    let (_, text) = f.harden(&f.frontier_says(findings));

    assert!(text.contains("reviewed by frontier"), "{text}");
    assert!(!text.contains("double-model"), "no fallback when the frontier answers: {text}");
    assert!(!text.contains("nothing was reviewed"), "an empty list is a real answer: {text}");
}
