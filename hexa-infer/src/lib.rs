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
pub mod discover;
pub mod registry;
pub mod spend;
pub mod tiers;
pub mod wiring;

pub use adapters::secondary::{
    AnthropicAdapter, ClaudeCodeInferenceAdapter, OllamaInferenceAdapter, OpenAiCompatAdapter,
};
pub use endpoint::Endpoint;
pub use tiers::{react_models, react_models_in_config, tier_model};
pub use local_provider::{configured_tiers, local_provider, LocalProvider};
pub use discover::{discover, enumerate_models, served_models, serves, Coverage, Found};

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
