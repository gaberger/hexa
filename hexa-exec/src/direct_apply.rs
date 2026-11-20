//! `hexa do apply` (ADR-2610281700): a gated change without a model.
//!
//! A unified diff, already known exactly, goes through `do run`'s gate:
//! `run_evidence`, then `commit_with_also` on exit 0 and `record_run`, or a
//! revert on red. No inference of any kind is called on any path.

use crate::direct_exec::{self, DirectResult, DirectTask};
use std::path::Path;

/// What one apply came to. `exit` is the process status: 0 committed,
/// 1 the evidence failed and the tree was restored, 2 refused (nothing changed).
#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub ok: bool,
    pub commit: Option<String>,
    /// `None` on green, `gate_failed` on red, `config` on a refusal.
    pub code: Option<&'static str>,
    pub message: String,
    pub exit: i32,
    /// The evidence's output, when it ran.
    pub evidence_output: String,
}

impl ApplyOutcome {
    fn refused(message: String) -> Self {
        ApplyOutcome {
            ok: false,
            commit: None,
            code: Some("config"),
            message,
            exit: 2,
            evidence_output: String::new(),
        }
    }
}

/// One path a diff touches, and whether the diff creates it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Touched {
    pub path: String,
    pub created: bool,
}

/// `a/x` / `b/x` → `x`, dropping a trailing timestamp; `/dev/null` → `None`.
fn header_path(rest: &str) -> Option<String> {
    let p = rest.split('\t').next().unwrap_or(rest).trim_end();
    if p == "/dev/null" {
        return None;
    }
    let p = p
        .strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p);
    Some(direct_exec::normalize_also(p))
}

/// The `-a,b +c,d` line counts of a hunk header.
fn hunk_counts(line: &str) -> Option<(usize, usize)> {
    let mut it = line.strip_prefix("@@ ")?.split(' ');
    let count = |s: &str, sign: char| -> Option<usize> {
        let s = s.strip_prefix(sign)?;
        Some(match s.split_once(',') {
            Some((_, n)) => n.parse().ok()?,
            None => 1,
        })
    };
    Some((count(it.next()?, '-')?, count(it.next()?, '+')?))
}

