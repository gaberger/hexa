//! ADR-2610270901 (as amended by ADR-2610281516 §1): `claude-text` serves
//! Claude as a tool-less completion. It runs with `--output-format json`
//! and reads its prompt on stdin.
//!
//! These tests sit beside the sealed `adr_2610270901_spec.rs`. They record
//! the prompt written to the CLI's stdin through the public
//! `ProcessSpawner` trait, so they rely on nothing private to the adapter.
#![allow(unused, clippy::all)]

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use hexa_core::ports::inference::{
    ContentBlock, IInferencePort, InferenceError, InferenceRequest, Message, Priority,
};
use hexa_infer::adapters::secondary::claude_code::{ProcessSpawner, SpawnedProcess};
use hexa_infer::adapters::secondary::ClaudeCodeInferenceAdapter;

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
struct RecordedCall {
    program: String,
    args: Vec<String>,
    input: Option<String>,
}

/// A spawner that records every invocation, including what is written to
/// stdin, and answers with a fixed stdout.
struct StdinRecorder {
    calls: Mutex<Vec<RecordedCall>>,
    stdout: String,
}

impl StdinRecorder {
    fn answering(stdout: &str) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::new(Vec::new()), stdout: stdout.to_string() })
    }

    fn record(&self, program: &str, args: &[String], input: Option<&str>) -> SpawnedProcess {
        self.calls.lock().unwrap().push(RecordedCall {
            program: program.to_string(),
            args: args.to_vec(),
            input: input.map(str::to_string),
        });
        SpawnedProcess { stdout: self.stdout.as_bytes().to_vec(), stderr: Vec::new(), exit_code: 0 }
    }

    fn recorded(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProcessSpawner for StdinRecorder {
    fn spawn<'l0, 'l1, 'l2, 'at>(
        &'l0 self,
        program: &'l1 str,
        args: &'l2 [String],
    ) -> BoxFut<'at, Result<SpawnedProcess, InferenceError>>
    where
        'l0: 'at,
        'l1: 'at,
        'l2: 'at,
        Self: 'at,
    {
        Box::pin(async move { Ok(self.record(program, args, None)) })
    }

    fn spawn_in<'l0, 'l1, 'l2, 'l3, 'at>(
        &'l0 self,
        program: &'l1 str,
        args: &'l2 [String],
        _dir: &'l3 Path,
    ) -> BoxFut<'at, Result<SpawnedProcess, InferenceError>>
    where
        'l0: 'at,
        'l1: 'at,
        'l2: 'at,
        'l3: 'at,
        Self: 'at,
    {
        Box::pin(async move { Ok(self.record(program, args, None)) })
    }

    fn spawn_in_with_input<'l0, 'l1, 'l2, 'l3, 'l4, 'at>(
        &'l0 self,
        program: &'l1 str,
        args: &'l2 [String],
        _dir: &'l3 Path,
        input: &'l4 str,
    ) -> BoxFut<'at, Result<SpawnedProcess, InferenceError>>
    where
        'l0: 'at,
        'l1: 'at,
        'l2: 'at,
        'l3: 'at,
        'l4: 'at,
        Self: 'at,
    {
        Box::pin(async move { Ok(self.record(program, args, Some(input))) })
    }
}

fn cli_json(result: &str) -> String {
    serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": result,
        "total_cost_usd": 0.0123
    })
    .to_string()
}

