//! `hexa test` — Full-stack integration testing from the CLI.
//!
//! Runs unit tests, linters, architecture checks and inference-provider
//! probes. The service-health, API-integration, swarm-coordination, E2E-browser
//! and MCP-parity suites went with the daemon they tested (ADR-2608241500).
//!
//! Usage:
//!   hexa test unit         # Unit tests only
//!   hexa test arch         # Architecture checks only
//!   hexa test inference    # Probe the configured inference backends
//!   hexa test all          # Everything

use hexa_infer::ports::EndpointRegistry;
use std::cmp::Reverse;
use std::process::Command;
use std::time::Instant;

use clap::Subcommand;
use chrono::Utc;
use colored::Colorize;
use serde::Serialize;
use tabled::Tabled;

use crate::fmt::{HexTable, status_badge, truncate};

#[derive(Subcommand)]
pub enum TestAction {
    /// Run all unit tests across Rust crates
    Unit,
    /// Check architecture health (boundaries, deps, dead code)
    Arch,
    /// Test self-hosted inference providers (Ollama, vLLM)
    Inference,
    /// Run all linters (clippy + tsc)
    Lint,
    /// Run full integration tests (unit + arch + services + inference + swarm)
    All,
    /// Show recent test run history
    History,
    /// Show test pass rate trends
    Trends,
}

