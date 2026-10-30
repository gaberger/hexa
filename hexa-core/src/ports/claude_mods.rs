//! What hexa needs from the Claude CLI to install its Claude Code mods
//! (ADR-2610301129 §2).

/// The Claude CLI's plugin commands, as hexa uses them.
pub trait ClaudePlugins: Send + Sync {
    /// The `claude` binary can be run.
    fn available(&self) -> bool;

    /// The ids of installed plugins, as `<plugin>@<marketplace>`.
    fn installed(&self) -> Result<Vec<String>, String>;

    /// Add the marketplace held in `folder`.
    fn add_marketplace(&self, folder: &std::path::Path) -> Result<(), String>;

    /// Install the plugin `id`, at user scope.
    fn install(&self, id: &str) -> Result<(), String>;
}
