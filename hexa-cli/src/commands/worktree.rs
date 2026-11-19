//! Git worktree management commands.
//!
//! `hexa worktree list|merge|cleanup` — manage feature worktrees.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand)]
pub enum WorktreeAction {
    /// List all active worktrees
    List {
        /// Only list worktrees with no commits in the last 24h (drift signal
        /// for the improver `git_drift` detector).
        #[arg(long)]
        stale: bool,
        /// Emit findings as JSON for the improver detector pipeline
        /// (`{findings: [{branch, path, age_hours}]}`).
        #[arg(long)]
        json: bool,
    },
    /// Merge worktree branches into main (file-level cherry-pick)
    Merge {
        /// Branch pattern to match (e.g. "feat/auth" matches all feat/auth/* branches)
        #[arg(value_name = "PATTERN")]
        pattern: Option<String>,
        /// Merge all non-main worktree branches
        #[arg(long)]
        all: bool,
        /// Actually perform the merge (default is dry-run)
        #[arg(long)]
        force: bool,
    },
    /// The order in which finished tasks' worktrees can be merged without a lease conflict.
    /// Reads the task ledger; runs no merge. JSON when piped or with --json.
    #[command(name = "merge-order")]
    MergeOrder {
        #[arg(long)]
        json: bool,
    },
    /// Remove worktrees whose branches are already merged into main. A worktree with
    /// uncommitted changes is never removed
    Cleanup {
        /// Actually remove (default is dry-run)
        #[arg(long)]
        force: bool,
        /// Also remove unmerged feature worktrees (feat/, hexa/, worktree-, claude/) with no
        /// commit in 24h. Their branches are kept
        #[arg(long)]
        stale: bool,
    },
}

pub async fn run(action: WorktreeAction) -> anyhow::Result<()> {
    match action {
        WorktreeAction::List { stale, json } => list(stale, json).await,
        WorktreeAction::Merge { pattern, all, force } => merge(pattern, all, force).await,
        WorktreeAction::MergeOrder { json } => merge_order(json).await,
        WorktreeAction::Cleanup { force, stale } => cleanup(force, stale).await,
    }
}

// ── Helpers ────────────────────────────────────────────

fn repo_root() -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Not inside a git repository");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main_branch() -> String {
    let output = Command::new("git")
        .args(["branch", "--list", "main"])
        .output()
        .ok();
    if let Some(o) = output {
        if !String::from_utf8_lossy(&o.stdout).trim().is_empty() {
            return "main".to_string();
        }
    }
    "master".to_string()
}

/// Parse `git worktree list --porcelain` into (path, branch) pairs.
fn parse_worktrees() -> anyhow::Result<Vec<(String, String)>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    let mut current_path = String::new();
    let mut current_branch = String::new();

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            if !current_path.is_empty() && !current_branch.is_empty() {
                results.push((current_path.clone(), current_branch.clone()));
            }
            current_path = line.strip_prefix("worktree ").unwrap_or("").to_string();
            current_branch.clear();
        } else if line.starts_with("branch ") {
            let raw = line.strip_prefix("branch ").unwrap_or("");
            current_branch = raw
                .strip_prefix("refs/heads/")
                .unwrap_or(raw)
                .to_string();
        } else if line == "detached" {
            current_branch = "(detached)".to_string();
        } else if line.is_empty() {
            if !current_path.is_empty() && !current_branch.is_empty() {
                results.push((current_path.clone(), current_branch.clone()));
            }
            current_path.clear();
            current_branch.clear();
        }
    }
    // Last entry
    if !current_path.is_empty() && !current_branch.is_empty() {
        results.push((current_path, current_branch));
    }

    Ok(results)
}

/// Get changed files for a branch vs main.
fn changed_files(branch: &str, main: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", &format!("{}...{}", main, branch), "--name-only"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git diff failed for branch {}: {}",
            branch,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

/// Get the Unix timestamp of the latest commit on a branch.
fn latest_commit_epoch(branch: &str) -> anyhow::Result<i64> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ct", branch])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git log failed for branch {}: {}",
            branch,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let ts = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ts.parse::<i64>()
        .map_err(|e| anyhow::anyhow!("Failed to parse timestamp for {}: {}", branch, e))
}