/// The agent id for this session, from `~/.hexa/sessions/agent-<id>.json`.
///
/// Lifted out of `nexus_client` (deleted with the daemon it spoke to). It never
/// touched the network: the session file is written by the hook.
fn read_session_agent_id() -> Option<String> {
    let sessions = dirs::home_dir()?.join(".hexa/sessions");
    let session_id = std::env::var("CLAUDE_SESSION_ID").ok().filter(|s| !s.is_empty())?;
    let text = std::fs::read_to_string(sessions.join(format!("agent-{session_id}.json"))).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("agentId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// A single test result entry with structured metadata.
#[derive(Debug, Clone, Serialize)]
struct TestResultEntry {
    category: String,
    name: String,
    status: String,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

struct TestResults {
    pass: u32,
    fail: u32,
    skip: u32,
    results: Vec<TestResultEntry>,
    session_start: Instant,
    /// Tracks the current test category (set by section headers).
    current_category: String,
}

impl TestResults {
    fn new() -> Self {
        Self {
            pass: 0,
            fail: 0,
            skip: 0,
            results: Vec::new(),
            session_start: Instant::now(),
            current_category: String::from("general"),
        }
    }

    /// Set the current category for subsequent check/skip calls.
    fn set_category(&mut self, category: &str) {
        self.current_category = category.to_string();
    }

    fn check(&mut self, label: &str, ok: bool) {
        if ok {
            println!("  {} {}", "✓".green(), label);
            self.pass += 1;
            self.results.push(TestResultEntry {
                category: self.current_category.clone(),
                name: label.to_string(),
                status: "pass".to_string(),
                duration_ms: 0,
                error_message: None,
            });
        } else {
            println!("  {} {}", "✗".red(), label);
            self.fail += 1;
            self.results.push(TestResultEntry {
                category: self.current_category.clone(),
                name: label.to_string(),
                status: "fail".to_string(),
                duration_ms: 0,
                error_message: Some(format!("{} failed", label)),
            });
        }
    }

    /// Record a tool outcome. A missing tool is skipped with its name, never
    /// counted as a failure (ADR-2609122048).
    fn record(&mut self, label: &str, outcome: ToolRun, tool: &str) {
        match outcome {
            ToolRun::Passed => self.check(label, true),
            ToolRun::Failed => self.check(label, false),
            ToolRun::Missing => self.skip(&format!("{label} ({tool} not installed)")),
        }
    }

    fn skip(&mut self, label: &str) {
        println!("  {} {} (skipped)", "○".yellow(), label);
        self.skip += 1;
        self.results.push(TestResultEntry {
            category: self.current_category.clone(),
            name: label.to_string(),
            status: "skip".to_string(),
            duration_ms: 0,
            error_message: None,
        });
    }

    fn summary(&self) -> bool {
        let total = self.pass + self.fail + self.skip;
        println!();
        // Nothing ran. A suite that executed no test has not passed, whatever
        // the reason it could not run (ADR-2609122048).
        if self.fail == 0 && self.pass == 0 && self.skip > 0 {
            println!(
                "  {}: {} skipped, nothing executed (of {})",
                "NOTHING RAN".yellow().bold(),
                self.skip,
                total
            );
            return false;
        }
        if self.fail == 0 {
            println!(
                "  {}: {} passed, {} skipped, {} failed (of {})",
                "ALL PASS".green().bold(),
                self.pass,
                self.skip,
                self.fail,
                total
            );
            true
        } else {
            println!(
                "  {}: {} passed, {} skipped, {} failed (of {})",
                "FAILURES".red().bold(),
                self.pass,
                self.skip,
                self.fail,
                total
            );
            false
        }
    }

    /// Build a complete test session JSON object for persistence.
    fn to_session_json(&self) -> serde_json::Value {
        let duration_ms = self.session_start.elapsed().as_millis() as u64;
        let total = self.pass + self.fail + self.skip;
        let overall_status = if self.fail == 0 { "pass" } else { "fail" };

        // Reads ~/.hexa/sessions/agent-<id>.json — a local file, not the
        // daemon's roster.
        let agent_id = read_session_agent_id().unwrap_or_else(|| "unknown".to_string());

        // Git metadata
        let commit_hash = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        let branch = Command::new("git")
            .args(["branch", "--show-current"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        let now = Utc::now();
        let started_at = now - chrono::Duration::milliseconds(duration_ms as i64);

        serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "agent_id": agent_id,
            "commit_hash": commit_hash,
            "branch": branch,
            "started_at": started_at.to_rfc3339(),
            "finished_at": now.to_rfc3339(),
            "trigger": "manual",
            "overall_status": overall_status,
            "pass_count": self.pass,
            "fail_count": self.fail,
            "skip_count": self.skip,
            "total_count": total,
            "duration_ms": duration_ms,
            "results": self.results,
        })
    }
}

/// Append a test session JSON to ~/.hexa/test-sessions/{YYYY-MM-DD}.jsonl
fn persist_to_local_file(session_json: &serde_json::Value) {
    let Some(home) = dirs::home_dir() else { return };
    let dir = home.join(".hexa/test-sessions");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let file_path = dir.join(format!("{}.jsonl", date));
    let line = match serde_json::to_string(session_json) {
        Ok(s) => s,
        Err(_) => return,
    };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
    {
        let _ = writeln!(f, "{}", line);
    }
}

pub async fn run(action: TestAction) -> anyhow::Result<()> {
    let mut results = TestResults::new();

    match action {
        TestAction::Unit => {
            run_unit_tests(&mut results);
        }
        TestAction::Arch => {
            run_arch_checks(&mut results).await;
        }
        TestAction::Inference => {
            run_inference_tests(&mut results).await;
        }
        TestAction::Lint => {
            run_lint_checks(&mut results);
        }
        TestAction::All => {
            run_unit_tests(&mut results);
            println!();
            run_lint_checks(&mut results);
            println!();
            run_arch_checks(&mut results).await;
            println!();
            run_inference_tests(&mut results).await;
        }
        TestAction::History => {
            return run_history().await;
        }
        TestAction::Trends => {
            return run_trends().await;
        }
    }

    println!("\n{}", "══════════════════════════════════════════".cyan());
    let ok = results.summary();
    println!("{}", "══════════════════════════════════════════".cyan());

    // Fire-and-forget: persist test session results
    let session_json = results.to_session_json();
    persist_to_local_file(&session_json);

    // Regression check: compare with previous session on same branch

    if ok {
        Ok(())
    } else if results.fail == 0 {
        anyhow::bail!("nothing ran: {} check(s) skipped", results.skip)
    } else {
        anyhow::bail!("{} test(s) failed", results.fail)
    }
}

// ── Unit Tests ──────────────────────────────────────

fn run_unit_tests(r: &mut TestResults) {
    println!("{}", "── Unit Tests ──".cyan());
    r.set_category("unit");

    // The surviving workspace crates.
    for crate_name in
        &["hexa-core", "hexa-exec", "hexa-infer", "hexa-analysis", "hexa-graph", "hexa-git"]
    {
        r.record(&format!("{crate_name} tests pass"), cargo_test(crate_name, None), "cargo");
    }

    r.record("hexa-cli compiles", cargo_check("hexa-cli"), "cargo");

}

// ── Architecture Checks ─────────────────────────────

async fn run_arch_checks(r: &mut TestResults) {
    println!("{}", "── Architecture Health ──".cyan());
    r.set_category("architecture");

    // The analyzer is this binary. A category that cannot run it fails; it
    // does not quietly pass on something else (ADR-2609122048).
    let report = match run_own_analyze() {
        Ok(v) => v,
        Err(e) => {
            r.check(&format!("hexa analyze runs ({e})"), false);
            return;
        }
    };

    let count = |key: &str| -> i64 {
        report
            .get("score_components")
            .and_then(|c| c.get(key))
            .and_then(|v| v.as_i64())
            .unwrap_or(-1)
    };
    let grade = report.get("grade").and_then(|g| g.as_str()).unwrap_or("");
    let score = report.get("score").and_then(|v| v.as_i64()).unwrap_or(-1);

    r.check(
        &format!("Architecture grade A or better (got {grade}, score {score})"),
        grade.starts_with('A'),
    );
    for (label, key) in [
        ("Zero boundary violations", "violations"),
        ("Zero circular dependencies", "circular_deps"),
        ("Zero dead exports", "dead_exports"),
        ("Zero unused ports", "unused_ports"),
    ] {
        let n = count(key);
        r.check(&format!("{label} (got {n})"), n == 0);
    }
}

/// Run this binary's own `analyze . --json` and parse it.
///
/// `std::env::current_exe()` is the only way that cannot pick up a different
/// implementation. Locating the analyzer through a package manager, a source
/// path or PATH is what let this category pass while checking nothing.
fn run_own_analyze() -> Result<serde_json::Value, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate self: {e}"))?;
    let out = Command::new(&exe)
        .args(["analyze", ".", "--json"])
        .env("HEXA_INTERNAL", "1")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("unreadable analyze output: {e}"))
}

// ── Inference Tests ─────────────────────────────────

async fn run_inference_tests(r: &mut TestResults) {
    println!("{}", "── Inference Providers ──".cyan());
    r.set_category("inference");

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    // Check env-configured providers
    let ollama_host = std::env::var("HEXA_OLLAMA_HOST").ok();
    let ollama_model = std::env::var("HEXA_OLLAMA_MODEL").ok();

    if let Some(ref host) = ollama_host {
        let tags_url = format!("{}/api/tags", host.trim_end_matches('/'));
        let reachable = http.get(&tags_url).send().await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        r.check(&format!("Ollama reachable at {}", host), reachable);

        if reachable {
            if let Some(ref model) = ollama_model {
                // Quick inference test
                let chat_url = format!("{}/v1/chat/completions", host.trim_end_matches('/'));
                let body = serde_json::json!({
                    "model": model,
                    "messages": [{"role": "user", "content": "Reply with just 'ok'"}],
                    "max_tokens": 10,
                });
                let start = std::time::Instant::now();
                let infer_ok = http.post(&chat_url).json(&body).send().await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                let latency = start.elapsed().as_millis();
                r.check(&format!("Inference {} ({}ms)", model, latency), infer_ok);
            }
        }
    } else {
        // Try auto-discover bazzite
        let discover_hosts = ["http://bazzite:11434", "http://127.0.0.1:11434"];
        let mut found = false;
        for host in &discover_hosts {
            let tags_url = format!("{}/api/tags", host);
            if http.get(&tags_url).send().await.map(|r| r.status().is_success()).unwrap_or(false) {
                r.check(&format!("Ollama discovered at {}", host), true);
                found = true;
                break;
            }
        }
        if !found {
            r.skip("No Ollama found (set HEXA_OLLAMA_HOST to test)");
        }
    }

    // Anthropic — optional, not a failure if missing. The daemon's vault
    // fallback is gone: a key is an environment variable now.
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        r.check("Anthropic API key configured", true);
    } else {
        r.skip("Anthropic API key not set (optional)");
    }

    // Registered backends come from ~/.hexa/inference-servers.json, which the
    // daemon used to mirror into SpacetimeDB and serve back over HTTP.
    let endpoints = hexa_infer::endpoints().load();
    if endpoints.is_empty() {
        r.skip("No inference backends registered (hexa config inference add)");
    } else {
        r.check(&format!("{} inference backend(s) registered", endpoints.len()), true);
    }
}

