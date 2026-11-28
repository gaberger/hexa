//! A failed `claude-text` call that wrote no JSON reason and nothing to
//! stderr still says what it printed (ADR-2611221646 §1): "exited with N:"
//! is never followed by nothing.

use std::sync::Arc;

use hexa_infer::adapters::secondary::claude_code::testing::{MockProcessSpawner, MockResponse};
use hexa_infer::adapters::secondary::claude_code::{ClaudeCodeInferenceAdapter, SpawnedProcess};

async fn error_for(exit_code: i32, stdout: &str, stderr: &str) -> String {
    let proc = SpawnedProcess { stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec(), exit_code };
    let mock = Arc::new(MockProcessSpawner::with_responses([MockResponse::Ok(proc)]));
    let adapter = ClaudeCodeInferenceAdapter::text_with_spawner(None, mock);
    match adapter.complete_with_cost(Default::default()).await {
        Ok(_) => panic!("a non-zero exit succeeded"),
        Err(e) => e.to_string(),
    }
}

fn after_colon(msg: &str, code: i32) -> String {
    let marker = format!("exited with {code}:");
    let (_, rest) = msg.split_once(&marker).unwrap_or_else(|| panic!("no {marker:?} in {msg:?}"));
    rest.trim().to_string()
}

#[tokio::test]
async fn plain_stdout_and_no_stderr_is_reported() {
    let msg = error_for(1, "  Invalid API key · Please run /login\n", "").await;
    assert!(msg.contains("Invalid API key · Please run /login"), "stdout is missing from: {msg:?}");
    assert!(!msg.contains("  Invalid"), "stdout was not trimmed: {msg:?}");
}

#[tokio::test]
async fn json_without_a_reason_and_no_stderr_is_not_empty() {
    let msg = error_for(2, r#"{"type":"result","is_error":true}"#, "").await;
    assert!(!after_colon(&msg, 2).is_empty(), "nothing follows the colon: {msg:?}");
}

#[tokio::test]
async fn no_output_at_all_says_so() {
    let msg = error_for(3, "", "").await;
    assert!(!after_colon(&msg, 3).is_empty(), "nothing follows the colon: {msg:?}");
}

#[tokio::test]
async fn long_stdout_is_shortened() {
    let long = "x".repeat(10_000);
    let msg = error_for(4, &long, "").await;
    assert!(msg.contains("xxxx"), "stdout is missing: {msg:?}");
    assert!(msg.len() < 2_000, "stdout was not shortened ({} bytes)", msg.len());
}
