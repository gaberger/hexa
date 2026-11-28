//! What hexa-infer's use cases need from the outside, as contracts.
//!
//! [`Endpoint`] is the registry's record of one backend — its URL, provider and status: the data
//! the registry adapter hands the router, not business domain, so it is a port-level type.

use async_trait::async_trait;

use crate::endpoint::Endpoint;
use hexa_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};

/// The environment variable naming a tool-less completion executable. The
/// command adapter runs it; discovery reports it as a path.
pub const COMMAND_ENV: &str = "HEXA_COMPLETION_COMMAND";

/// The same route under the name the spec pipeline's tests give it.
pub const AGENT_ENV: &str = "HEXA_AGENT";

/// Whatever serves a model. The use case builds a request and asks; which adapter answers is
/// wiring ([`crate::wiring`]).
#[async_trait]
pub trait Backends: Send + Sync {
    /// Run `request` on the backend that serves `request.model`.
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError>;

    /// Record what a completion cost.
    fn record_spend(&self, model: &str, input_tokens: u64, output_tokens: u64);

    /// Run `request` and say what it cost (ADR-2610281516 §1). Defaults to
    /// the response's tokens and no cost; a backend that is told the cost
    /// overrides it.
    async fn complete_with_usage(&self, request: InferenceRequest) -> Result<(InferenceResponse, Usage), InferenceError> {
        let response = self.complete(request).await?;
        let usage = Usage::of(&response);
        Ok((response, usage))
    }

    /// Record a completion's whole usage. Defaults to its tokens.
    fn record_usage(&self, model: &str, usage: &Usage) {
        self.record_spend(model, usage.input_tokens, usage.output_tokens);
    }

    /// Where tool-less replies are kept, if anywhere (§2).
    fn cache(&self) -> Option<&dyn CompletionCache> {
        None
    }

    /// Record a completion answered from the cache: source `cache`, no tokens.
    fn record_cache_hit(&self, model: &str) {
        let _ = model;
    }
}

// ── The local runtime ────────────────────────────────────

/// The local inference server hexa bootstraps against.
///
/// One value, because this is a single-binary tool for one machine. When hexa
/// needs to support a second local server, this becomes an enum and every
/// caller below keeps compiling — which is the point of naming it here rather
/// than spelling it out in six files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalProvider {
    /// For display: "Ollama".
    pub display_name: &'static str,
    /// The executable on `PATH`, and the process name to match.
    pub binary: &'static str,
    /// The port it listens on by default.
    pub default_port: u16,
    /// The subcommand that starts it in the foreground.
    pub serve_arg: &'static str,
    /// How to install it, per platform.
    pub install_macos: &'static str,
    pub install_linux: &'static str,
    /// The environment variable that overrides where it listens.
    pub host_env: &'static str,
}

/// The local provider hexa targets.
pub const fn local_provider() -> LocalProvider {
    LocalProvider {
        display_name: "Ollama",
        binary: "ollama",
        default_port: 11434,
        serve_arg: "serve",
        install_macos: "brew install ollama",
        install_linux: "curl https://ollama.ai/install.sh | sh",
        host_env: "OLLAMA_HOST",
    }
}

/// The frontier CLI's executable on `PATH`. hexa-cli asks for it here, as it
/// asks for [`local_provider`], so no file outside hexa-infer names it
/// (founding goal G1).
pub const fn frontier_cli() -> &'static str {
    "claude"
}

/// The provider name and executable of the Codex CLI (ADR-2611231735), asked for
/// here so no file outside hexa-infer names it (founding goal G1).
pub const fn codex_cli() -> &'static str {
    "codex"
}

impl LocalProvider {
    /// `http://127.0.0.1:<port>` — the base URL for a default install.
    pub fn default_base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.default_port)
    }

    /// The install hint for the platform this binary was built for.
    pub fn install_hint(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            self.install_macos
        } else {
            self.install_linux
        }
    }
}

// ── Discovery ────────────────────────────────────────────

/// One place a model can be reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// `local`, `api`, `registered`, `command` or `frontier`.
    pub kind: &'static str,
    pub name: String,
    /// The address, path or model; never a secret.
    pub detail: String,
    /// What told us: an environment variable, a file, or PATH.
    pub via: String,
    /// `Some(true)` answered a probe, `Some(false)` did not, `None` not probed.
    pub reachable: Option<bool>,
    /// The models this path is known to serve. Empty means *not
    /// enumerated*, never *none* (ADR-2609131617 §1).
    pub models: Vec<String>,
}

impl Found {
    /// A path counts unless a probe said no.
    pub fn open(&self) -> bool {
        self.reachable != Some(false)
    }
}

/// What a configured tier's model resolves to among the discovered paths
/// (ADR-2609131617 §3).
#[derive(Debug, Clone, PartialEq)]
pub enum Coverage {
    /// A reachable path lists it; the name is that path's.
    Served(String),
    /// Every reachable path was enumerated and none lists it.
    NotServed,
    /// A reachable path could not be enumerated, so nothing can be said.
    /// The name is that path's.
    Unverified(String),
}