// ── Integration Tests ───────────────────────────────

// ── Agent Guard Helpers ─────────────────────────────

// ── Lint Checks ────────────────────────────────────

/// Find the hexa project root — the directory that contains both `Cargo.toml`
/// and a `spacetime-modules/` subdirectory. Tries the hexa binary location
/// first (reliable), then walks up from CWD as a fallback.
fn locate_workspace_root() -> Option<std::path::PathBuf> {
    // Primary: hexa binary lives at <root>/target/debug/hexa — go up 3 levels.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
            if root.join("spacetime-modules").is_dir() {
                return Some(root.to_path_buf());
            }
        }
    }
    // Fallback: walk up from CWD looking for a dir with spacetime-modules/.
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("spacetime-modules").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn run_lint_checks(r: &mut TestResults) {
    println!("{}", "── Lint ──".cyan());
    r.set_category("lint");

    // Rust workspace clippy
    r.record(
        "cargo clippy (workspace)",
        run_cargo(&["clippy", "--workspace", "--", "-D", "warnings"]),
        "cargo",
    );

    // A TypeScript check belongs to a project that has TypeScript. Running it
    // in a Rust-only repository failed on "Script not found" every time.
    let root = locate_workspace_root().unwrap_or_else(|| std::path::PathBuf::from("."));
    if !root.join("package.json").is_file() {
        return;
    }
    let tsc = match Command::new("bun").args(["run", "check"]).status() {
        Ok(st) if st.success() => ToolRun::Passed,
        Ok(_) => ToolRun::Failed,
        Err(_) => ToolRun::Missing,
    };
    r.record("bun run check (TypeScript)", tsc, "bun");
}

