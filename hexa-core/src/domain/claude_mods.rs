//! Which steps install hexa's Claude Code mods (ADR-2610301129 §3).
//!
//! The domain decides the steps and is idempotent; running them goes through
//! the `ClaudePlugins` port.

/// One step toward installing hexa's Claude Code mods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing can be installed; the reason says why.
    Skip(String),
    /// Register the hexa marketplace at this path.
    AddMarketplace(std::path::PathBuf),
    /// Install this plugin, named `<mod>@hexa`.
    Install(String),
}

/// The steps that install `mods` from `marketplace`, given whether `claude` is
/// `available` and which plugins are already `installed`.
pub fn install_steps(
    available: bool,
    installed: &[String],
    marketplace: &std::path::Path,
    mods: &[&str],
) -> Vec<Step> {
    let _ = (available, installed, marketplace, mods);
    todo!("ADR-2610301129 §3")
}
