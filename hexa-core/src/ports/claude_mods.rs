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

/// Run the install steps through the port (ADR-2610301129 §4).
///
/// Computes `install_steps` from `plugins.available()` and
/// `plugins.installed()`, runs each step through the port, and reports every
/// step with its outcome. A step that fails does not stop the ones after it.
/// If `installed()` fails, it is treated as nothing installed. It never panics.
#[allow(unused_variables)] // stub until §4 is built
pub fn apply(
    plugins: &dyn ClaudePlugins,
    marketplace: &std::path::Path,
    mods: &[&str],
) -> Vec<(crate::domain::claude_mods::Step, Result<(), String>)> {
    todo!("ADR-2610301129 §4")
}
