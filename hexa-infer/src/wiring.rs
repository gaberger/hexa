//! Which backend serves a model, wired — hexa-infer's composition root for inference.
//!
//! Reads the operator's registry, resolves an endpoint's key, and constructs the adapter that
//! serves the model. This lived in `complete.rs`, which made a use case import every adapter it
//! might choose. The use case now asks the [`Backends`] port; this is the one implementation.

use std::time::Duration;

use async_trait::async_trait;
use hexa_core::ports::inference::{IInferencePort, InferenceError, InferenceRequest, InferenceResponse};

use crate::adapters::secondary::{
    AnthropicAdapter, ClaudeCodeInferenceAdapter, OllamaInferenceAdapter, OpenAiCompatAdapter,
};
use crate::endpoint::Endpoint;
use crate::ports::Backends;
use crate::registry;

/// The registry-backed [`Backends`]: what `hexa_infer::complete_text` and `complete_raw` run on.
pub struct RegistryBackends;

#[async_trait]
impl Backends for RegistryBackends {
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        // TOOLS MEAN /api/chat. The OllamaInferenceAdapter posts to /api/generate, which has no
        // `tools` parameter and whose `collapse_prompt` sends only the LAST USER MESSAGE — so a
        // multi-step loop got neither its tools nor its transcript. It did not fail loudly: the model
        // invented a JSON-in-text convention that `extract_tool_uses` cannot see, and the loop
        // reported "ended with no edit" as though the model were incapable.
        if !request.tools.is_empty() && !request.model.to_lowercase().starts_with("claude") {
            return crate::adapters::secondary::ollama_chat::chat(
                &crate::local_provider().base_url(),
                Duration::from_secs(600),
                request,
            )
            .await;
        }
        let model = request.model.clone();
        adapter_for(&model).complete(request).await
    }

    fn record_spend(&self, model: &str, input_tokens: u64, output_tokens: u64) {
        crate::spend::record(model, input_tokens, output_tokens);
    }
}

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
