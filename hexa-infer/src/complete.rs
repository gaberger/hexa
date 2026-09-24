//! One text completion, without a daemon in the path.
//!
//! `hexa-exec` used to POST `{model, messages, max_tokens}` to
//! `http://127.0.0.1:$HEXA_NEXUS_PORT/api/inference/complete` and read `{content}` back. The daemon
//! then called the provider. It added routing and logging to a call that already knew its model —
//! and it made the agent loop unable to run at all unless a control plane was up.
//!
//! This is that call as a function. Same inputs, same output, one process.
//!
//! The use case only: build the request, run it on whatever the [`Backends`] port provides,
//! record the spend, shape the reply. Which adapter serves a model is wiring, and lives in
//! [`crate::wiring`] — it was here, and made this use case import every adapter it might pick.

use hexa_core::domain::messages::{ContentBlock, Message, Role};
use hexa_core::domain::messages::StopReason;
use hexa_core::ports::inference::{InferenceRequest, Priority};

use crate::ports::Backends;


/// Tools, or a real error — never a silent empty list.
///
/// This was `.ok().unwrap_or_default()`, which turned any shape mismatch into "no tools". A model
/// given no tools does not fail: it invents a `{"tool": ...}` JSON-in-text convention, which
/// `extract_tool_uses` cannot see, so the loop reports "ended with no edit" and the MODEL takes the
/// blame for a bridge that dropped its arguments on the floor.
fn parse_tools(v: &serde_json::Value) -> Result<Vec<hexa_core::domain::tools::ToolDefinition>, String> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(v.clone())
        .map_err(|e| format!("inference: tools did not match ToolDefinition ({e})"))
}

/// Send one system+user turn and return the text of the reply.
///
/// The return is `Result<String, String>` rather than a typed error because every caller in
/// `hexa-exec` already funnels failures into a string it shows the operator, and widening that here
/// would be a change to the loop rather than to inference.
pub async fn complete_text_with(
    backends: &dyn Backends,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let request = InferenceRequest {
        model: model.to_string(),
        system_prompt: system.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user.to_string() }],
        }],
        tools: Vec::new(),
        max_tokens,
        // 0.0: this path asks for a precise edit or a structured reply, never for variety. It is
        // what the daemon route sent, kept rather than re-decided.
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    };

    let response = backends.complete(request).await.map_err(|e| e.to_string())?;

    backends.record_spend(model, response.input_tokens, response.output_tokens);

    // The daemon returned a flat `content` string. Concatenating the text blocks reproduces that
    // exactly for a reply with no tool use, which is all this path ever asks for.
    text_or_truncation(&response.content, response.stop_reason, model, max_tokens)
}

/// The reply's text, or an error when there is none because the model ran
/// out of budget (ADR-2609131835 §1).
///
/// A reasoning model emits its thinking first and its answer after, into one
/// budget. When the budget covers only the thinking the reply carries no
/// content at all, and `Ok("")` makes that indistinguishable from a model
/// that answered with nothing.
fn text_or_truncation(
    content: &[ContentBlock],
    stop_reason: StopReason,
    model: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let text: String = content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    if text.trim().is_empty() && stop_reason == StopReason::MaxTokens {
        return Err(format!(
            "{model} produced no answer within {max_tokens} tokens — it stopped at the limit, \
             which a reasoning model does when the budget covers its thinking but not its reply; \
             raise it with HEXA_REVIEW_MAX_TOKENS"
        ));
    }
    Ok(text)
}


/// The daemon's `/api/inference/complete` contract, as a function.
///
/// Takes and returns the same JSON the HTTP route did, so the ReAct loop's request building and
/// its `extract_tool_uses` parsing are untouched. That is deliberate: a phase should move the
/// boundary, not rewrite what sits either side of it. `hexa-exec` is 9,000 lines of loop that
/// works; the only thing wrong with it was the hop.
///
/// Tolerant about message content on the way in — the loop sends `"content": "text"` for the seed
/// and block arrays for tool results, and the route accepted both.
pub async fn complete_raw_with(backends: &dyn Backends, req: &serde_json::Value) -> Result<serde_json::Value, String> {
    let model = req.get("model").and_then(|v| v.as_str()).unwrap_or_default();
    if model.is_empty() {
        return Err("inference: request has no model".into());
    }

    let messages = req
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(message_from_json).collect::<Vec<_>>())
        .unwrap_or_default();

    let tools = parse_tools(req.get("tools").unwrap_or(&serde_json::Value::Null))?;

    let request = InferenceRequest {
        model: model.to_string(),
        system_prompt: req.get("system").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        messages,
        tools,
        max_tokens: u32::try_from(req.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096))
            .unwrap_or(u32::MAX),
        temperature: req.get("temperature").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    };

    // Which backend runs it — including sending a tool-using request to one
    // that carries tools and the transcript — is the port's decision.
    let response = backends.complete(request).await.map_err(|e| e.to_string())?;

    // ContentBlock's serde renames already produce Anthropic's {type: text|tool_use} shape, which
    // is exactly what extract_tool_uses reads. No hand-rolled mapping to drift.
    backends.record_spend(model, response.input_tokens, response.output_tokens);

    Ok(serde_json::json!({
        "content": serde_json::to_value(&response.content).map_err(|e| e.to_string())?,
        "model": response.model_used,
        "usage": {
            "input_tokens": response.input_tokens,
            "output_tokens": response.output_tokens,
        },
    }))
}

