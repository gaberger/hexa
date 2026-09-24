//! Inference for hexa, with no daemon in the path.
//!
//! `hexa do` reached its model through an HTTP call to a locally-running nexus, which then called
//! the provider. The agent loop therefore could not run unless a control plane was up — the daemon
//! mediated a call it added nothing to. This crate is that call, as a library.
//!
//! Everything here speaks `hexa_core::ports::inference::IInferencePort`. G1 requires that no
//! consumer names a provider, and this is the enforcement point: callers hold the trait, and which
//! adapter is behind it is a composition decision.

pub mod adapters;
pub mod complete;
pub mod endpoint;
pub mod local_provider;
pub mod ports;
pub mod reach;
pub mod discover;
pub mod registry;
pub mod spend;
pub mod spend_report;
pub mod tiers;
pub mod wiring;

pub use adapters::secondary::{
    AnthropicAdapter, ClaudeCodeInferenceAdapter, OllamaInferenceAdapter, OpenAiCompatAdapter,
};
pub use endpoint::Endpoint;
pub use tiers::{react_models, react_models_in_config, tier_model};
pub use local_provider::configured_tiers;
pub use ports::{local_provider, LocalProvider};
pub use discover::enumerate_models;
pub use ports::{Coverage, Found};
pub use reach::{served_models, serves};

/// Send one system+user turn and return the text of the reply, on the registry-backed backends.
/// See [`complete::complete_text_with`].
pub async fn complete_text(model: &str, system: &str, user: &str, max_tokens: u32) -> Result<String, String> {
    complete::complete_text_with(&wiring::RegistryBackends, model, system, user, max_tokens).await
}

/// The loop's JSON completion contract, on the registry-backed backends.
/// See [`complete::complete_raw_with`].
pub async fn complete_raw(req: &serde_json::Value) -> Result<serde_json::Value, String> {
    complete::complete_raw_with(&wiring::RegistryBackends, req).await
}

/// Every row in the spend log.
pub fn spend_entries() -> Vec<serde_json::Value> {
    spend::entries()
}

/// Where the spend log lives.
pub fn spend_log_path() -> std::path::PathBuf {
    spend::home().join("inference-log.jsonl")
}

/// `inference.budget_usd_per_day`, if the project declares one.
pub fn spend_budget() -> Option<f64> {
    spend::budget_usd_per_day()
}

/// Whether the frontier path may spend today: the log and the budget, judged
/// by [`spend_report::budget_verdict`].
pub fn spend_budget_check() -> Result<(), String> {
    spend_report::budget_verdict(&spend::entries(), spend::budget_usd_per_day())
}

/// Discovery, wired: the environment, the registry and PATH.
pub fn discovery() -> impl ports::Discovery {
    discover::EnvDiscovery
}

/// The endpoint registry, wired: `~/.hexa/inference-servers.json`.
pub fn endpoints() -> impl ports::EndpointRegistry {
    registry::FileRegistry
}
