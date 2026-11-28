//! `hexa bench` — agentic inference benchmark runner (ADR-2606071734).
//!
//! Runs the benchmark corpus (`docs/benchmarks/fixtures/*.json`) through the REAL
//! evidence-gated direct executor, in-process (ADR-2608241500 P6.2 — it used
//! to POST /api/direct/execute, a route whose whole body was a pass-through to
//! the same function), and scores a capability
//! **vector** — did it edit, did it pass an independent oracle, how many steps, how long.
//! This tests whether a model can *drive the loop*, not just write a function: the gap the
//! single-turn `hexa config inference bench` misses (a model can ace codegen and still wander
//! the loop and ship nothing).
//!
//! ISOLATION (per operator directive): every fixture runs in a dedicated git **worktree**,
//! never in the operator's tree. We reuse the executor's own `isolate:true` path
//! (ADR-2606071323), which forks a `hexa/auto/<slug>` worktree as a sibling of the repo and
//! is hard-guarded against the operator branch. Because a worktree forks from *committed*
//! state, the bench commits each fixture's oracle to a throwaway `bench/<ts>` branch before
//! the run, then GCs the run's worktree+branch and resets the oracle after — so neither the
//! operator branch nor the operator working tree is ever mutated. (MicroVM isolation — via
//! `hexa sandbox` — is the heavier alternative for untrusted/destructive fixtures; tracked in
//! the ADR. Worktrees suffice for benchmarking hexa's own loop on hexa's own repo.)

use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::time::Instant;


