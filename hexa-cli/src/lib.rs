// Library shim — exposes crate internals to integration tests.
// The binary entry point is src/main.rs; this file re-exports the modules
// needed by tests in hexa-cli/tests/.

pub mod assets;
pub mod fmt;
pub mod commands;
pub mod http_probe;

/// The HTTP client `hexa api test` sends through (ADR-2610092329). Wired
/// here, at the crate root, so the verb asks for the port and never
/// constructs the adapter.
pub fn default_probe(base_url: &str) -> std::sync::Arc<dyn hexa_analysis::ports::HttpProbe> {
    std::sync::Arc::new(http_probe::ReqwestProbe::new(base_url))
}