/// Check if a branch is merged into main.
fn is_merged(branch: &str, main: &str) -> bool {
    let output = Command::new("git")
        .args(["branch", "--merged", main])
        .output()
        .ok();
    if let Some(o) = output {
        let stdout = String::from_utf8_lossy(&o.stdout);
        return stdout
            .lines()
            .any(|l| l.trim_start_matches(['*', '+', ' ']).trim() == branch);
    }
    false
}

// ── Ledger (ADR-2611051850) ────────────────────────────

/// The ledger task registered for the worktree at `path`, if any.
fn task_for_worktree<'a>(ledger: &'a hexa_core::ports::ledger::Ledger, path: &str) -> Option<&'a hexa_core::ports::ledger::Task> {
    let here = crate::commands::task_ledger::canonical_lenient(Path::new(path)).display().to_string();
    // The newest registration for the path decides.
    ledger.tasks.iter().rev().find(|t| t.worktree.as_deref() == Some(here.as_str()))
}

/// Refuse to merge a worktree whose task is not `done`. A ledger that cannot
/// be read refuses too: "no task found" must not mean "could not look".
fn refuse_unfinished_tasks(candidates: &[(String, String)]) -> anyhow::Result<()> {
    use hexa_core::ports::ledger::TaskStatus;
    let cwd = std::env::current_dir()?;
    let ledger = crate::commands::task_ledger::read(&cwd).map_err(|e| anyhow::anyhow!("{e}"))?;
    let blocked: Vec<String> = candidates
        .iter()
        .filter_map(|(path, branch)| {
            let t = task_for_worktree(&ledger, path)?;
            (t.status != TaskStatus::Done).then(|| format!("{branch} (task {} `{}` is {})", t.id, t.name, t.status))
        })
        .collect();
    if blocked.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to merge worktrees whose task is not done: {}. Close each with `hexa agent report --id <id> --outcome passed`, or free it with `hexa agent release --id <id>`; `hexa dev worktree merge-order` lists what is ready.",
        blocked.join("; ")
    )
}