#[derive(Subcommand)]
pub enum BenchAction {
    /// Run the agentic benchmark corpus through the direct executor and score a vector.
    Agentic {
        /// Reasoning model to benchmark (e.g. "qwen2.5-coder:14b"). Defaults to the
        /// executor's configured model when omitted.
        #[arg(short, long)]
        model: Option<String>,
        /// Corpus fixtures directory.
        #[arg(long, default_value = "docs/benchmarks/fixtures")]
        corpus: String,
        /// Comma-separated arms to run: react,fast.
        #[arg(long, default_value = "react,fast")]
        arms: String,
        /// Only run fixtures whose id contains this substring.
        #[arg(long)]
        filter: Option<String>,
        /// Include fixtures with status != "verified" (default: verified only).
        #[arg(long)]
        include_draft: bool,
    },
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    tier: String,
    instruction: String,
    target_file: String,
    oracle: Oracle,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct Oracle {
    #[serde(default)]
    setup_files: BTreeMap<String, String>,
    command: String,
}

struct ArmResult {
    id: String,
    tier: String,
    arm: String,
    did_edit: bool,
    evidence_pass: bool,
    attempts: u64,
    failure_reason: String,
    wall_ms: u128,
}

pub async fn run(action: BenchAction) -> anyhow::Result<()> {
    match action {
        BenchAction::Agentic { model, corpus, arms, filter, include_draft } => {
            // The run record (ADR-2611051850): a bench is a run, and it
            // checks out a staging branch, so "is one in progress" matters.
            let record = crate::commands::run_record::RunGuard::begin("bench", &corpus, None);
            let r = agentic(model, corpus, arms, filter, include_draft).await;
            record.finish_result(&r);
            r
        }
    }
}

async fn agentic(
    model: Option<String>,
    corpus: String,
    arms: String,
    filter: Option<String>,
    include_draft: bool,
) -> anyhow::Result<()> {
    let arms: Vec<String> = arms.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let fixtures = load_fixtures(&corpus, filter.as_deref(), include_draft)?;
    if fixtures.is_empty() {
        println!("{} no fixtures matched in {}", "⬡ bench:".yellow(), corpus);
        return Ok(());
    }
    // ADR-2611221646 §2: every evidence program must resolve before anything runs.
    if let Some(refusal) = missing_evidence_programs(&fixtures) {
        anyhow::bail!("{refusal}");
    }

    println!(
        "{} {} fixture(s) × arms [{}] · model {} · isolation: worktree",
        "⬡ agentic bench".cyan().bold(),
        fixtures.len(),
        arms.join(", "),
        model.as_deref().unwrap_or("(default)").yellow()
    );

    let orig_branch = git_capture(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    if orig_branch == "HEAD" {
        anyhow::bail!("detached HEAD — checkout a branch before benching");
    }
    let bench_branch = format!("bench/{}", now_stamp());
    git_run(&["checkout", "-b", &bench_branch])?;
    println!("  {} staging branch {}", "→".dimmed(), bench_branch.dimmed());

    let mut results: Vec<ArmResult> = Vec::new();
    for fx in &fixtures {
        for arm in &arms {
            results.push(run_one(fx, arm, model.as_deref()).await);
        }
    }

    // Teardown: back to the operator branch, drop the staging branch, prune worktrees.
    git_run(&["checkout", &orig_branch])?;
    let _ = git_run(&["branch", "-D", &bench_branch]);
    let _ = git_run(&["worktree", "prune"]);

    print_vector(&results);
    Ok(())
}

async fn run_one(fx: &Fixture, arm: &str, model: Option<&str>) -> ArmResult {
    let fast = arm == "fast";

    // 1. Materialize + COMMIT the oracle so the executor's worktree (forked from HEAD)
    //    contains it. Commit only the oracle paths explicitly — never `git add -A`, so any
    //    unrelated working-tree changes are left untouched.
    let oracle_paths: Vec<String> = fx.oracle.setup_files.keys().cloned().collect();
    for (rel, content) in &fx.oracle.setup_files {
        let _ = write_file(rel, content);
    }
    for p in &oracle_paths {
        let _ = git_run(&["add", p]);
    }
    let committed_oracle = git_run(&["commit", "-q", "-m", &format!("bench oracle: {}", fx.id)]).is_ok();

    let auto_before = auto_branches();

    // 2. Run the real loop in an ISOLATED worktree (isolate:true → hexa/auto/<slug>).
    let task = hexa_exec::direct_exec::DirectTask {
        instruction: fx.instruction.clone(),
        file: fx.target_file.clone(),
        evidence: fx.oracle.command.clone(),
        model: model.map(str::to_string),
        max_attempts: None,
        fast,
        max_steps: None,
        isolate: Some(true),
        ..Default::default()
    };

    print!("  {:<22} {:<5} … ", fx.id.dimmed(), arm);
    use std::io::Write;
    let _ = std::io::stdout().flush();

    let t0 = Instant::now();
    let r = hexa_exec::execute_direct(task).await;
    let wall_ms = t0.elapsed().as_millis();

    let ok = r.ok;
    let ev = r.evidence_passed;
    let attempts = r.attempts as u64;
    let err = r.error.as_deref().unwrap_or("");
    let (did_edit, reason) = classify(ok, ev, err);

    let badge = if ev { "PASS".green().bold() } else { "FAIL".red().bold() };
    println!("{} edit={} {} {}ms ({})", badge, yn(did_edit), reason.dimmed(), wall_ms, attempts);

    // 3. GC the run's worktree+branch (on success the executor leaves it for `hexa worktree
    //    merge`; for a bench we only want the verdict), then undo the oracle commit.
    gc_new_worktrees(&auto_before);
    if committed_oracle {
        let _ = git_run(&["reset", "--mixed", "HEAD~1"]); // undo commit, keep files untracked
    }
    for p in &oracle_paths {
        let _ = std::fs::remove_file(p);
        // prune now-empty ancestor dirs the materialization created (remove_dir
        // only succeeds on empty dirs, so it stops at the first non-empty one).
        let mut dir = Path::new(p).parent();
        while let Some(d) = dir {
            if d.as_os_str().is_empty() || d == Path::new(".") || std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }

    ArmResult {
        id: fx.id.clone(),
        tier: fx.tier.clone(),
        arm: arm.to_string(),
        did_edit,
        evidence_pass: ev,
        attempts,
        failure_reason: reason,
        wall_ms,
    }
}

/// Map the executor's response to (did_edit, failure_reason).
fn classify(ok: bool, ev: bool, err: &str) -> (bool, String) {
    if ok && ev {
        return (true, "pass".to_string());
    }
    let e = err.to_lowercase();
    if e.contains("no edit") {
        (false, "no_edit".to_string())
    } else if e.contains("no progress") {
        (true, "no_progress".to_string())
    } else if e.contains("exhausted") {
        (true, "max_steps".to_string())
    } else if e.contains("inference") || e.contains("transport") {
        (false, "inference_error".to_string())
    } else {
        (true, "evidence_fail".to_string())
    }
}

/// Snapshot of `hexa/auto/*` branch names (the executor's isolated run branches).
fn auto_branches() -> HashSet<String> {
    git_capture(&["branch", "--list", "hexa/auto/*", "--format=%(refname:short)"])
        .unwrap_or_default()
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Remove any `hexa/auto/*` worktree+branch created since `before` (the run's leftovers).
fn gc_new_worktrees(before: &HashSet<String>) {
    let after = auto_branches();
    for branch in after.difference(before) {
        if let Some(path) = worktree_path_for(branch) {
            let _ = git_run(&["worktree", "remove", "--force", &path]);
        }
        let _ = git_run(&["branch", "-D", branch]);
    }
    let _ = git_run(&["worktree", "prune"]);
}

/// Find the worktree path checked out to a given branch, via `git worktree list --porcelain`.
fn worktree_path_for(branch: &str) -> Option<String> {
    let listing = git_capture(&["worktree", "list", "--porcelain"]).ok()?;
    let want = format!("refs/heads/{branch}");
    let mut cur_path: Option<String> = None;
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            cur_path = Some(p.to_string());
        } else if let Some(b) = line.strip_prefix("branch ") {
            if b == want {
                return cur_path;
            }
        }
    }
    None
}

fn print_vector(results: &[ArmResult]) {
    println!("\n{}", "── capability vector ──".cyan().bold());
    let mut arms: Vec<&str> = results.iter().map(|r| r.arm.as_str()).collect();
    arms.sort();
    arms.dedup();
    for arm in arms {
        let rows: Vec<&ArmResult> = results.iter().filter(|r| r.arm == arm).collect();
        let n = rows.len().max(1);
        let edits = rows.iter().filter(|r| r.did_edit).count();
        let passes = rows.iter().filter(|r| r.evidence_pass).count();
        let mean_ms = rows.iter().map(|r| r.wall_ms).sum::<u128>() / n as u128;
        println!(
            "  arm {:<6}  edit_rate {:>3}%  evidence_pass {:>3}%  mean {}ms  (n={})",
            arm, edits * 100 / n, passes * 100 / n, mean_ms, rows.len()
        );
    }
    println!("\n  {}", "per fixture:".dimmed());
    for r in results {
        let mark = if r.evidence_pass { "✓".green() } else { "✗".red() };
        println!(
            "    {} {:<22} {:<4} {:<6} edit={} reason={:<14} {}ms attempts={}",
            mark, r.id, r.tier, r.arm, yn(r.did_edit), r.failure_reason, r.wall_ms, r.attempts
        );
    }
}

/// Shell builtins and keywords: always present, never looked up on `PATH`.
const SHELL_BUILTINS: &[&str] = &[
    ":", ".", "[", "[[", "]]", "alias", "break", "builtin", "case", "cd", "command", "continue", "declare", "done",
    "echo", "esac", "eval", "exec", "exit", "export", "false", "fi", "getopts", "hash", "local", "printf", "pushd",
    "popd", "pwd", "read", "readonly", "return", "set", "shift", "source", "test", "trap", "true", "type", "ulimit",
    "umask", "unalias", "unset", "wait", "{", "}", ")",
];

/// Words that start a command without being one: the program follows them.
const SHELL_PREFIXES: &[&str] = &["if", "then", "else", "elif", "while", "until", "do", "!", "time"];

/// A leading `NAME=value` assignment: an identifier, then `=`.
fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// A redirection word such as `2>/dev/null`, `>out` or `<in`: not a program.
fn is_redirection(word: &str) -> bool {
    word.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(['>', '<'])
}

/// The commands of a shell line, split at `&&`, `||`, `;`, `|`, `&` and
/// newlines outside quotes. The `&` of a redirection (`2>&1`, `&>`) is not a
/// separator.
fn command_segments(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'\'' | b'"' => quote = Some(b),
                b'\\' => i += 1,
                b'&' if (i > 0 && matches!(bytes[i - 1], b'>' | b'<')) || bytes.get(i + 1) == Some(&b'>') => {}
                b'&' | b'|' | b';' | b'\n' => {
                    segments.push(&command[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    segments.push(&command[start.min(command.len())..]);
    segments
}

/// Every program an evidence command runs: in each command between `&&`,
/// `||`, `;` and `|`, its first word after any leading `NAME=value`
/// assignments. Shell builtins are left out; they are always present.
fn evidence_programs(command: &str) -> Vec<&str> {
    let mut programs = Vec::new();
    for segment in command_segments(command) {
        let program = segment
            .split_whitespace()
            .map(|w| w.trim_start_matches(['(', '{']))
            .filter(|w| !w.is_empty())
            .find(|w| !is_assignment(w) && !is_redirection(w) && !SHELL_PREFIXES.contains(w));
        if let Some(p) = program {
            if !SHELL_BUILTINS.contains(&p) && !programs.contains(&p) {
                programs.push(p);
            }
        }
    }
    programs
}

/// Whether `program` can be run: a path that exists, or a name found on `PATH`.
fn program_resolves(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
        .unwrap_or(false)
}

/// The refusal when a selected fixture's evidence program is not on `PATH`
/// (ADR-2611221646 §2): each missing program with the fixtures that need it.
fn missing_evidence_programs(fixtures: &[Fixture]) -> Option<String> {
    let mut missing: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for fx in fixtures {
        for program in evidence_programs(&fx.oracle.command) {
            if !program_resolves(program) {
                missing.entry(program.to_string()).or_default().push(&fx.id);
            }
        }
    }
    if missing.is_empty() {
        return None;
    }
    let lines: Vec<String> = missing
        .iter()
        .map(|(program, ids)| format!("  {program} (needed by {})", ids.join(", ")))
        .collect();
    Some(format!(
        "bench refused: evidence program(s) not found on PATH, so no fixture was run:\n{}",
        lines.join("\n")
    ))
}

fn load_fixtures(dir: &str, filter: Option<&str>, include_draft: bool) -> anyhow::Result<Vec<Fixture>> {
    let path = Path::new(dir);
    if !path.is_dir() {
        anyhow::bail!("corpus dir not found: {dir}");
    }
    let mut entries: Vec<_> = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    entries.sort();
    let mut out = Vec::new();
    for p in entries {
        let raw = std::fs::read_to_string(&p)?;
        let fx: Fixture = match serde_json::from_str(&raw) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{} skipping {}: {}", "⚠".yellow(), p.display(), e);
                continue;
            }
        };
        if let Some(f) = filter {
            if !fx.id.contains(f) {
                continue;
            }
        }
        if !include_draft && fx.status != "verified" {
            continue;
        }
        out.push(fx);
    }
    Ok(out)
}

fn write_file(rel: &str, content: &str) -> anyhow::Result<()> {
    let p = Path::new(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, content)?;
    Ok(())
}

fn git_run(args: &[&str]) -> anyhow::Result<()> {
    let st = Command::new("git").args(args).status()?;
    if !st.success() {
        anyhow::bail!("git {:?} failed", args);
    }
    Ok(())
}

fn git_capture(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("git {:?} failed", args);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn now_stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn yn(b: bool) -> colored::ColoredString {
    if b { "y".green() } else { "n".red() }
}
