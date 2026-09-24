//! [`Worktrees`], on git — through hexa-git.

use std::path::Path;

use crate::ports::Worktrees;

/// Worktrees created and removed with git.
pub struct GitWorktrees;

impl Worktrees for GitWorktrees {
    fn create(&self, repo: &Path, branch: &str, path: &Path) -> Result<(), String> {
        hexa_git::worktree::create_worktree(repo, branch, path).map(|_| ())
    }

    fn remove(&self, repo: &Path, path: &Path) -> Result<(), String> {
        hexa_git::worktree::remove_worktree(repo, &path.to_string_lossy(), true, true).map(|_| ())
    }
}
