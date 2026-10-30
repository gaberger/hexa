// Library shim — exposes crate internals to integration tests.
// The binary entry point is src/main.rs; this file re-exports the modules
// needed by tests in hexa-cli/tests/.

pub mod assets;
pub mod claude_plugins;
pub mod fmt;
pub mod commands;
pub mod http_probe;

/// The HTTP client `hexa api test` sends through (ADR-2610092329). Wired
/// here, at the crate root, so the verb asks for the port and never
/// constructs the adapter.
pub fn default_probe(base_url: &str) -> std::sync::Arc<dyn hexa_analysis::ports::HttpProbe> {
    std::sync::Arc::new(http_probe::ReqwestProbe::new(base_url))
}

/// The Claude CLI's plugin commands, which setup installs hexa's mods through
/// (ADR-2610301129 §4). The binary's name comes from hexa-infer (G1).
pub fn default_claude_plugins() -> Box<dyn hexa_core::ports::claude_mods::ClaudePlugins> {
    claude_plugins_at(hexa_infer::ports::frontier_cli().as_ref(), claude_plugins::Deadlines::default())
}

/// The same adapter over a given binary and deadlines, for tests.
pub fn claude_plugins_at(
    bin: &std::path::Path,
    deadlines: claude_plugins::Deadlines,
) -> Box<dyn hexa_core::ports::claude_mods::ClaudePlugins> {
    Box::new(claude_plugins::ClaudeCli::new(bin.to_path_buf(), deadlines))
}
