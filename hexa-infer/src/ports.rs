//! What hexa-infer's use cases need from the outside, as contracts.
//!
//! [`Endpoint`] is the registry's record of one backend — its URL, provider and status: the data
//! the registry adapter hands the router, not business domain, so it is a port-level type.

use async_trait::async_trait;

use crate::endpoint::Endpoint;
use hexa_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};

/// Whatever serves a model. The use case builds a request and asks; which adapter answers is
/// wiring ([`crate::wiring`]).
#[async_trait]
pub trait Backends: Send + Sync {
    /// Run `request` on the backend that serves `request.model`.
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError>;

    /// Record what a completion cost.
    fn record_spend(&self, model: &str, input_tokens: u64, output_tokens: u64);
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
    /// `local`, `api`, `registered` or `frontier`.
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