/// `hexa dev worktree merge-order`: done tasks' worktrees inner layer first
/// (domain, ports, use cases, adapters, composition root; ADR-2611160750 §4),
/// in finish order within a layer, with every pair whose leases overlap named, and the worktrees
/// that are not ready and why. Pure reading; no merge is run.
async fn merge_order(json: bool) -> anyhow::Result<()> {
    use hexa_core::ports::ledger::{overlaps, TaskStatus};
    use std::io::IsTerminal;
    let cwd = std::env::current_dir()?;
    let ledger = match crate::commands::task_ledger::read(&cwd) {
        Ok(l) => l,
        Err(e) => {
            if json || !std::io::stdout().is_terminal() {
                let mut v = serde_json::to_value(&e)?;
                v["ok"] = serde_json::json!(false);
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                eprintln!("{} {}\n  hint: {}", "\u{2717}".red(), e.message, e.hint);
            }
            crate::commands::events::exit(1);
        }
    };
    let mut ready: Vec<&hexa_core::ports::ledger::Task> =
        ledger.tasks.iter().filter(|t| t.status == TaskStatus::Done && t.worktree.is_some()).collect();
    ready.sort_by(|a, b| a.updated.cmp(&b.updated).then(a.id.cmp(&b.id)));
    // Inner to outer; a stable sort, so a tie keeps finish order. A task's
    // layer is the innermost of its leases'.
    let layers = match crate::commands::lease_layers::Arch::layer_map(&cwd) {
        Ok(l) => l,
        Err(e) => anyhow::bail!("{e}"),
    };
    let layer_of = |t: &hexa_core::ports::ledger::Task| {
        t.leases
            .iter()
            .map(|l| layers.classify(l))
            .min_by_key(|l| crate::commands::lease_layers::merge_rank(*l))
            .unwrap_or(hexa_analysis::ports::HexLayer::Unknown)
    };
    ready.sort_by_key(|t| crate::commands::lease_layers::merge_rank(layer_of(t)));
    let mut conflicts: Vec<serde_json::Value> = Vec::new();
    for (i, a) in ready.iter().enumerate() {
        for b in ready.iter().skip(i + 1) {
            if let Some((x, y)) = a.leases.iter().flat_map(|x| b.leases.iter().map(move |y| (x, y))).find(|(x, y)| overlaps(x, y)) {
                conflicts.push(serde_json::json!({
                    "first": a.id, "second": b.id, "paths": [x, y],
                    "hint": "Their leases overlap (the first was released before the second was registered): merge in this order and expect to resolve the overlap by hand.",
                }));
            }
        }
    }
    let not_ready: Vec<serde_json::Value> = ledger
        .tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Done && t.status != TaskStatus::Released && t.worktree.is_some())
        .map(|t| serde_json::json!({
            "id": t.id, "name": t.name, "worktree": t.worktree, "status": t.status,
            "reason": format!("task is {}; a worktree is mergeable when its task is done", t.status),
        }))
        .collect();
    let order: Vec<serde_json::Value> = ready
        .iter()
        .enumerate()
        .map(|(n, t)| serde_json::json!({
            "position": n + 1, "id": t.id, "name": t.name, "worktree": t.worktree,
            "commit": t.commit, "finished": t.updated, "leases": t.leases,
            "layer": layer_of(t).to_string(),
        }))
        .collect();
    let report = serde_json::json!({
        "ok": true,
        "order": order,
        "conflicts": conflicts,
        "not_ready": not_ready,
        "note": "Inner layers merge first (domain, ports, use cases, adapters, composition root), each later task rebasing on the earlier and re-passing its gate; within a layer the order is by finish time. This verb merges nothing: use `hexa dev worktree merge <branch> --force`.",
    });
    if json || !std::io::stdout().is_terminal() {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("{}", "Merge order".bold());
    if order.is_empty() {
        println!("  No finished task has a worktree.");
    }
    for o in &order {
        println!("  {}. {} {}  {}", o["position"], o["id"].as_str().unwrap_or("?").cyan(), o["name"].as_str().unwrap_or(""), o["worktree"].as_str().unwrap_or("").dimmed());
    }
    for c in &conflicts {
        println!("  {} {} then {} overlap on {}", "\u{26a0}".yellow(), c["first"], c["second"], c["paths"]);
    }
    for n in &not_ready {
        println!("  {} {} {}", "\u{25cb}".yellow(), n["id"].as_str().unwrap_or("?"), n["reason"].as_str().unwrap_or(""));
    }
    Ok(())
}

// ── Subcommands ────────────────────────────────────────

async fn list(stale: bool, json: bool) -> anyhow::Result<()> {
    let worktrees = parse_worktrees()?;
    let main = main_branch();

    // Compute hours since the most recent commit on each worktree's branch.
    // A worktree with >24h since last commit is "stale" — likely abandoned
    // mid-feature or already merged elsewhere. Branches we can't query (e.g.
    // detached) get u64::MAX so the --stale filter treats them as stale too.
    let last_commit_hours = |branch: &str| -> u64 {
        let out = Command::new("git")
            .args(["log", "-1", "--format=%ct", branch])
            .output();
        let Ok(out) = out else { return u64::MAX };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let Ok(committed_at) = s.parse::<i64>() else { return u64::MAX };
        let now = chrono::Utc::now().timestamp();
        ((now - committed_at).max(0) / 3600) as u64
    };

    let mut entries: Vec<(String, String, u64, bool)> = worktrees
        .iter()
        .map(|(path, branch)| {
            let age = if branch == &main { 0 } else { last_commit_hours(branch) };
            let is_stale = branch != &main && age > 24;
            (path.clone(), branch.clone(), age, is_stale)
        })
        .collect();
    if stale {
        entries.retain(|e| e.3);
    }

    if json {
        let findings: Vec<_> = entries
            .iter()
            .map(|(path, branch, age, is_stale)| {
                serde_json::json!({
                    "branch": branch,
                    "path": path,
                    "age_hours": age,
                    "stale": is_stale,
                    "severity": if *is_stale { "warning" } else { "info" },
                })
            })
            .collect();
        println!("{}", serde_json::json!({"findings": findings}));
        return Ok(());
    }

    println!("{}", "Worktrees".bold());
    println!("{}", "\u{2500}".repeat(60));

    if entries.is_empty() {
        println!("  No worktrees found.");
        return Ok(());
    }

    for (path, branch, _age, _is_stale) in &entries {
        let is_main = branch == &main;
        let merged = if !is_main { is_merged(branch, &main) } else { false };
        let indicator = if is_main {
            "\u{25cf}".green().to_string()
        } else if merged {
            "\u{2713}".green().to_string()
        } else {
            "\u{25cb}".yellow().to_string()
        };
        let merged_tag = if merged && !is_main {
            " (merged)".green().to_string()
        } else {
            String::new()
        };
        let display_branch = if is_main {
            branch.green().bold().to_string()
        } else {
            branch.cyan().to_string()
        };
        println!("  {} {} {}{}", indicator, display_branch, path.dimmed(), merged_tag);
    }

    Ok(())
}

