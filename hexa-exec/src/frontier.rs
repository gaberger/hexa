//! What every `claude -p` call hexa makes has in common: the budget is
//! asked first, the call runs with `--output-format json`, and the answer's
//! own usage and `total_cost_usd` are recorded before the text is returned.
//! Before this, the frontier path ran for its text alone and its cost was
//! never written anywhere.

use serde_json::Value;

/// The arguments that make `claude -p` report its usage.
pub const OUTPUT_JSON: [&str; 2] = ["--output-format", "json"];

/// Refuse to spend when the project's daily budget is reached.
pub fn budget_check() -> Result<(), String> {
    hexa_infer::spend::budget_check()
}

/// The text of a `claude -p --output-format json` answer, with its usage
/// recorded under `source`. Plain text is returned as is: an older CLI, or
/// a run that printed something else, still yields its output.
pub fn take_answer(stdout: &str, source: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        return stdout.to_string();
    };
    let input = v.get("usage").and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0);
    let output = v.get("usage").and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0);
    let cost = v.get("total_cost_usd").and_then(Value::as_f64);
    hexa_infer::spend::record_with("claude-code", input, output, cost, source);
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