// ── E2E Browser Tests ──────────────────────────────

// ── Dashboard Tests ─────────────────────────────────

// ── CLI-MCP Parity (ADR-019) ────────────────────────

// ── History ─────────────────────────────────────────

/// Represents a single test session for display purposes.
#[derive(Debug, serde::Deserialize)]
struct TestSessionRecord {
    #[serde(default)]
    commit_hash: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    overall_status: String,
    #[serde(default)]
    pass_count: u32,
    #[serde(default)]
    fail_count: u32,
    #[serde(default)]
    skip_count: u32,
    #[serde(default)]
    duration_ms: u64,
}

async fn run_history() -> anyhow::Result<()> {
    println!("{}", "── Test Run History ──".cyan());
    println!();

    let sessions = load_sessions_from_local(10);

    let sessions = match sessions {
        Some(s) if !s.is_empty() => s,
        _ => {
            println!("  No test history found.");
            return Ok(());
        }
    };

    #[derive(Tabled)]
    struct HistoryRow {
        #[tabled(rename = "Commit")]
        commit: String,
        #[tabled(rename = "Branch")]
        branch: String,
        #[tabled(rename = "Status")]
        status: String,
        #[tabled(rename = "Pass")]
        pass: u32,
        #[tabled(rename = "Fail")]
        fail: u32,
        #[tabled(rename = "Skip")]
        skip: u32,
        #[tabled(rename = "Duration")]
        duration: String,
    }

    let rows: Vec<HistoryRow> = sessions
        .iter()
        .map(|s| {
            let short_commit = if s.commit_hash.len() >= 7 {
                s.commit_hash[..7].to_string()
            } else {
                s.commit_hash.clone()
            };
            HistoryRow {
                commit: short_commit,
                branch: truncate(&s.branch, 12),
                status: status_badge(&s.overall_status),
                pass: s.pass_count,
                fail: s.fail_count,
                skip: s.skip_count,
                duration: format_duration(s.duration_ms),
            }
        })
        .collect();

    println!("{}", HexTable::render(&rows));
    Ok(())
}

