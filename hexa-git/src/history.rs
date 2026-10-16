//! History operations on the current branch: reset it to a commit and
//! cherry-pick a commit onto it — what harden's prefix rebuild needs
//! (ADR-2610160756, Implementation).
//!
//! Neither touches the operator's uncommitted files (ADR-2610141733 §6): a reset
//! uses `git reset --keep`, which refuses rather than overwrite them, and a
//! cherry-pick that fails is aborted so HEAD, the index and the tree are as they
//! were. Uses the git CLI, like `worktree.rs`.

use std::path::Path;
use std::process::{Command, Output};

/// hexa commits as itself: CI containers have no configured identity, and an
/// operator's must not be borrowed (ADR-2606071323 §4).
const NAME: &str = "hexa-factory";
const EMAIL: &str = "factory@hexa.local";
const IDENTITY: [&str; 4] = ["-c", "user.name=hexa-factory", "-c", "user.email=factory@hexa.local"];

fn run(repo: &Path, args: &[&str]) -> Result<Output, String> {
    git(repo, args).output().map_err(|e| format!("Failed to run git {}: {}", args.join(" "), e))
}

fn git(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo);
    cmd
}

/// Run git as hexa: `GIT_COMMITTER_*` in the environment outrank `-c user.*`,
/// so they are pinned too, or an inherited identity would be borrowed.
fn run_as_hexa(repo: &Path, args: &[&str]) -> Result<Output, String> {
    let mut full: Vec<&str> = IDENTITY.to_vec();
    full.extend_from_slice(args);
    git(repo, &full)
        .env("GIT_COMMITTER_NAME", NAME)
        .env("GIT_COMMITTER_EMAIL", EMAIL)
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", args.join(" "), e))
}

fn reason(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    }
}

/// Move the current branch to `sha` with `git reset --keep` semantics: the
/// operator's uncommitted changes stay, and git refuses rather than overwrite one.
pub fn reset_to(repo: &Path, sha: &str) -> Result<(), String> {
    let output = run(repo, &["reset", "--keep", sha, "--"])?;
    if !output.status.success() {
        return Err(format!("git reset --keep {} failed: {}", sha, reason(&output)));
    }
    Ok(())
}

/// Apply the commit `sha` on HEAD, keeping its message, committed under hexa's
/// own identity. On a conflict or any other failure the cherry-pick is aborted,
/// leaving HEAD, the index and the tree as they were.
pub fn cherry_pick(repo: &Path, sha: &str) -> Result<(), String> {
    let output = run_as_hexa(repo, &["cherry-pick", "--allow-empty", sha])?;
    if output.status.success() {
        return Ok(());
    }
    let why = reason(&output);
    if repo_has_cherry_pick_in_progress(repo) {
        let abort = run(repo, &["cherry-pick", "--abort"])?;
        if !abort.status.success() {
            return Err(format!(
                "git cherry-pick {} failed: {}; and git cherry-pick --abort failed: {}",
                sha,
                why,
                reason(&abort)
            ));
        }
    }
    Err(format!("git cherry-pick {} failed: {}", sha, why))
}

fn repo_has_cherry_pick_in_progress(repo: &Path) -> bool {
    run(repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}