fn text_request(system: &str, messages: Vec<Message>) -> InferenceRequest {
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

struct Completed {
    result: Result<String, String>,
    calls: Vec<RecordedCall>,
}

async fn complete_through(model: Option<&str>, stdout: &str, request: InferenceRequest) -> Completed {
    let recorder = StdinRecorder::answering(stdout);
    let adapter = ClaudeCodeInferenceAdapter::text_with_spawner(model.map(str::to_string), recorder.clone());
    let result = adapter
        .complete(request)
        .await
        .map(|resp| {
            resp.content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<String>()
        })
        .map_err(|e| e.to_string());
    Completed { result, calls: recorder.recorded() }
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn only_call(c: &Completed) -> &RecordedCall {
    assert_eq!(c.calls.len(), 1, "exactly one CLI invocation: {:?}", c.calls);
    &c.calls[0]
}

fn stdin_of(call: &RecordedCall) -> &str {
    call.input.as_deref().unwrap_or_else(|| panic!("the prompt is written to stdin: {call:?}"))
}

/// S01: The bare `claude-text` model id is served by the Claude CLI adapter in text mode with the isolating flags.
#[tokio::test]
async fn bare_claude_text_runs_the_cli_with_isolating_flags_and_returns_the_json_result() {
    let done = complete_through(
        None,
        &cli_json("the answer from claude"),
        text_request("", vec![Message::user("hello")]),
    )
    .await;
    let call = only_call(&done).clone();
    let name = Path::new(&call.program).file_name().and_then(|n| n.to_str()).unwrap_or("");
    assert_eq!(name, "claude", "the Claude CLI is invoked: {}", call.program);
    assert!(call.args.iter().any(|a| a == "-p"), "-p missing: {:?}", call.args);
    assert_eq!(flag_value(&call.args, "--tools"), Some(""), "--tools \"\": {:?}", call.args);
    assert_eq!(flag_value(&call.args, "--setting-sources"), Some(""), "--setting-sources \"\": {:?}", call.args);
    for flag in ["--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence"] {
        assert!(call.args.iter().any(|a| a == flag), "{flag} missing: {:?}", call.args);
    }
    assert_eq!(flag_value(&call.args, "--output-format"), Some("json"), "{:?}", call.args);
    assert!(!call.args.iter().any(|a| a == "--dangerously-skip-permissions"), "{:?}", call.args);
    assert_eq!(done.result, Ok("the answer from claude".to_string()), "the completion is the JSON `result`");
}

/// S08: The system prompt and the user message are both delivered.
#[tokio::test]
async fn the_system_prompt_and_user_message_are_both_delivered() {
    let done = complete_through(
        None,
        &cli_json("ok"),
        text_request("You are the spec author.", vec![Message::user("Draft ADR-X")]),
    )
    .await;
    assert!(done.result.is_ok(), "{:?}", done.result);
    let call = only_call(&done);
    assert_eq!(flag_value(&call.args, "--system-prompt"), Some("You are the spec author."), "{:?}", call.args);
    assert!(stdin_of(call).contains("Draft ADR-X"), "stdin: {:?}", call.input);
}

/// S09: The system prompt is not dropped or folded into the prompt.
#[tokio::test]
async fn the_system_prompt_is_not_dropped_or_only_folded_into_the_prompt() {
    let system = "Answer only in haiku, system-marker-9137.";
    let user = "user-marker-4410 what is a port?";
    let done = complete_through(None, &cli_json("ok"), text_request(system, vec![Message::user(user)])).await;
    assert!(done.result.is_ok(), "{:?}", done.result);
    let call = only_call(&done);
    assert_eq!(
        call.args.iter().filter(|a| *a == "--system-prompt").count(),
        1,
        "exactly one --system-prompt flag: {:?}",
        call.args
    );
    assert_eq!(flag_value(&call.args, "--system-prompt"), Some(system), "{:?}", call.args);
    let input = stdin_of(call);
    assert!(input.contains(user), "the user message is the prompt: {input:?}");
}

/// S11: A multi-turn conversation is sent as the prompt, not just the last user message.
#[tokio::test]
async fn a_multi_turn_conversation_is_sent_whole_on_stdin() {
    let done = complete_through(
        None,
        &cli_json("ok"),
        text_request(
            "",
            vec![
                Message::user("first-turn-alpha A"),
                Message::assistant("second-turn-bravo B"),
                Message::user("third-turn-charlie C"),
            ],
        ),
    )
    .await;
    assert!(done.result.is_ok(), "{:?}", done.result);
    let input = stdin_of(only_call(&done)).to_string();
    for part in ["first-turn-alpha A", "second-turn-bravo B", "third-turn-charlie C"] {
        assert!(input.contains(part), "{part} missing from stdin: {input:?}");
    }
}

/// S12: Nothing in the request is lost.
#[tokio::test]
async fn nothing_in_a_claude_text_request_is_lost() {
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("", vec![("user", "only-user-m1")]),
        ("sys-one", vec![("user", "u-m2")]),
        ("multi\nline system prompt with \"quotes\"", vec![("user", "u-m3"), ("assistant", "a-m4"), ("user", "u-m5")]),
        ("", vec![("user", "u-m6"), ("assistant", "a-m7"), ("user", "u-m8"), ("assistant", "a-m9"), ("user", "u-m10")]),
        ("s", vec![("user", "unicode ✓ — m11")]),
    ];
    for (system, turns) in cases {
        let messages = turns
            .iter()
            .map(|(role, text)| if *role == "user" { Message::user(*text) } else { Message::assistant(*text) })
            .collect();
        let done = complete_through(Some("some-model"), &cli_json("ok"), text_request(system, messages)).await;
        assert!(done.result.is_ok(), "{system:?}: {:?}", done.result);
        let call = only_call(&done);
        if !system.is_empty() {
            assert_eq!(flag_value(&call.args, "--system-prompt"), Some(system), "{:?}", call.args);
        }
        let input = stdin_of(call);
        for (_, text) in &turns {
            assert!(input.contains(text), "{text:?} missing from stdin: {input:?}");
        }
    }
}