fn load_sessions_from_local(limit: usize) -> Option<Vec<TestSessionRecord>> {
    let home = dirs::home_dir()?;
    let dir = home.join(".hexa/test-sessions");
    if !dir.exists() {
        return None;
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .collect();

    // Sort by filename descending (YYYY-MM-DD.jsonl — newest first)
    entries.sort_by_key(|e| Reverse(e.file_name()));

    let mut sessions = Vec::new();
    for entry in entries {
        if sessions.len() >= limit {
            break;
        }
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            let mut file_sessions: Vec<TestSessionRecord> = content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<TestSessionRecord>(l).ok())
                .collect();
            file_sessions.reverse();
            for s in file_sessions {
                if sessions.len() >= limit {
                    break;
                }
                sessions.push(s);
            }
        }
    }
    if sessions.is_empty() {
        None
    } else {
        Some(sessions)
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

// ── Trends ──────────────────────────────────────────

/// Per-category trend data.
struct CategoryTrend {
    category: String,
    /// true = pass, false = fail for each of the last N runs.
    results: Vec<bool>,
}

async fn run_trends() -> anyhow::Result<()> {
    println!("{}", "── Test Pass Rate Trends ──".cyan());
    println!();

    let runs = 10usize;
    let trends = compute_trends_from_local(runs);

    let trends = match trends {
        Some(t) if !t.is_empty() => t,
        _ => {
            println!("  No trend data found.");
            return Ok(());
        }
    };

    #[derive(Tabled)]
    struct TrendRow {
        #[tabled(rename = "Category")]
        category: String,
        #[tabled(rename = "Last Runs")]
        bar: String,
        #[tabled(rename = "Pass Rate")]
        rate: String,
    }

    let rows: Vec<TrendRow> = trends
        .iter()
        .map(|trend| {
            let pass_count = trend.results.iter().filter(|&&b| b).count();
            let total = trend.results.len();
            let rate = if total > 0 {
                (pass_count as f64 / total as f64 * 100.0) as u32
            } else {
                0
            };

            let mut bar = String::new();
            for (i, &passed) in trend.results.iter().enumerate() {
                if i >= runs {
                    break;
                }
                if passed {
                    bar.push_str(&"█".green().to_string());
                } else {
                    bar.push_str(&"░".red().to_string());
                }
            }
            for _ in trend.results.len()..runs {
                bar.push(' ');
            }

            let rate_display = if rate == 100 {
                format!("{}%", rate).green().to_string()
            } else if rate >= 80 {
                format!("{}%", rate).yellow().to_string()
            } else {
                format!("{}%", rate).red().to_string()
            };

            TrendRow {
                category: trend.category.clone(),
                bar,
                rate: rate_display,
            }
        })
        .collect();

    println!("{}", HexTable::render(&rows));
    Ok(())
}

fn compute_trends_from_local(runs: usize) -> Option<Vec<CategoryTrend>> {
    let sessions = load_sessions_with_results(runs)?;
    if sessions.is_empty() {
        return None;
    }

    // Aggregate per category across sessions
    let mut category_runs: std::collections::BTreeMap<String, Vec<bool>> =
        std::collections::BTreeMap::new();

    for session in &sessions {
        let mut cat_pass: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        for result in &session.results {
            let cat = result.category.clone();
            let passed = result.status == "pass" || result.status == "skip";
            // A category fails if ANY test in it fails
            let entry = cat_pass.entry(cat).or_insert(true);
            if !passed {
                *entry = false;
            }
        }
        for (cat, passed) in cat_pass {
            category_runs.entry(cat).or_default().push(passed);
        }
    }

    let trends: Vec<CategoryTrend> = category_runs
        .into_iter()
        .map(|(category, results)| CategoryTrend { category, results })
        .collect();

    if trends.is_empty() {
        None
    } else {
        Some(trends)
    }
}

/// A session with full result entries, for trend computation.
#[derive(Debug, serde::Deserialize)]
struct TestSessionWithResults {
    #[serde(default)]
    results: Vec<TestResultEntryDeser>,
}

#[derive(Debug, serde::Deserialize)]
struct TestResultEntryDeser {
    #[serde(default)]
    category: String,
    #[serde(default)]
    status: String,
}

fn load_sessions_with_results(limit: usize) -> Option<Vec<TestSessionWithResults>> {
    let home = dirs::home_dir()?;
    let dir = home.join(".hexa/test-sessions");
    if !dir.exists() {
        return None;
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| Reverse(e.file_name()));

    let mut sessions = Vec::new();
    for entry in entries {
        if sessions.len() >= limit {
            break;
        }
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            let mut file_sessions: Vec<TestSessionWithResults> = content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<TestSessionWithResults>(l).ok())
                .collect();
            file_sessions.reverse();
            for s in file_sessions {
                if sessions.len() >= limit {
                    break;
                }
                sessions.push(s);
            }
        }
    }
    if sessions.is_empty() {
        None
    } else {
        Some(sessions)
    }
}

// ── Helpers ─────────────────────────────────────────

/// A tool that is not installed did not fail. It never ran.
///
/// Mapping a spawn error to `false` reported "hexa-core tests fail" seven
/// times over on a machine without cargo (ADR-2609122048).
enum ToolRun {
    Passed,
    Failed,
    Missing,
}

fn run_cargo(args: &[&str]) -> ToolRun {
    let mut cmd = Command::new("cargo");
    cmd.args(args);
    if let Some(root) = locate_workspace_root() {
        cmd.current_dir(root);
    }
    match cmd.status() {
        Ok(st) if st.success() => ToolRun::Passed,
        Ok(_) => ToolRun::Failed,
        Err(_) => ToolRun::Missing,
    }
}

fn cargo_test(crate_name: &str, extra: Option<&str>) -> ToolRun {
    let mut args = vec!["test", "-p", crate_name, "--quiet"];
    if let Some(flag) = extra {
        args.push(flag);
    }
    run_cargo(&args)
}

fn cargo_check(crate_name: &str) -> ToolRun {
    run_cargo(&["check", "-p", crate_name])
}

// ── Coordination Tests (ADR-2026-03-28-2000) ─────────────────────────────────────

