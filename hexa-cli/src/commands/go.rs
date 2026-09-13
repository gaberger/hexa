//! `hexa go` — do the next right thing.
//!
//! Examines project state and takes or suggests the best next action.
//! Safe auto-fixes (binary rebuild) are executed; everything else is suggested.

use colored::Colorize;
use std::path::Path;


pub async fn run() -> anyhow::Result<()> {
    println!("{} hexa go\n", "\u{2b21}".cyan());

    let mut actions_needed = false;

    // 1. Check if nexus is running

    // 2. Check if release binary is stale
    let rebuild_in_flight = check_binary_staleness().await;
    actions_needed |= rebuild_in_flight;

    // 3. Check for pending workplans
    actions_needed |= check_pending_workplans().await;

    // 4. Check for stale worktrees
    actions_needed |= check_stale_worktrees().await;

    // 5. Check if tests pass
    let tests = check_tests(rebuild_in_flight).await;
    println!("{}", tests.line());
    actions_needed |= tests.is_problem();

    if !actions_needed {
        println!("\n  {}", "All clear. hexa is healthy.".green().bold());
    }

    Ok(())
}

/// Check if the release binary is stale relative to HEAD commit time.
/// If stale, auto-rebuild (this is a safe auto-fix).
async fn check_binary_staleness() -> bool {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return false,
    };

    // Find the release binary
    let binary_path = cwd.join("target/release/hexa");
    if !binary_path.exists() {
        // No release binary at all — suggest building
        println!(
            "  {} no release binary {}",
            "\u{2192}".yellow(),
            "— build with: cargo build -p hexa-cli --release".dimmed()
        );
        return true;
    }

    // Get binary mtime
    let bin_mtime = match std::fs::metadata(&binary_path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };

    // Get HEAD commit timestamp
    let head_time = match get_head_commit_time(&cwd).await {
        Some(t) => t,
        None => return false,
    };

    if head_time > bin_mtime {
        match std::process::Command::new("cargo")
            .args(["build", "-p", "hexa-cli", "--release"])
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                println!(
                    "  {} binary stale — rebuild spawned (PID {})",
                    "\u{2192}".yellow(),
                    child.id()
                );
                // Drop child without waiting — cargo runs detached in background.
                // On Unix the child is reparented to init when hexa go exits.
                drop(child);
                true
            }
            Err(e) => {
                println!(
                    "  {} binary stale — rebuild spawn failed: {} {}",
                    "\u{2192}".red(),
                    e,
                    "— run manually: cargo build -p hexa-cli --release".dimmed()
                );
                true
            }
        }
    } else {
        println!("  {} binary up to date", "\u{2713}".green());
        false
    }
}

