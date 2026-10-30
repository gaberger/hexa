//! What hexa needs from the Claude CLI to install its Claude Code mods
//! (ADR-2610301129 §3).

/// The step `apply` reports, re-exported so a caller of the port needs no
/// import from the domain.
pub use crate::domain::claude_mods::Step;

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

/// Run the install steps through the port (ADR-2610301129 §3).
///
/// Computes `install_steps` from `plugins.available()` and
/// `plugins.installed()`, runs each step through the port, and reports every
/// step with its outcome. A step that fails does not stop the ones after it.
/// If `installed()` fails, it is treated as nothing installed. It never panics.
pub fn apply(
    plugins: &dyn ClaudePlugins,
    marketplace: &std::path::Path,
    mods: &[&str],
) -> Vec<(Step, Result<(), String>)> {
    use crate::domain::claude_mods::install_steps;

    if !plugins.available() {
        return install_steps(false, &[], marketplace, mods)
            .into_iter()
            .map(|step| (step, Ok(())))
            .collect();
    }
    // A failed listing errs toward a redundant install, never a skipped one.
    let installed = plugins.installed().unwrap_or_default();
    install_steps(true, &installed, marketplace, mods)
        .into_iter()
        .map(|step| {
            let outcome = match &step {
                Step::Skip(_) => Ok(()),
                Step::AddMarketplace(folder) => plugins.add_marketplace(folder),
                Step::Install(id) => plugins.install(id),
            };
            (step, outcome)
        })
        .collect()
}