/// One message, from the loop's JSON. Content may be a bare string or a block array.
fn message_from_json(v: &serde_json::Value) -> Message {
    let role = match v.get("role").and_then(|r| r.as_str()) {
        Some("assistant") => Role::Assistant,
        _ => Role::User,
    };
    let content = match v.get("content") {
        Some(serde_json::Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(arr @ serde_json::Value::Array(_)) => serde_json::from_value(arr.clone())
            .unwrap_or_else(|_| vec![ContentBlock::Text { text: arr.to_string() }]),
        _ => Vec::new(),
    };
    Message { role, content }
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    /// The shape `direct_react::curated_schema` actually emits.
    fn curated_like() -> serde_json::Value {
        serde_json::json!([
            { "name": "repo_read", "description": "Read a file",
              "input_schema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] } },
            { "name": "propose_edit", "description": "Apply the edit and run evidence",
              "input_schema": { "type": "object", "properties": { "mode": { "type": "string" } }, "required": [] } }
        ])
    }

    #[test]
    fn the_curated_schema_survives_the_bridge() {
        let tools = parse_tools(&curated_like()).expect("curated schema must deserialize");
        assert_eq!(tools.len(), 2, "both tools must reach the model");
        assert_eq!(tools[0].name, "repo_read");
    }

    /// The bug this replaced: `.ok().unwrap_or_default()` turned a shape mismatch into an EMPTY
    /// tool list. The model then has no tools, invents a `{"tool": …}` JSON-in-text convention,
    /// and `extract_tool_uses` finds nothing — so the loop reports "ended with no edit" and it
    /// reads as the model being incapable. Observed with devstral-small-2:24b, which emits a
    /// perfectly good structured tool_call when it is actually given tools.
    #[test]
    fn a_malformed_tool_is_an_error_not_a_silent_empty_list() {
        // `input_schema` missing its required `type`.
        let bad = serde_json::json!([{ "name": "x", "description": "d", "input_schema": { "properties": {} } }]);
        assert!(parse_tools(&bad).is_err(), "a shape mismatch must be reported, never swallowed");
    }

    #[test]
    fn absent_tools_are_legitimately_empty() {
        assert_eq!(parse_tools(&serde_json::Value::Null).unwrap().len(), 0);
    }
}

#[cfg(test)]
mod truncated_tests {
    use super::text_or_truncation;
    use hexa_core::domain::messages::{ContentBlock, StopReason};

    fn text(t: &str) -> Vec<ContentBlock> {
        vec![ContentBlock::Text { text: t.to_string() }]
    }

    /// ADR-2609131835 §1: no text plus a max-tokens stop is a truncation,
    /// named, with the knob to turn. This is the reply gpt-oss-120b sent.
    #[test]
    fn no_text_and_a_max_tokens_stop_is_an_error_that_names_the_budget() {
        let err = text_or_truncation(&[], StopReason::MaxTokens, "openai/gpt-oss-120b", 4096).unwrap_err();
        assert!(err.contains("openai/gpt-oss-120b"), "{err}");
        assert!(err.contains("4096"), "names the budget it hit: {err}");
        assert!(err.contains("HEXA_REVIEW_MAX_TOKENS"), "names the knob: {err}");

        // Whitespace-only is no answer either.
        assert!(text_or_truncation(&text("  \n "), StopReason::MaxTokens, "m", 10).is_err());
    }

    /// An ordinary stop with no text is a real, if empty, answer — the
    /// distinction ADR-2609131646 drew, kept here.
    #[test]
    fn no_text_and_an_ordinary_stop_is_an_empty_answer_not_an_error() {
        assert_eq!(text_or_truncation(&[], StopReason::EndTurn, "m", 4096).unwrap(), "");
    }

    /// Text is returned whole under either stop: a truncated reply that
    /// still said something is that something.
    #[test]
    fn text_is_returned_under_either_stop() {
        assert_eq!(text_or_truncation(&text("{\"findings\":[]}"), StopReason::EndTurn, "m", 4096).unwrap(), "{\"findings\":[]}");
        assert_eq!(text_or_truncation(&text("partial"), StopReason::MaxTokens, "m", 4096).unwrap(), "partial");
    }
}
