//! One text completion, without a daemon in the path.
//!
//! `hexa-exec` used to POST `{model, messages, max_tokens}` to
//! `http://127.0.0.1:$HEXA_NEXUS_PORT/api/inference/complete` and read `{content}` back. The daemon
//! then called the provider. It added routing and logging to a call that already knew its model —
//! and it made the agent loop unable to run at all unless a control plane was up.
//!
//! This is that call as a function. Same inputs, same output, one process.

use hexa_core::domain::messages::{ContentBlock, Message, Role};
use hexa_core::domain::messages::StopReason;
use hexa_core::ports::inference::{IInferencePort, InferenceRequest, Priority};

use crate::adapters::{
    AnthropicAdapter, ClaudeCodeInferenceAdapter, OllamaInferenceAdapter, OpenAiCompatAdapter,
};
use crate::endpoint::Endpoint;
use crate::registry;

/// Which backend serves a model id.
///
/// **The registry decides.** `~/.hexa/inference-servers.json` is asked first,
/// and the matched entry's provider family picks the adapter. Only when no
/// registered endpoint advertises the model does the prefix test below apply.
///
/// It used to be the prefix test alone: `claude*` to `claude -p`, everything
/// else to the local runtime. That made the set of reachable providers a
/// property of the source code — an operator could register an
/// OpenAI-compatible host or an OpenRouter key with `hexa inference add`, see
/// it in `hexa inference list`, and still have every request to it sent to
/// local ollama, which 404s on an id it has never heard of. Founding goal G1
/// requires that adding or retiring a provider is a configuration change, not
/// a refactor.
///
/// The prefix fallback is kept for the no-registry case, and it is the same
/// reasoning weave's tier router settled on: ollama tags have no common form,
/// so a rule like "contains a colon" misroutes anything vendor-prefixed.
fn adapter_for(model: &str) -> Box<dyn IInferencePort> {
    if let Some(endpoint) = registry::serving(&registry::load(), model) {
        return adapter_for_endpoint(&endpoint);
    }
    if model.to_lowercase().starts_with("claude") {
        Box::new(ClaudeCodeInferenceAdapter::new(None))
    } else {
        Box::new(OllamaInferenceAdapter::new(None))
    }
}

/// The adapter for one registered endpoint.
///
/// The API key is the *name* of an environment variable, resolved here. The
/// daemon kept keys in a SpacetimeDB vault and resolved references at dispatch
/// time under a 3-second timeout — a distributed system standing in for
/// `std::env::var`, for a single-user tool on one machine.
fn adapter_for_endpoint(endpoint: &Endpoint) -> Box<dyn IInferencePort> {
    let key = resolve_key(endpoint);
    match endpoint.provider.to_ascii_lowercase().as_str() {
        // The local runtime streams NDJSON of its own; its adapter speaks that.
        "ollama" => Box::new(OllamaInferenceAdapter::new(Some(endpoint.url.clone()))),
        "anthropic" => Box::new(AnthropicAdapter::new(key, endpoint.model.clone())),
        "claude-code" | "claude_code" => Box::new(ClaudeCodeInferenceAdapter::new(None)),
        // openrouter, openai, openai_compat, vllm, llama-cpp and anything else
        // registered: all of them speak the OpenAI chat-completions shape.
        _ => Box::new(OpenAiCompatAdapter::new(
            key,
            endpoint.url.clone(),
            endpoint.model.clone(),
        )),
    }
}

/// Read an endpoint's key out of the environment variable it names.
fn resolve_key(endpoint: &Endpoint) -> String {
    if endpoint.secret_key.is_empty() {
        return String::new();
    }
    if let Ok(v) = std::env::var(&endpoint.secret_key) {
        if !v.is_empty() {
            return v;
        }
    }
    // A key that lives in a file is the normal case, not an exotic one
    // (ADR-2609131811). Read by the tool, into one header; never printed.
    for path in key_files() {
        if let Some(v) = value_in_env_file(&path, &endpoint.secret_key) {
            return v;
        }
    }
    // A literal key in the field rather than a variable name: tolerated,
    // because a hand-edited registry is a real thing operators produce.
    if endpoint.secret_key.starts_with("sk-") {
        return endpoint.secret_key.clone();
    }
    tracing::warn!(
        endpoint = %endpoint.id,
        variable = %endpoint.secret_key,
        looked_in = %key_files().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "),
        "no value for this endpoint's key reference, in the environment or any env file"
    );
    String::new()
}

/// Where a named key may live, in the order they are consulted
/// (ADR-2609131811 §1). A path that does not exist is simply skipped.
fn key_files() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("HEXA_ENV_FILE") {
        if !p.is_empty() {
            out.push(std::path::PathBuf::from(p));
        }
    }
    // Same way `registry::registry_path` finds home: no new dependency.
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        out.push(home.join(".hexa/.env"));
    }
    let root = std::env::var("HEXA_PROJECT_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    out.push(root.join(".env"));
    out
}

/// `name`'s value in an env file, if the file has one.
fn value_in_env_file(path: &std::path::Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_env_value(&text, name)
}

