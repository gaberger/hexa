//! [`GitHistory`], on git — through hexa-git (ADR-2610160756 §4).

use std::path::Path;

use crate::ports::GitHistory;

/// The current branch's history, moved with the git CLI.
pub struct GitHistoryCli;

impl GitHistory for GitHistoryCli {
    fn reset_to(&self, repo: &Path, sha: &str) -> Result<(), String> {
        hexa_git::history::reset_to(repo, sha)
    }

    fn cherry_pick(&self, repo: &Path, sha: &str) -> Result<(), String> {
        hexa_git::history::cherry_pick(repo, sha)
    }
}
