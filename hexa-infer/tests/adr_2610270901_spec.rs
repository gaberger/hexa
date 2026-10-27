//! ADR-2610270901: `claude-text` serves Claude as a tool-less completion.
//!
//! Written by hand from the accepted spec, before the adapter, under the
//! ADR's §5 bootstrap exception: the route a test writer needs is the route
//! these tests are for.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use hexa_core::ports::inference::{
    IInferencePort, InferenceRequest, Message, Priority, ToolDefinition, ToolInputSchema,
};
use hexa_infer::adapters::secondary::claude_code::testing::{DirSeen, MockProcessSpawner, MockResponse};
use hexa_infer::adapters::secondary::claude_code::SpawnedProcess;
use hexa_infer::adapters::secondary::ClaudeCodeInferenceAdapter;
use hexa_infer::wiring::{adapter_for_endpoint, claude_text_model, served_by_frontier_cli, served_by_frontier_cli_in};
use hexa_infer::Endpoint;

const ISOLATION: [&str; 6] =
    ["--tools", "--setting-sources", "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence", "-p"];

fn ok(stdout: &str) -> MockResponse {
    MockResponse::Ok(SpawnedProcess { stdout: stdout.as_bytes().to_vec(), stderr: Vec::new(), exit_code: 0 })
}

fn failed() -> MockResponse {
    MockResponse::Ok(SpawnedProcess { stdout: Vec::new(), stderr: b"boom".to_vec(), exit_code: 1 })
}

fn request(system: &str, messages: Vec<Message>) -> InferenceRequest {
    InferenceRequest {
        model: "claude-text".into(),
        system_prompt: system.into(),
        messages,
        tools: Vec::new(),
        max_tokens: 100,
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    }
}

fn tool() -> ToolDefinition {
    ToolDefinition {
        name: "read_file".into(),
        description: "reads a file".into(),
        input_schema: ToolInputSchema {
            schema_type: "object".into(),
            properties: serde_json::json!({}),
            required: Vec::new(),
        },
    }
}

fn ep(provider: &str, model: &str) -> Endpoint {
    Endpoint { id: "e".into(), provider: provider.into(), model: model.into(), models: vec![model.into()], ..Default::default() }
}

struct Ran {
    result: Result<String, String>,
    args: Vec<Vec<String>>,
    dirs: Vec<DirSeen>,
}