/// The ordinary env-file format: `KEY=value` per line, an `export ` prefix
/// tolerated, surrounding quotes stripped, blanks and `#` comments ignored.
/// A line that is not a assignment is skipped rather than guessed at.
fn parse_env_value(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != name {
            continue;
        }
        let v = value.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
            .unwrap_or(v);
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}


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
pub async fn complete_text(
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

    let response = adapter_for(model)
        .complete(request)
        .await
        .map_err(|e| e.to_string())?;

    crate::spend::record(model, response.input_tokens, response.output_tokens);

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

#[cfg(test)]
mod tests {
    /// The classification rule, pinned.
    ///
    /// An exact PREFIX, not a substring: a local model called `my-claude-clone` is not Anthropic's,
    /// and routing it there fails with a message about a model that does exist elsewhere — the
    /// confusing kind of wrong.
    fn is_claude(model: &str) -> bool {
        model.to_lowercase().starts_with("claude")
    }

    #[test]
    fn claude_ids_route_to_claude() {
        assert!(is_claude("claude-sonnet-4-6"));
        assert!(is_claude("Claude-Haiku-4-5"));
    }

    #[test]
    fn local_ids_do_not_and_neither_does_a_lookalike() {
        assert!(!is_claude("qwen2.5-coder:14b"));
        assert!(!is_claude("gemma4-12b:latest"));
        assert!(!is_claude("my-claude-clone"), "substring matching would send this to Anthropic");
    }
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
pub async fn complete_raw(req: &serde_json::Value) -> Result<serde_json::Value, String> {
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

    // TOOLS MEAN /api/chat. The OllamaInferenceAdapter posts to /api/generate, which has no
    // `tools` parameter and whose `collapse_prompt` sends only the LAST USER MESSAGE — so a
    // multi-step loop got neither its tools nor its transcript. It did not fail loudly: the model
    // invented a JSON-in-text convention that `extract_tool_uses` cannot see, and the loop
    // reported "ended with no edit" as though the model were incapable.
    let response = if !request.tools.is_empty() && !model.to_lowercase().starts_with("claude") {
        crate::adapters::ollama_chat::chat(
            &crate::local_provider().base_url(),
            std::time::Duration::from_secs(600),
            request,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        adapter_for(model)
            .complete(request)
            .await
            .map_err(|e| e.to_string())?
    };

    // ContentBlock's serde renames already produce Anthropic's {type: text|tool_use} shape, which
    // is exactly what extract_tool_uses reads. No hand-rolled mapping to drift.
    crate::spend::record(model, response.input_tokens, response.output_tokens);

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
mod env_file_tests {
    use super::parse_env_value;

    /// ADR-2609131811 §2: the ordinary format, and nothing invented.
    #[test]
    fn the_ordinary_env_file_format_parses() {
        let text = "\
# a comment
EMPTY=

PLAIN=abc123
export EXPORTED=def456
QUOTED=\"ghi789\"
SINGLE='jkl012'
  SPACED  =  mno345
NOT AN ASSIGNMENT
TRAILING=xyz
";
        assert_eq!(parse_env_value(text, "PLAIN").as_deref(), Some("abc123"));
        assert_eq!(parse_env_value(text, "EXPORTED").as_deref(), Some("def456"), "an export prefix is tolerated");
        assert_eq!(parse_env_value(text, "QUOTED").as_deref(), Some("ghi789"), "double quotes stripped");
        assert_eq!(parse_env_value(text, "SINGLE").as_deref(), Some("jkl012"), "single quotes stripped");
        assert_eq!(parse_env_value(text, "SPACED").as_deref(), Some("mno345"), "whitespace around the name and value");
        assert_eq!(parse_env_value(text, "TRAILING").as_deref(), Some("xyz"));
        assert_eq!(parse_env_value(text, "EMPTY"), None, "an empty value is no value");
        assert_eq!(parse_env_value(text, "MISSING"), None);
        assert_eq!(parse_env_value(text, "NOT"), None, "a line that is not an assignment is skipped");
    }

    /// A name that is a prefix of another is not that other.
    #[test]
    fn a_name_matches_exactly_never_by_prefix() {
        let text = "TT_STUDIO_GATEWAY_KEY_OLD=stale\nTT_STUDIO_GATEWAY_KEY=current\n";
        assert_eq!(parse_env_value(text, "TT_STUDIO_GATEWAY_KEY").as_deref(), Some("current"));
        assert_eq!(parse_env_value(text, "TT_STUDIO").as_deref(), None);
    }

    /// The first assignment wins, as a shell would take it on first read.
    #[test]
    fn the_first_assignment_wins() {
        assert_eq!(parse_env_value("K=first\nK=second\n", "K").as_deref(), Some("first"));
    }

    #[test]
    fn an_empty_or_malformed_file_yields_nothing_rather_than_panicking() {
        assert_eq!(parse_env_value("", "K"), None);
        assert_eq!(parse_env_value("\n\n#only comments\n", "K"), None);
        assert_eq!(parse_env_value("=novalue\n", "K"), None);
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