/// What inference is reachable from here.
pub trait Discovery: Send + Sync {
    fn discover(&self) -> Vec<Found>;
}

// ── The endpoint registry ────────────────────────────────

/// The operator's registered inference backends (`~/.hexa/inference-servers.json`).
pub trait EndpointRegistry: Send + Sync {
    /// Where the registry lives.
    fn path(&self) -> std::path::PathBuf;
    fn load(&self) -> Vec<Endpoint>;
    fn save(&self, endpoints: &[Endpoint]) -> Result<(), String>;
    /// Add or replace one endpoint by id.
    fn upsert(&self, endpoint: Endpoint) -> Result<(), String>;
    /// Whether an endpoint with this id was there to remove.
    fn remove(&self, id: &str) -> Result<bool, String>;
}

// ── Context compression (ADR-2611050000, amendment) ──────

/// The one context budget, in estimated tokens (chars / 4), that both
/// compression layers share.
///
/// The ReAct loop (`hexa-exec`'s `direct_react`) compresses mechanically every
/// step and makes one cheap summarization call only when the result is still
/// over this. [`crate::complete::complete_raw_with`] never compresses a request
/// at or below it, and never compresses a request whose caller says it already
/// did (see [`COMPRESSION_KEY`]). It was two numbers — 12 000 in the loop and
/// 4 000 here — and the smaller one undid the larger on most steps.
pub const COMPRESSION_THRESHOLD_TOKENS: u64 = 12_000;

/// The request-JSON key a caller uses to say it owns compression.
///
/// ```json
/// "compression": { "already_compressed": true, "layer": "mechanical",
///                  "original_tokens": 30000, "compressed_tokens": 9000,
///                  "summarization_calls": 0 }
/// ```
///
/// With `already_compressed: true`, `complete_raw_with` does not run its
/// compressor, and reports the caller's figures as the reply's telemetry, so
/// the telemetry names the layer that actually acted.
pub const COMPRESSION_KEY: &str = "compression";

/// The estimated tokens of a transcript: every block, tool results and tool
/// inputs included, at chars / 4 — the same rough measure the loop uses.
///
/// `Message::text_content` counts only text blocks, so a transcript of tool
/// results estimated as nearly empty however large it was.
pub fn estimate_message_tokens(messages: &[hexa_core::domain::messages::Message]) -> u64 {
    use hexa_core::domain::messages::ContentBlock;
    let chars: usize = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .map(|b| match b {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::ToolUse { input, .. } => input.to_string().len(),
            ContentBlock::ToolResult { content, .. } => content.len(),
        })
        .sum();
    chars as u64 / 4
}

/// A service that compresses conversation history to fit within
/// model context windows while preserving semantic signal.
#[async_trait]
pub trait PromptCompressor: Send + Sync {
    /// Compresses the provided messages for the given model.
    ///
    /// Returns the compressed messages and a map of telemetry metadata
    /// (e.g. compression ratio, tokens saved). An implementation that cannot
    /// summarize should fall back rather than fail; an `Err` is still never
    /// allowed to fail the caller's completion.
    async fn compress(&self, model: &str, messages: Vec<hexa_core::domain::messages::Message>) -> Result<(Vec<hexa_core::domain::messages::Message>, std::collections::HashMap<String, String>), String>;

    /// The estimate above which [`Self::compress`] is called for `model`.
    /// Never below [`COMPRESSION_THRESHOLD_TOKENS`].
    fn threshold_tokens(&self, model: &str) -> u64 {
        let _ = model;
        COMPRESSION_THRESHOLD_TOKENS
    }
}

/// `base_url`, with the environment reader injected. The local server's
/// address comes from `host_env` (`HEXA_OLLAMA_HOST`), then `OLLAMA_HOST`,
/// then the default port; a bare `host:port` gets `http://`.
pub fn base_url_with(p: &LocalProvider, env: &dyn Fn(&str) -> Option<String>) -> String {
    // Trim before deciding: a blank override falls back to the default,
    // and "  " once became "http://".
    let raw = env(p.host_env)
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| env("OLLAMA_HOST").map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| !v.is_empty()));
    match raw {
        Some(v) => {
            if v.starts_with("http://") || v.starts_with("https://") { v } else { format!("http://{v}") }
        }
        None => p.default_base_url(),
    }
}

/// What one completion cost, as its provider reported it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: Option<f64>,
}

impl Usage {
    /// A response's tokens, with no cost known.
    pub fn of(response: &InferenceResponse) -> Usage {
        Usage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            cache_read_tokens: response.cache_read_tokens,
            cost_usd: None,
        }
    }
}

/// Replies to tool-less completions, by the sha256 of model, system and user
/// prompt (ADR-2610281516 §2).
pub trait CompletionCache: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn put(&self, key: &str, reply: &str);
}

/// The cache key of one tool-less completion.
pub fn completion_key(model: &str, system: &str, user: &str) -> String {
    use sha2::{Digest, Sha256};
    // Each field length-prefixed, so text moving across a boundary changes
    // the key: ("ab","c") and ("a","bc") differ.
    let mut h = Sha256::new();
    for part in [model, system, user] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