async fn run(model: Option<&str>, responses: Vec<MockResponse>, requests: Vec<InferenceRequest>) -> Ran {
    let mock = Arc::new(MockProcessSpawner::with_responses(responses));
    let adapter = ClaudeCodeInferenceAdapter::text_with_spawner(model.map(str::to_string), mock.clone());
    let mut result = Err("no request".to_string());
    for r in requests {
        result = adapter
            .complete(r)
            .await
            .map(|resp| {
                resp.content
                    .iter()
                    .filter_map(|b| match b {
                        hexa_core::ports::inference::ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .map_err(|e| e.to_string());
    }
    Ran { result, args: mock.calls().into_iter().map(|(_, a)| a).collect(), dirs: mock.dirs() }
}

fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn prompt(args: &[String]) -> &str {
    args.last().map(String::as_str).unwrap_or("")
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn git(args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(root()).output().expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn spec(adr: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(root().join(format!("docs/specs/{adr}.json"))).expect("the spec exists");
    serde_json::from_str(&text).expect("the spec parses")
}

// ── §1 ────────────────────────────────────────────────────────────────────

/// S01: The bare `claude-text` model id is served by the Claude CLI adapter in text mode with the isolating flags
#[tokio::test]
async fn bare_claude_text_is_served_in_text_mode_with_the_isolating_flags() {
    assert_eq!(claude_text_model("claude-text"), Some(None));
    let ran = run(None, vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
    assert_eq!(ran.result.as_deref(), Ok("hi"));
    let args = &ran.args[0];
    for flag in ISOLATION {
        assert!(args.iter().any(|a| a == flag), "{flag} missing from {args:?}");
    }
    assert_eq!(after(args, "--tools"), Some(""), "no tool is allowed: {args:?}");
    assert_eq!(after(args, "--setting-sources"), Some(""), "no setting source is read: {args:?}");
    assert_eq!(after(args, "--output-format"), Some("text"));
}

/// S02: An endpoint registered with provider `claude-text` is served the same way.
#[test]
fn an_endpoint_with_provider_claude_text_is_a_tool_less_completion() {
    let e = ep("claude-text", "claude-text");
    let caps = adapter_for_endpoint(&e).capabilities();
    assert!(!caps.supports_tool_use, "a claude-text endpoint runs no tools");
    assert_eq!(caps.models.first().map(|m| m.provider.as_str()), Some("claude-text"));
    assert!(!served_by_frontier_cli_in(&[e], "claude-text"), "and it is not an agent");
}

/// S03: The CLI runs in a new empty directory, which is removed afterwards.
#[tokio::test]
async fn the_cli_runs_in_a_new_empty_directory_removed_afterwards() {
    let ran = run(None, vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
    assert_eq!(ran.dirs.len(), 1, "one spawn, in a directory of its own");
    let d = &ran.dirs[0];
    assert!(d.existed && d.empty, "the directory exists and is empty while the CLI runs: {d:?}");
    assert!(!d.path.exists(), "and is gone afterwards: {}", d.path.display());
}

/// S04: Text mode never passes `--dangerously-skip-permissions`.
#[tokio::test]
async fn text_mode_never_skips_permissions() {
    let ran = run(Some("m"), vec![ok("hi")], vec![request("sys", vec![Message::user("hello")])]).await;
    assert_eq!(ran.args.len(), 1);
    assert!(!ran.args[0].iter().any(|a| a == "--dangerously-skip-permissions"), "{:?}", ran.args[0]);
}

/// S05: The empty directory is removed even when the CLI fails.
#[tokio::test]
async fn the_directory_is_removed_when_the_cli_fails() {
    let exit = run(None, vec![failed()], vec![request("", vec![Message::user("hello")])]).await;
    assert!(exit.result.is_err(), "a failed CLI is an error");
    let spawn = run(
        None,
        vec![MockResponse::Err(hexa_core::ports::inference::InferenceError::Network("gone".into()))],
        vec![request("", vec![Message::user("hello")])],
    )
    .await;
    assert!(spawn.result.is_err());
    for d in exit.dirs.iter().chain(&spawn.dirs) {
        assert!(!d.path.exists(), "left behind after a failure: {}", d.path.display());
    }
    assert_eq!(exit.dirs.len() + spawn.dirs.len(), 2, "each failure ran in a directory of its own");
}

/// S06: Text mode does not run in the caller's directory.
#[tokio::test]
async fn text_mode_does_not_run_in_the_callers_directory() {
    let ran = run(None, vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(ran.dirs.len(), 1, "text mode names its directory");
    assert_ne!(ran.dirs[0].path, cwd);
    assert!(!ran.dirs[0].path.starts_with(root()), "not inside the repository either");
}

/// S07: Every `claude-text` invocation is isolated and cleaned up.
#[tokio::test]
async fn every_invocation_is_isolated_and_cleaned_up() {
    let cases: Vec<(Option<&str>, InferenceRequest)> = vec![
        (None, request("", vec![Message::user("a")])),
        (Some("m1"), request("sys", vec![Message::user("b")])),
        (Some("x/y"), request("s", vec![Message::user("c"), Message::assistant("d"), Message::user("e")])),
        (None, request("long ".repeat(200).as_str(), vec![Message::user("f")])),
    ];
    let mut seen = Vec::new();
    for (model, r) in cases {
        let ran = run(model, vec![ok("ok")], vec![r]).await;
        assert_eq!(ran.dirs.len(), 1);
        let d = &ran.dirs[0];
        assert!(d.existed && d.empty && !d.path.exists(), "{d:?}");
        for flag in ISOLATION {
            assert!(ran.args[0].iter().any(|a| a == flag), "{flag} missing");
        }
        assert!(!ran.args[0].iter().any(|a| a == "--dangerously-skip-permissions"));
        seen.push(d.path.clone());
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4, "no two invocations share a directory");
}

// ── §2 ────────────────────────────────────────────────────────────────────

/// S08: The system prompt and the user message are both delivered.
#[tokio::test]
async fn the_system_prompt_and_the_user_message_are_both_delivered() {
    let ran = run(None, vec![ok("hi")], vec![request("You are the spec author.", vec![Message::user("The ADR")])]).await;
    let args = &ran.args[0];
    assert_eq!(after(args, "--system-prompt"), Some("You are the spec author."));
    assert!(prompt(args).contains("The ADR"), "{args:?}");
}

/// S09: The system prompt is not dropped or folded into the prompt.
#[tokio::test]
async fn the_system_prompt_is_not_folded_into_the_prompt() {
    let ran = run(None, vec![ok("hi")], vec![request("SYSTEM-MARKER", vec![Message::user("user text")])]).await;
    let args = &ran.args[0];
    assert!(args.iter().any(|a| a == "--system-prompt"), "{args:?}");
    assert!(!prompt(args).contains("SYSTEM-MARKER"), "the prompt carries the conversation only: {args:?}");
}

/// S10: With no system prompt, no `--system-prompt` flag is sent.
#[tokio::test]
async fn no_system_prompt_sends_no_flag() {
    let ran = run(None, vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
    assert_eq!(ran.args.len(), 1);
    assert!(!ran.args[0].iter().any(|a| a == "--system-prompt"), "{:?}", ran.args[0]);
}

/// S11: A multi-turn conversation is sent as the prompt, not just the last user message.
#[tokio::test]
async fn a_multi_turn_conversation_is_sent_whole() {
    let ran = run(
        None,
        vec![ok("hi")],
        vec![request("", vec![Message::user("FIRST-Q"), Message::assistant("FIRST-A"), Message::user("SECOND-Q")])],
    )
    .await;
    let p = prompt(&ran.args[0]);
    let (q1, a1, q2) = (p.find("FIRST-Q"), p.find("FIRST-A"), p.find("SECOND-Q"));
    assert!(q1.is_some() && a1.is_some() && q2.is_some(), "every turn is sent: {p:?}");
    assert!(q1 < a1 && a1 < q2, "in order: {p:?}");
}

/// S12: Nothing in the request is lost.
#[tokio::test]
async fn nothing_in_the_request_is_lost() {
    let convos: Vec<(&str, Vec<Message>)> = vec![
        ("sys-1", vec![Message::user("u-1")]),
        ("", vec![Message::user("u-2"), Message::assistant("a-2"), Message::user("u-3")]),
        ("sys-4 with spaces\nand a line", vec![Message::user("u-4 \"quoted\"")]),
    ];
    for (system, messages) in convos {
        let texts: Vec<String> = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                hexa_core::ports::inference::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let ran = run(None, vec![ok("ok")], vec![request(system, messages)]).await;
        let args = &ran.args[0];
        if !system.is_empty() {
            assert_eq!(after(args, "--system-prompt"), Some(system));
        }
        for t in texts {
            assert!(prompt(args).contains(&t), "{t:?} lost from {args:?}");
        }
    }
}

// ── §3 ────────────────────────────────────────────────────────────────────

/// S13: `claude-text` is not treated as a frontier CLI agent.
#[test]
fn claude_text_is_not_an_agent() {
    assert!(!served_by_frontier_cli_in(&[], "claude-text"));
    assert!(!served_by_frontier_cli_in(&[], "claude-text:some-model"));
}

/// S14: The spec verbs accept `claude-text`.
#[test]
fn the_spec_verbs_accept_claude_text() {
    // `hexa spec draft` and `spec tests` refuse exactly the models this
    // predicate calls agents (`spec_author::refuse_an_agent`).
    assert!(!served_by_frontier_cli("claude-text"));
    assert!(!served_by_frontier_cli("claude-text:some-model"));
}

/// S15: A request carrying tool definitions is refused, and the error names `claude-text`.
#[tokio::test]
async fn a_request_with_tools_is_refused_naming_claude_text() {
    let mut r = request("", vec![Message::user("hello")]);
    r.tools = vec![tool()];
    let ran = run(None, vec![ok("should not be reached")], vec![r]).await;
    let err = ran.result.expect_err("a tool-carrying request is refused");
    assert!(err.contains("claude-text"), "the error names claude-text: {err}");
    assert!(ran.args.is_empty(), "nothing is spawned: {:?}", ran.args);
}

/// S16: Other `claude` ids are still treated as agents.
#[test]
fn other_claude_ids_are_still_agents() {
    for id in ["claude-code", "claude", "claude-some-model", "claude-texts", "claude-textual"] {
        assert!(served_by_frontier_cli_in(&[], id), "{id} is still the agent");
        assert_eq!(claude_text_model(id), None, "{id} is not a claude-text id");
    }
    assert!(served_by_frontier_cli_in(&[ep("claude-code", "x")], "x"));
}

/// S17: No `claude-text` id is an agent, and no tool-carrying request is served.
#[tokio::test]
async fn no_claude_text_id_is_an_agent_and_none_serves_tools() {
    for (id, model) in [("claude-text", None), ("claude-text:a", Some("a")), ("claude-text:b/c-1", Some("b/c-1"))] {
        assert!(!served_by_frontier_cli_in(&[], id), "{id}");
        assert!(!served_by_frontier_cli_in(&[ep("claude-text", id)], id), "{id} registered");
        let mut r = request("s", vec![Message::user("u")]);
        r.model = id.into();
        r.tools = vec![tool()];
        let ran = run(model, vec![ok("x")], vec![r]).await;
        assert!(ran.result.is_err() && ran.args.is_empty(), "{id} served a tool request");
    }
}

// ── §4 ────────────────────────────────────────────────────────────────────

/// S18: The suffix after the colon selects the model.
#[tokio::test]
async fn the_suffix_selects_the_model() {
    assert_eq!(claude_text_model("claude-text:some-model"), Some(Some("some-model".into())));
    let ran = run(Some("some-model"), vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
    assert_eq!(after(&ran.args[0], "--model"), Some("some-model"));
}

/// S19: Bare `claude-text` passes no `--model`.
#[tokio::test]
async fn bare_claude_text_passes_no_model() {
    let ran = run(None, vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
    assert_eq!(ran.args.len(), 1);
    assert!(!ran.args[0].iter().any(|a| a == "--model"), "{:?}", ran.args[0]);
}

/// S20: No concrete Claude model id is hard-coded outside `hexa-infer`.
#[test]
fn no_claude_model_id_is_hard_coded_outside_hexa_infer() {
    // The id is the operator's: whatever follows the colon is passed on
    // as written, never mapped to a built-in id.
    assert_eq!(claude_text_model("claude-text:anything-at-all"), Some(Some("anything-at-all".into())));
    let families = ["claude-opus-", "claude-sonnet-", "claude-haiku-", "claude-fable-", "claude-3"];
    let mut found = Vec::new();
    for krate in ["hexa-cli", "hexa-exec", "hexa-core", "hexa-graph", "hexa-analysis", "hexa-git"] {
        let src = root().join(krate).join("src");
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let text = std::fs::read_to_string(&p).unwrap_or_default();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("");
                    if families.iter().any(|f| code.contains(f)) {
                        found.push(p.display().to_string());
                    }
                }
            }
        }
    }
    assert!(found.is_empty(), "a Claude model id outside hexa-infer: {found:?}");
}

/// S21: The model flag is exactly the suffix.
#[tokio::test]
async fn the_model_flag_is_exactly_the_suffix() {
    for suffix in ["a", "some-model", "vendor/model-1.5", "x:y"] {
        assert_eq!(claude_text_model(&format!("claude-text:{suffix}")), Some(Some(suffix.to_string())), "{suffix}");
        let ran = run(Some(suffix), vec![ok("hi")], vec![request("", vec![Message::user("hello")])]).await;
        assert_eq!(after(&ran.args[0], "--model"), Some(suffix));
        assert_eq!(ran.args[0].iter().filter(|a| *a == "--model").count(), 1);
    }
}

// ── §5 ────────────────────────────────────────────────────────────────────

/// S22: This ADR's tests are recorded red before the adapter exists, then sealed.
#[test]
fn this_adrs_tests_were_recorded_red_and_sealed() {
    let s = spec("ADR-2610270901");
    let red = s["red"].as_str().expect("a red commit is recorded");
    assert!(git(&["log", "-1", "--format=%s", red]).ends_with("(failing)"), "the red commit is the failing one");
    let file = "hexa-infer/tests/adr_2610270901_spec.rs";
    assert!(!git(&["ls-tree", "--name-only", red, "--", file]).is_empty(), "the tests are in the red commit");
    assert!(s["sealed"].get(file).is_some(), "and sealed: {}", s["sealed"]);
}

/// S23: The bootstrap exception is not available to other ADRs.
#[test]
fn the_bootstrap_exception_is_not_available_to_other_adrs() {
    // The next ADR to need tests gets them from `hexa spec tests`, whose
    // red commit is authored by the test writer, not by hand.
    let s = spec("ADR-2610270853");
    let red = s["red"].as_str().expect("ADR-2610270853's tests are generated and recorded red");
    assert_eq!(git(&["log", "-1", "--format=%an", red]), "hexa-spec", "generated by `hexa spec tests`, not by hand");
}