async fn merge(
    pattern: Option<String>,
    all: bool,
    force: bool,
) -> anyhow::Result<()> {
    if pattern.is_none() && !all {
        anyhow::bail!("Specify a branch pattern or use --all");
    }

    let worktrees = parse_worktrees()?;
    let main = main_branch();

    // Filter worktrees by pattern
    let candidates: Vec<(String, String)> = worktrees
        .into_iter()
        .filter(|(_, branch)| {
            if branch == &main || branch == "(detached)" {
                return false;
            }
            if all {
                return true;
            }
            if let Some(ref pat) = pattern {
                branch.contains(pat)
            } else {
                false
            }
        })
        .collect();

    if candidates.is_empty() {
        println!("  No matching worktree branches found.");
        return Ok(());
    }

    // ADR-2611051850: a worktree whose task has not finished well is not
    // merged. A worktree no task was registered for is unaffected.
    refuse_unfinished_tasks(&candidates)?;

    println!("{}", "Worktree Merge".bold());
    println!("{}", "\u{2500}".repeat(60));

    // Collect changed files per branch
    let mut branch_files: HashMap<String, Vec<String>> = HashMap::new();
    for (_, branch) in &candidates {
        match changed_files(branch, &main) {
            Ok(files) => {
                if !files.is_empty() {
                    branch_files.insert(branch.clone(), files);
                } else {
                    println!("  {} {} — no changes vs main", "\u{2013}".dimmed(), branch.dimmed());
                }
            }
            Err(e) => {
                println!("  {} {} — {}", "\u{2717}".red(), branch, e);
            }
        }
    }

    if branch_files.is_empty() {
        println!("\n  No files to merge.");
        return Ok(());
    }

    // Detect file overlaps between branches
    let mut file_owners: HashMap<String, Vec<String>> = HashMap::new();
    for (branch, files) in &branch_files {
        for file in files {
            file_owners
                .entry(file.clone())
                .or_default()
                .push(branch.clone());
        }
    }

    let overlapping: HashMap<&String, &Vec<String>> = file_owners
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .collect();

    // For overlapping files, pick the branch with the most recent commit
    let mut overlap_winner: HashMap<String, String> = HashMap::new();
    if !overlapping.is_empty() {
        println!("\n  {} File overlaps detected — resolving by most recent commit:", "\u{26a0}".yellow());

        // Cache branch timestamps
        let mut branch_ts: HashMap<String, i64> = HashMap::new();
        for branch in branch_files.keys() {
            if let Ok(ts) = latest_commit_epoch(branch) {
                branch_ts.insert(branch.clone(), ts);
            }
        }

        for (file, owners) in &overlapping {
            // Find the owner with the highest (most recent) timestamp
            let winner = owners
                .iter()
                .max_by_key(|b| branch_ts.get(*b).copied().unwrap_or(0))
                .cloned()
                .unwrap_or_else(|| owners[0].clone());
            println!(
                "    {} owned by [{}] → picking {}",
                file.yellow(),
                owners.join(", ").dimmed(),
                winner.cyan()
            );
            overlap_winner.insert((*file).clone(), winner);
        }
    }

    // Build the set of files that are overlapping (handled separately)
    let overlap_files: HashSet<&String> = overlapping.keys().copied().collect();

    // Determine merge order by tier prefix (P0 < P1 < P2 ...)
    let mut ordered_branches: Vec<(u32, String)> = branch_files
        .keys()
        .map(|b| {
            // Try to extract tier from branch name (e.g. feat/foo/p0-domain -> 0)
            let tier = b
                .split('/')
                .next_back()
                .and_then(|seg| {
                    let seg_lower = seg.to_lowercase();
                    if seg_lower.starts_with('p') {
                        seg_lower
                            .strip_prefix('p')
                            .unwrap_or(&seg_lower)
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect::<String>()
                            .parse::<u32>()
                            .ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(99);
            (tier, b.clone())
        })
        .collect();
    ordered_branches.sort_by_key(|(tier, _)| *tier);

    // Show plan
    println!("\n  Merge order (non-overlapping, by tier):");
    let mut total_files = 0usize;
    for (tier, branch) in &ordered_branches {
        let files = &branch_files[branch];
        let safe_files: Vec<&String> = files.iter().filter(|f| !overlap_files.contains(f)).collect();
        if safe_files.is_empty() {
            continue;
        }
        println!(
            "    {} {} — {} file(s)",
            format!("T{}", tier).cyan(),
            branch,
            safe_files.len()
        );
        for f in &safe_files {
            println!("      {}", f);
        }
        total_files += safe_files.len();
    }

    if !overlap_winner.is_empty() {
        println!("\n  Overlap resolution (most recent branch wins):");
        for (file, winner) in &overlap_winner {
            println!("    {} ← {}", file, winner.cyan());
        }
        total_files += overlap_winner.len();
    }

    if !force {
        println!(
            "\n  {} Dry run — {} file(s) would be merged. Pass {} to execute.",
            "\u{26a0}".yellow(),
            total_files,
            "--force".bold()
        );
        return Ok(());
    }

    // Execute merge — Phase 1: non-overlapping files in tier order
    let root = repo_root()?;
    let root_path = Path::new(&root);
    let mut merged_count = 0usize;
    let mut errors = Vec::new();

    for (_tier, branch) in &ordered_branches {
        let files = &branch_files[branch];
        let safe_files: Vec<String> = files
            .iter()
            .filter(|f| !overlap_files.contains(f))
            .cloned()
            .collect();

        if safe_files.is_empty() {
            continue;
        }

        // git checkout <branch> -- <files>
        let mut args = vec!["checkout".to_string(), branch.clone(), "--".to_string()];
        args.extend(safe_files.iter().cloned());

        let output = Command::new("git")
            .args(&args)
            .current_dir(root_path)
            .output()?;

        if output.status.success() {
            println!(
                "  {} Merged {} file(s) from {}",
                "\u{2713}".green(),
                safe_files.len(),
                branch.cyan()
            );
            merged_count += safe_files.len();
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            println!("  {} Failed to merge from {}: {}", "\u{2717}".red(), branch, err);
            errors.push((branch.clone(), err));
        }
    }

    // Execute merge — Phase 2: overlapping files from winner branches
    // Group overlap files by winner branch for batch checkout
    let mut winner_files: HashMap<String, Vec<String>> = HashMap::new();
    for (file, winner) in &overlap_winner {
        winner_files
            .entry(winner.clone())
            .or_default()
            .push(file.clone());
    }

    for (branch, files) in &winner_files {
        let mut args = vec!["checkout".to_string(), branch.clone(), "--".to_string()];
        args.extend(files.iter().cloned());

        let output = Command::new("git")
            .args(&args)
            .current_dir(root_path)
            .output()?;

        if output.status.success() {
            println!(
                "  {} Cherry-picked {} overlapping file(s) from {}",
                "\u{2713}".green(),
                files.len(),
                branch.cyan()
            );
            merged_count += files.len();
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            println!(
                "  {} Failed to cherry-pick from {}: {}",
                "\u{2717}".red(),
                branch,
                err
            );
            errors.push((branch.clone(), err));
        }
    }

    // ── Merge Integrity Check (ADR-2026-04-13-1800) ─────────────────────────
    // Verify every worktree's added lines are present on main after merge.
    // This catches the "last write wins" problem where git checkout from
    // one branch silently drops additions from another branch.
    println!("\n  Verifying merge integrity...");
    let mut integrity_failures: Vec<(String, String, usize)> = Vec::new(); // (branch, file, missing_lines)

    for (_, branch) in &ordered_branches {
        // Get what this branch added vs main (before our merge)
        let diff_output = Command::new("git")
            .args(["diff", &format!("{}...{}", main, branch), "--unified=0"])
            .current_dir(root_path)
            .output();

        let diff_text = match diff_output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => continue,
        };

        // Parse added lines from the diff (lines starting with + but not +++)
        let mut current_file = String::new();
        let mut missing_count = 0usize;

        for line in diff_text.lines() {
            if line.starts_with("+++ b/") {
                // Check previous file's missing lines
                if missing_count > 0 {
                    integrity_failures.push((branch.clone(), current_file.clone(), missing_count));
                }
                current_file = line.strip_prefix("+++ b/").unwrap_or("").to_string();
                missing_count = 0;
            } else if line.starts_with('+') && !line.starts_with("+++") {
                // This is an added line — verify it exists in the working tree
                let added_content = &line[1..]; // strip the leading +
                if added_content.trim().is_empty() {
                    continue; // skip blank lines
                }
                // Read the file from working tree and check if the line is present
                let file_path = root_path.join(&current_file);
                if let Ok(contents) = std::fs::read_to_string(&file_path) {
                    if !contents.contains(added_content.trim()) {
                        missing_count += 1;
                    }
                }
            }
        }
        // Check last file
        if missing_count > 0 {
            integrity_failures.push((branch.clone(), current_file, missing_count));
        }
    }

    if !integrity_failures.is_empty() {
        println!(
            "\n  {} MERGE INCOMPLETE — code from worktree agents was dropped:",
            "\u{2717}".red().bold()
        );
        for (branch, file, count) in &integrity_failures {
            println!(
                "    {} {} lines from {} missing in {}",
                "\u{2717}".red(),
                count,
                branch.cyan(),
                file.yellow()
            );
        }
        println!(
            "\n  Fix: use {} on the affected files to manually merge both branches' changes.",
            "git merge <branch>".bold()
        );
        // Don't proceed to cargo check — merge is incomplete
        anyhow::bail!(
            "Merge integrity check failed: {} file(s) have missing lines from worktree agents",
            integrity_failures.len()
        );
    } else {
        println!(
            "  {} All worktree additions verified on main",
            "\u{2713}".green()
        );
    }

    // Run cargo check as gate
    println!("\n  Running cargo check --workspace ...");
    let check = Command::new("cargo")
        .args(["check", "--workspace"])
        .current_dir(root_path)
        .status()?;

    if check.success() {
        println!("  {} cargo check passed", "\u{2713}".green());
    } else {
        println!(
            "  {} cargo check FAILED — merged files may have conflicts",
            "\u{2717}".red()
        );
    }

    // Summary
    println!("\n  {} Merged {} file(s) from {} branch(es).",
        "\u{2500}".repeat(3),
        merged_count,
        ordered_branches.len()
    );
    if !errors.is_empty() {
        println!("  {} error(s) encountered.", errors.len());
    }

    Ok(())
}

/// Remove merged worktrees, and with `stale` unmerged feature worktrees idle
/// for a day.
///
/// `git worktree remove` runs without `--force`, so git refuses a worktree with
/// modified or untracked files and nothing uncommitted is lost. It used to pass
/// `--force` and include stale worktrees by default, which deleted a day-old
/// claude/ worktree's uncommitted files. A branch is deleted with `branch -d`,
/// which refuses an unmerged one, and the report says which happened.
async fn cleanup(force: bool, include_stale: bool) -> anyhow::Result<()> {
    let worktrees = parse_worktrees()?;
    let main = main_branch();
    let stale_threshold = std::time::Duration::from_secs(24 * 60 * 60);

    println!("{}", "Worktree Cleanup".bold());
    println!("{}", "\u{2500}".repeat(60));

    // Find merged worktrees
    let merged_wts: Vec<(String, String)> = worktrees
        .iter()
        .filter(|(_, branch)| {
            branch != &main && branch != "(detached)" && is_merged(branch, &main)
        })
        .cloned()
        .collect();

    // Find stale worktrees (24h+ no commits), only when asked for
    let stale_wts: Vec<(String, String)> = worktrees
        .iter()
        .filter(|(_path, branch)| {
            if !include_stale || branch == &main || branch == "(detached)" || is_merged(branch, &main) {
                return false;
            }
            // Check if branch is a feature-type branch
            let is_feature = branch.starts_with("feat/")
                || branch.starts_with("hexa/")
                || branch.starts_with("worktree-")
                || branch.starts_with("claude/");
            if !is_feature {
                return false;
            }
            // Check last commit age
            if let Ok(output) = Command::new("git")
                .args(["log", "-1", "--format=%ct", branch])
                .output()
            {
                if output.status.success() {
                    if let Ok(ts) = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        return now.saturating_sub(ts) > stale_threshold.as_secs();
                    }
                }
            }
            false
        })
        .cloned()
        .collect();

    let all_to_clean: Vec<(String, String)> = merged_wts
        .into_iter()
        .chain(stale_wts)
        .collect();

    if all_to_clean.is_empty() {
        let what = if include_stale { "merged or stale" } else { "merged" };
        println!("  {} No {what} worktrees to clean up.", "\u{2713}".green());
        if !include_stale {
            println!("  {} Unmerged worktrees idle for 24h+ are left alone; {} lists them for removal.", "\u{2022}".dimmed(), "--stale".bold());
        }
        return Ok(());
    }

    let merged_count = all_to_clean.len();
    println!("  Found {} worktree(s) to clean up:", merged_count);
    for (path, branch) in &all_to_clean {
        let status = if is_merged(branch, &main) { "merged" } else { "stale" };
        println!("    {} {} ({}) [{}]", "\u{2013}".green(), branch.cyan(), path.dimmed(), status);
    }

    if !force {
        println!(
            "\n  {} Dry run — pass {} to remove.",
            "\u{26a0}".yellow(),
            "--force".bold()
        );
        return Ok(());
    }

    let mut removed = 0usize;
    for (path, branch) in &all_to_clean {
        // No --force: git refuses a worktree with modified or untracked files.
        let wt_result = Command::new("git")
            .args(["worktree", "remove", path])
            .output()?;

        if wt_result.status.success() {
            // -d refuses an unmerged branch; say which happened.
            let br = Command::new("git").args(["branch", "-d", branch]).output();
            match br {
                Ok(o) if o.status.success() => {
                    println!("  {} Removed {} (branch {} deleted)", "\u{2713}".green(), path, branch);
                }
                Ok(o) => {
                    let why = String::from_utf8_lossy(&o.stderr);
                    let why = why.lines().next().unwrap_or("").trim();
                    println!("  {} Removed {}; branch {} kept: {}", "\u{2713}".green(), path, branch, why);
                }
                Err(e) => {
                    println!("  {} Removed {}; branch {} kept: {}", "\u{2713}".green(), path, branch, e);
                }
            }
            removed += 1;
        } else {
            let err = String::from_utf8_lossy(&wt_result.stderr);
            println!("  {} Kept {}: {}", "\u{2717}".red(), path, err.trim());
        }
    }

    println!("\n  Removed {} worktree(s).", removed);
    Ok(())
}

// ============================================================
//  Merge-team CLI surface (ADR-2026-05-08-1126 P5)
// ============================================================