/// Get the commit time of HEAD as a SystemTime.
async fn get_head_commit_time(cwd: &Path) -> Option<std::time::SystemTime> {
    let output = tokio::process::Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .current_dir(cwd)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let timestamp_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let secs: u64 = timestamp_str.parse().ok()?;
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

/// Check for pending workplans (status=planned with remaining tasks).
async fn check_pending_workplans() -> bool {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return false,
    };

    let workplans_dir = cwd.join("docs/workplans");
    if !workplans_dir.is_dir() {
        println!("  {} no workplans directory", "\u{2713}".green());
        return false;
    }

    let mut pending = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&workplans_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // Look for workplan files with incomplete tasks
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    let has_pending_tasks = json["phases"]
                        .as_array()
                        .map(|phases| {
                            phases.iter().any(|phase| {
                                phase["tasks"]
                                    .as_array()
                                    .map(|tasks| {
                                        tasks.iter().any(|t| {
                                            t["status"].as_str() == Some("planned")
                                                || t["status"].as_str() == Some("in_progress")
                                        })
                                    })
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);

                    if has_pending_tasks {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            pending.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    if pending.is_empty() {
        println!("  {} workplans consistent", "\u{2713}".green());
        false
    } else {
        for wp in &pending {
            println!(
                "  {} pending workplan: {} {}",
                "\u{2192}".yellow(),
                wp,
                "— hexa plan execute <file>".dimmed()
            );
        }
        true
    }
}

/// Check for stale worktrees (>24h with no commits).
async fn check_stale_worktrees() -> bool {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return false,
    };

    let output = tokio::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&cwd)
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => {
            println!("  {} worktrees ok", "\u{2713}".green());
            return false;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let now = std::time::SystemTime::now();
    let twenty_four_hours = std::time::Duration::from_secs(24 * 60 * 60);
    let mut stale_count = 0u32;

    // Parse porcelain output — each worktree block starts with "worktree <path>"
    for block in stdout.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.is_empty() {
            continue;
        }

        let wt_path = lines
            .iter()
            .find_map(|l| l.strip_prefix("worktree "))
            .unwrap_or("");

        // Skip the main worktree (bare = true or the repo root)
        if lines.contains(&"bare") {
            continue;
        }
        // Skip the main worktree itself
        if wt_path == cwd.to_str().unwrap_or("") {
            continue;
        }
        if wt_path.is_empty() {
            continue;
        }

        // Check last commit time in worktree
        let last_commit = tokio::process::Command::new("git")
            .args(["log", "-1", "--format=%ct"])
            .current_dir(wt_path)
            .output()
            .await;

        if let Ok(lc) = last_commit {
            if lc.status.success() {
                let ts_str = String::from_utf8_lossy(&lc.stdout).trim().to_string();
                if let Ok(secs) = ts_str.parse::<u64>() {
                    let commit_time =
                        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
                    if let Ok(age) = now.duration_since(commit_time) {
                        if age > twenty_four_hours {
                            stale_count += 1;
                        }
                    }
                }
            }
        }
    }

    if stale_count == 0 {
        println!("  {} worktrees ok", "\u{2713}".green());
        false
    } else {
        println!(
            "  {} {} stale worktree{} {}",
            "\u{2192}".yellow(),
            stale_count,
            if stale_count == 1 { "" } else { "s" },
            "(hexa worktree cleanup --force)".dimmed()
        );
        true
    }
}

/// What a check found. A check that could not run is not a failing one:
/// it is one the operator cannot act on, and a next-action list is for
/// things to act on (ADR-2609131800 §2).
#[derive(Debug, Clone, PartialEq)]
pub enum Checked {
    /// The activity ran and was clean.
    Ok(String),
    /// The activity ran and found something to fix.
    Problem(String),
    /// The activity did not run. The string says why.
    Unknown(String),
}

impl Checked {
    /// Only a real problem belongs in the next-action list.
    pub fn is_problem(&self) -> bool {
        matches!(self, Checked::Problem(_))
    }

    /// The line, marked by what it is.
    pub fn line(&self) -> String {
        match self {
            Checked::Ok(m) => format!("  {} {m}", "\u{2713}".green()),
            Checked::Problem(m) => format!("  {} {m}", "\u{2192}".red()),
            Checked::Unknown(m) => format!("  {} {m}", "\u{25cb}".dimmed()),
        }
    }
}

/// Do the test binaries compile?
///
/// This compiles and runs nothing, so it reports on compilation
/// (ADR-2609131800 §1). The old failure branch said "tests failing" for a
/// check that had never run a test, and sent an operator to a green suite.
async fn check_tests(rebuild_in_flight: bool) -> Checked {
    // A rebuild this verb itself spawned holds the build lock; racing it
    // and blaming the code is the fault this replaces (§3).
    if rebuild_in_flight {
        return Checked::Unknown("tests not compiled — a rebuild is in flight".to_string());
    }
    let Ok(cwd) = std::env::current_dir() else {
        return Checked::Unknown("tests not compiled — no working directory".to_string());
    };

    let test_result = tokio::process::Command::new("cargo")
        .args(["test", "--workspace", "--no-run", "--quiet"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    match test_result {
        Ok(status) if status.success() => Checked::Ok("tests compile".to_string()),
        Ok(_) => Checked::Problem("tests do not compile — cargo test --workspace --no-run".to_string()),
        Err(e) => Checked::Unknown(format!("could not check whether tests compile: {e}")),
    }
}

#[cfg(test)]
mod checked_tests {
    use super::{check_tests, Checked};

    /// ADR-2609131800 §1: a check names the activity it performed. This one
    /// compiles and never runs a test, so neither branch may say "tests
    /// failing" — that sent an operator to a suite of 819 passing tests.
    #[test]
    fn a_compile_check_reports_compilation_never_test_results() {
        for c in [
            Checked::Ok("tests compile".into()),
            Checked::Problem("tests do not compile — cargo test --workspace --no-run".into()),
            Checked::Unknown("tests not compiled — a rebuild is in flight".into()),
        ] {
            let line = c.line();
            assert!(!line.contains("tests failing"), "names an activity that never happened: {line}");
            assert!(!line.contains("tests pass"), "this check runs nothing: {line}");
        }
    }

    /// §2: only a real problem is a next action. A check that could not run
    /// is not something the operator can act on.
    #[test]
    fn only_a_problem_counts_as_a_next_action() {
        assert!(Checked::Problem("x".into()).is_problem());
        assert!(!Checked::Ok("x".into()).is_problem());
        assert!(!Checked::Unknown("x".into()).is_problem(), "an unrunnable check accuses nothing");
    }

    /// §3: the verb does not race the rebuild it just spawned — the exact
    /// sequence that produced the false report.
    #[tokio::test]
    async fn a_rebuild_in_flight_skips_the_check_rather_than_racing_it() {
        let got = check_tests(true).await;
        assert!(matches!(got, Checked::Unknown(_)), "{got:?}");
        assert!(!got.is_problem(), "a skipped check is not a failing one");
        assert!(got.line().contains("rebuild is in flight"), "{}", got.line());
    }

    /// The three states are visually distinct, so a list of them reads.
    #[test]
    fn each_state_is_marked_differently() {
        let marks: Vec<char> = [
            Checked::Ok("a".into()),
            Checked::Problem("b".into()),
            Checked::Unknown("c".into()),
        ]
        .iter()
        .map(|c| c.line().trim_start().chars().next().unwrap())
        .collect();
        assert_eq!(marks.len(), 3);
        assert!(marks[0] != marks[1] && marks[1] != marks[2] && marks[0] != marks[2], "{marks:?}");
    }
}