/// Every path the diff touches, from its `---` / `+++` headers, read outside
/// hunk bodies so a removed line that starts `--` is never taken for one.
pub(crate) fn touched_paths(patch: &str) -> Result<Vec<Touched>, String> {
    let mut out: Vec<Touched> = Vec::new();
    let lines: Vec<&str> = patch.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if let Some(old) = l.strip_prefix("--- ") {
            let new = lines
                .get(i + 1)
                .and_then(|n| n.strip_prefix("+++ "))
                .ok_or_else(|| format!("a `--- ` header with no `+++ ` after it: {l}"))?;
            let (old, new) = (header_path(old), header_path(new));
            let created = old.is_none();
            for p in [old, new].into_iter().flatten() {
                match out.iter_mut().find(|t| t.path == p) {
                    Some(t) => t.created = t.created && created,
                    None => out.push(Touched { path: p, created }),
                }
            }
            i += 2;
            continue;
        }
        if let Some((mut old_n, mut new_n)) = hunk_counts(l) {
            i += 1;
            while (old_n > 0 || new_n > 0) && i < lines.len() {
                match lines[i].chars().next() {
                    Some('-') => old_n = old_n.saturating_sub(1),
                    Some('+') => new_n = new_n.saturating_sub(1),
                    Some('\\') => {}
                    _ => {
                        old_n = old_n.saturating_sub(1);
                        new_n = new_n.saturating_sub(1);
                    }
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    if out.is_empty() {
        return Err("the diff touches no file".to_string());
    }
    Ok(out)
}

async fn git_apply(repo_root: &Path, flags: &[&str], patch: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("git")
        .arg("apply")
        .args(flags)
        .arg("-")
        .current_dir(repo_root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("git apply: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(patch.as_bytes())
            .await
            .map_err(|e| format!("git apply: {e}"))?;
    }
    let o = child
        .wait_with_output()
        .await
        .map_err(|e| format!("git apply: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// The listed paths with uncommitted changes, by `git status --porcelain`.
async fn uncommitted(repo_root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let o = tokio::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all", "--"])
        .args(paths)
        .current_dir(repo_root)
        .output()
        .await
        .map_err(|e| format!("git status: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "git status: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].to_string())
        .collect())
}

/// Apply `patch` to `repo_root`, gate it on `evidence`, and commit exactly the
/// listed paths on green or restore them on red (ADR-2610281700).
pub async fn apply(
    repo_root: &Path,
    description: &str,
    file: &str,
    also: &[String],
    evidence: &str,
    patch: &str,
) -> ApplyOutcome {
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();

    let mut listed = vec![file.to_string()];
    listed.extend(also.iter().cloned());
    if let Err(e) = direct_exec::validate_also_paths(&listed) {
        return ApplyOutcome::refused(e);
    }
    let listed: Vec<String> = listed
        .iter()
        .map(|p| direct_exec::normalize_also(p))
        .collect();

    // 1. Only the listed files.
    let touched = match touched_paths(patch) {
        Ok(t) => t,
        Err(e) => return ApplyOutcome::refused(format!("refused: {e}")),
    };
    let unlisted: Vec<&str> = touched
        .iter()
        .map(|t| t.path.as_str())
        .filter(|p| !listed.iter().any(|l| l == p))
        .collect();
    if !unlisted.is_empty() {
        return ApplyOutcome::refused(format!(
            "refused: the diff touches {}, which is not --file or an --also; nothing changed",
            unlisted.join(", ")
        ));
    }

    // 2. Never mix into, or revert, the user's uncommitted work.
    match uncommitted(repo_root, &listed).await {
        Ok(d) if !d.is_empty() => {
            return ApplyOutcome::refused(format!(
                "refused: {} has uncommitted changes; commit or stash them first; nothing changed",
                d.join(", ")
            ))
        }
        Ok(_) => {}
        Err(e) => return ApplyOutcome::refused(format!("refused: {e}")),
    }

    // 3. Whole or not at all. A diff already in the tree (it reverses
    // cleanly) has nothing to apply; the tree is still gated as it stands.
    let mut already = false;
    if let Err(e) = git_apply(repo_root, &["--check"], patch).await {
        if git_apply(repo_root, &["--check", "--reverse"], patch)
            .await
            .is_err()
        {
            return ApplyOutcome::refused(format!(
                "refused: the diff does not apply; nothing changed: {e}"
            ));
        }
        already = true;
    }
    // 4.
    if !already {
        if let Err(e) = git_apply(repo_root, &[], patch).await {
            restore(repo_root, &touched).await;
            return ApplyOutcome::refused(format!("refused: git apply failed: {e}"));
        }
    }

    // 5. The gate.
    let (passed, output) = direct_exec::run_evidence(evidence, repo_root).await;

    let mut outcome = if passed && already {
        ApplyOutcome {
            ok: true,
            commit: None,
            code: None,
            message: "the diff was already applied; evidence passed; nothing to commit".to_string(),
            exit: 0,
            evidence_output: output,
        }
    } else if passed {
        // 6. Commit on green.
        let subject: String = description
            .lines()
            .next()
            .unwrap_or("apply")
            .chars()
            .take(72)
            .collect();
        let msg = format!("feat(apply): {subject}\n\nApplied by `hexa do apply` (ADR-2610281700): a gated change without a model.");
        match direct_exec::commit_with_also_message(repo_root, file, &msg, false, &[], also).await {
            Ok(short) => {
                let hash = full_hash(repo_root, &short).await.unwrap_or(short);
                ApplyOutcome {
                    ok: true,
                    message: format!("evidence passed; committed {hash}"),
                    commit: Some(hash),
                    code: None,
                    exit: 0,
                    evidence_output: output,
                }
            }
            Err(e) => {
                restore(repo_root, &touched).await;
                ApplyOutcome {
                    ok: false,
                    commit: None,
                    code: Some("gate_failed"),
                    message: format!(
                        "evidence passed but the commit failed; the change was restored: {e}"
                    ),
                    exit: 1,
                    evidence_output: output,
                }
            }
        }
    } else {
        restore(repo_root, &touched).await;
        ApplyOutcome {
            ok: false,
            commit: None,
            code: Some("gate_failed"),
            message: "evidence failed; every touched file was restored".to_string(),
            exit: 1,
            evidence_output: output,
        }
    };

    let task = DirectTask {
        instruction: description.to_string(),
        file: file.to_string(),
        evidence: evidence.to_string(),
        also: also.to_vec(),
        isolate: Some(false),
        ..Default::default()
    };
    let r = DirectResult {
        ok: outcome.ok,
        attempts: 1,
        edit_applied: true,
        committed: outcome.commit.clone(),
        evidence_passed: passed,
        evidence_output: std::mem::take(&mut outcome.evidence_output),
        error: (!outcome.ok).then(|| outcome.message.clone()),
        restored_unlisted: Vec::new(),
        left_modified: Vec::new(),
    };
    direct_exec::record_run(
        &crate::run_log(),
        started_at,
        &task,
        "none",
        &r,
        started.elapsed().as_millis() as u64,
    );
    outcome.evidence_output = r.evidence_output;
    outcome
}

/// The full id of the commit `git commit` named by its abbreviation.
async fn full_hash(repo_root: &Path, short: &str) -> Option<String> {
    let o = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "-q"])
        .arg(format!("{short}^{{commit}}"))
        .current_dir(repo_root)
        .output()
        .await
        .ok()?;
    let h = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !h.is_empty()).then_some(h)
}

/// Put every touched path back as `HEAD` has it; a created one is deleted.
async fn restore(repo_root: &Path, touched: &[Touched]) {
    let paths: Vec<String> = touched.iter().map(|t| t.path.clone()).collect();
    direct_exec::restore_unlisted(repo_root, &paths).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_read_outside_hunks() {
        let p = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n--- not a header\n+++ nor this\n";
        assert_eq!(
            touched_paths(p).unwrap(),
            vec![Touched {
                path: "x".into(),
                created: false
            }]
        );
    }

    #[test]
    fn dev_null_means_created() {
        let p = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+fresh\n";
        assert_eq!(
            touched_paths(p).unwrap(),
            vec![Touched {
                path: "new.txt".into(),
                created: true
            }]
        );
    }
}
