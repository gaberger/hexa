//! Which steps install hexa's Claude Code mods (ADR-2610301129 §2).
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
    if !available {
        return vec![Step::Skip(
            "claude CLI not available (not on PATH, or `claude --version` failed); \
             Claude Code mods not installed"
                .into(),
        )];
    }
    // Exact match on `<mod>@hexa`: a copy from another marketplace is not ours.
    let missing: Vec<String> = mods
        .iter()
        .map(|m| format!("{m}@hexa"))
        .filter(|id| !installed.iter().any(|i| i == id))
        .collect();
    if missing.is_empty() {
        return Vec::new();
    }
    std::iter::once(Step::AddMarketplace(marketplace.to_path_buf()))
        .chain(missing.into_iter().map(Step::Install))
        .collect()
}
