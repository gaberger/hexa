//! What every `claude -p` call hexa makes has in common: the budget is
//! asked first, the call runs with `--output-format json`, and the answer's
//! own usage and `total_cost_usd` are recorded before the text is returned.
//! Before this, the frontier path ran for its text alone and its cost was
//! never written anywhere.
//!
//! The model recorded is the one the answer itself names (ADR-2609160300 §4),
//! read out of the JSON envelope by `spend::model_from_frontier_json`. The
//! earlier value, `"claude-code"`, was the name of a code path, not of a
//! model: it could not tell Opus from Haiku, so a spend log full of it could
//! not say what the money was spent on, and no per-model total or price
//! comparison could be recovered from it afterwards. An answer that names no
//! model is recorded as unknown rather than as the path that carried it.

use serde_json::Value;

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;

use crate::ports::{Frontier, FrontierError};

/// The arguments that make `claude -p` report its usage.
const OUTPUT_JSON: [&str; 2] = ["--output-format", "json"];

fn claude_binary() -> String {
    std::env::var("HEXA_CLAUDE_BINARY").unwrap_or_else(|_| "claude".to_string())
}

/// [`Frontier`] on the operator's logged-in `claude` CLI. Both loops that
/// used it — the do-loop's frontier fallback and the adversarial harness —
/// spawned it themselves, identically; this is that spawn, once.
pub struct ClaudeCli;

#[async_trait]
impl Frontier for ClaudeCli {
    async fn run(&self, prompt: &str, cwd: &Path, timeout: Duration, source: &str) -> Result<String, FrontierError> {
        // Refuse to spend when the project's daily budget is reached.
        hexa_infer::spend_budget_check().map_err(FrontierError::OverBudget)?;
        let binary = claude_binary();
        let fut = tokio::process::Command::new(&binary)
            .arg("-p")
            .args(OUTPUT_JSON)
            // hexa's own prompt. The project's hooks run inside this claude and must
            // not treat it as a person's work: `route` once drafted workplans from the
            // harden reviewer prompts. `hexa hook` returns early when this is set.
            .env("HEXA_INTERNAL", "1")
            .arg("--dangerously-skip-permissions")
            .arg(prompt)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();
        match tokio::time::timeout(timeout, fut).await {
            // Its usage and cost are recorded either way: they were spent.
            Ok(Ok(o)) => Ok(take_answer(&String::from_utf8_lossy(&o.stdout), source)),
            Ok(Err(e)) => Err(FrontierError::Spawn(format!("{binary}: {e}"))),
            Err(_) => Err(FrontierError::Timeout(timeout)),
        }
    }
}

/// The text of a `claude -p --output-format json` answer, with its usage
/// recorded under `source`. Plain text is returned as is: an older CLI, or
/// a run that printed something else, still yields its output.
fn take_answer(stdout: &str, source: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        return stdout.to_string();
    };
    let input = v.get("usage").and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0);
    let output = v.get("usage").and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0);
    let cost = v.get("total_cost_usd").and_then(Value::as_f64);
    let model = hexa_infer::spend::model_from_frontier_json(&v);
    hexa_infer::spend::record_with(&model, input, output, cost, source);
    v.get("result").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stdout.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_text_comes_out_of_the_json_envelope() {
        let out = r#"{"type":"result","is_error":false,"result":"the review","total_cost_usd":0.0123,"usage":{"input_tokens":1200,"output_tokens":300}}"#;
        assert_eq!(take_answer(out, "test"), "the review");
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(take_answer("not json\n", "test"), "not json\n");
    }
}
