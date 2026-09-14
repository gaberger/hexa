//! `hexa hook <event>` — Claude Code hook handler.
//!
//! When `hexa init` installs hooks into a project, they call back to
//! `hexa hook <event>` rather than running Node.js helper scripts.
//! This keeps hexa self-contained — no need to copy JS files around.
//!
//! Hook events receive context via environment variables set by Claude Code:
//! - `CLAUDE_PROJECT_DIR` — project root
//! - `CLAUDE_SESSION_ID` — current session
//! - `TOOL_NAME` / `TOOL_INPUT` — for PreToolUse/PostToolUse hooks
//!
//! ADR-050: Hook-Enforced Agent Lifecycle Pipeline
//! Multi-agent work runs in harness subagents, one worktree each. The hooks
//! enforce that at spawn time and report what the subagents leave behind.

pub mod punch_list;

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};


/// Extended session state file (ADR-050).
/// Persisted to ~/.hexa/sessions/agent-{sessionId}.json
#[derive(Serialize, Deserialize, Default)]
struct SessionState {
    #[serde(rename = "agentId")]
    agent_id: String,
    name: String,
    project: String,
    registered_at: String,
    /// PID of the parent `claude` process — used by the statusline to match
    /// this session file to the correct Claude instance.
    #[serde(default)]
    claude_pid: Option<u32>,
    #[serde(default)]
    workplan_id: Option<String>,
    /// The tier the last prompt was classified as: T1, T2 or T3.
    #[serde(default)]
    last_tier: Option<String>,
    #[serde(default)]
    edits: u64,
    #[serde(default)]
    phase: Option<String>,
    /// Active worktree path for current task (ADR-2026-03-23-1700)
    #[serde(default)]
    worktree_path: Option<String>,
    /// Allowed file paths for adapter boundary enforcement (ADR-2026-03-23-1700)
    #[serde(default)]
    allowed_paths: Vec<String>,
    /// RFC-3339 timestamp of last architecture fingerprint generation (ADR-2026-03-30-1200).
    /// Used to detect staleness when key project files change.
    #[serde(default)]
    fingerprint_generated_at: Option<String>,
    /// ADR-2026-04-11-0227: path to an in-flight workplan draft spawned by
    /// `hexa plan draft` (auto-invoked when a T3 prompt is detected).
    /// Cleared on SessionEnd or when the user approves/clears the draft.
    #[serde(default)]
    pending_workplan_draft: Option<String>,
}

impl SessionState {
    fn state_file_path() -> PathBuf {
        let sessions_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".hexa/sessions");
        sessions_dir.join(format!("agent-{}.json", session_key()))
    }

    fn load() -> Option<Self> {
        let path = Self::state_file_path();
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// The session's state, created on first use. Nothing else creates it:
    /// the daemon that once registered sessions is gone.
    fn load_or_new() -> Self {
        Self::load().unwrap_or_else(|| Self {
            registered_at: chrono::Utc::now().to_rfc3339(),
            ..Self::default()
        })
    }

    fn save(&self) -> Result<()> {
        let path = Self::state_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Returns true if the given path is permitted for this session.
    ///
    /// Fail-open: when `allowed_paths` is empty all paths are allowed.
    /// Cross-cutting directories (`docs/`, `tests/`, `config/`, `.hexa/`) are
    /// always allowed regardless of the allow-list.
    fn is_path_allowed(&self, path: &str) -> bool {
        const ALWAYS_ALLOWED: &[&str] = &["docs/", "tests/", "config/", ".hexa/"];
        if self.allowed_paths.is_empty() {
            return true;
        }
        if ALWAYS_ALLOWED.iter().any(|prefix| path.contains(prefix)) {
            return true;
        }
        self.allowed_paths.iter().any(|allowed| path.starts_with(allowed.as_str()))
    }
}

/// Check lifecycle enforcement mode for this project.
/// Default is "mandatory" — all hexa projects enforce the ADR → workplan → code pipeline.
/// Set "lifecycle_enforcement": "advisory" in .hexa/project.json to downgrade to warnings only.
/// The one JSON object Claude Code passes a hook on stdin: `session_id`,
/// `cwd`, `hook_event_name`, and per event `prompt`, `tool_name`,
/// `tool_input`, `agent_id`, `agent_type`. Read once per process; every hook
/// reads it from here. `Null` when stdin is a terminal or not JSON.
fn hook_payload() -> &'static serde_json::Value {
    static PAYLOAD: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    PAYLOAD.get_or_init(|| {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            return serde_json::Value::Null;
        }
        std::io::read_to_string(std::io::stdin())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(serde_json::Value::Null)
    })
}

/// The tool input for a PreToolUse/PostToolUse hook, as a JSON string.
/// Older setups exported `TOOL_INPUT` instead; that is the fallback.
fn tool_input_json() -> String {
    let v = hook_payload();
    if let Some(ti) = v.get("tool_input") {
        return ti.to_string();
    }
    if !v.is_null() {
        return v.to_string();
    }
    std::env::var("TOOL_INPUT").unwrap_or_default()
}

/// What one Claude session is called across its hooks. The payload's
/// `session_id` first; the `CLAUDE_SESSION_ID` variable if someone set it;
/// else the parent process, which is the `claude` process for the whole
/// session. Before this, the key was the hook's own pid, so every hook wrote
/// a file no other hook read, and the session state was never seen.
fn session_key() -> String {
    if let Some(id) = hook_payload().get("session_id").and_then(|v| v.as_str()) {
        if !id.is_empty() {
            return id.to_string();
        }
    }
    if let Ok(id) = std::env::var("CLAUDE_SESSION_ID") {
        if !id.is_empty() {
            return id;
        }
    }
    format!("ppid-{}", std::os::unix::process::parent_id())
}

/// Subagent types that only read. They may run anywhere.
const READ_ONLY_AGENTS: &[&str] = &["Explore", "Plan", "claude-code-guide", "code-explorer", "statusline-setup"];

fn is_read_only_agent(kind: &str) -> bool {
    READ_ONLY_AGENTS.iter().any(|t| kind.eq_ignore_ascii_case(t))
}

/// A git worktree has a `.git` file; the main checkout has a `.git` directory.
fn in_worktree(dir: &Path) -> bool {
    dir.join(".git").is_file()
}

fn enforcement_mode(project_dir: &Path) -> &'static str {
    let project_json = project_dir.join(".hexa/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if project["lifecycle_enforcement"].as_str() == Some("advisory") {
                return "advisory";
            }
        }
    }
    "mandatory"
}

/// ADR-2026-04-11-0227: Check whether auto-invoking the planner on T3
/// work-intent prompts is enabled for this project.
///
/// Precedence (highest to lowest):
/// 1. `HEXA_AUTO_PLAN` env var — `0` disables, anything else enables
/// 2. `workplan.auto_invoke.enabled` in `.hexa/project.json` — bool
/// 3. Default: `true` (opt-out, not opt-in)
///
/// The env var is checked first so individual shells can disable without
/// touching the project config file (useful for one-off debugging or for
/// CI environments that want deterministic behavior).
fn auto_plan_enabled(project_dir: &Path) -> bool {
    // Env var takes precedence
    if let Ok(val) = std::env::var("HEXA_AUTO_PLAN") {
        return val != "0" && val.to_lowercase() != "false";
    }

    // Then project config
    let project_json = project_dir.join(".hexa/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(enabled) = project["workplan"]["auto_invoke"]["enabled"].as_bool() {
                return enabled;
            }
        }
    }

    // Default: enabled
    true
}

#[derive(Subcommand)]
pub enum HookEvent {
    /// Session started — print project status
    SessionStart,
    /// Session ending — cleanup
    SessionEnd,
    /// Before a Write/Edit/MultiEdit — validate hexa boundaries
    PreEdit,
    /// After a Write/Edit/MultiEdit
    PostEdit,
    /// Before a Bash command
    PreBash,
    /// User submitted a prompt — route/classify
    Route,
    /// Before an Agent tool call — a code-writing subagent needs its own worktree
    PreAgent,
    /// Subagent spawned — record it, and say if it is not in a worktree
    SubagentStart,
    /// Subagent completed — auto-complete task
    SubagentStop,
    /// Before a tool call — record insights the transcript carries
    ObservePre,
    /// After a tool call — record insights the transcript carries
    ObservePost,
    /// On Stop — record insights the transcript carries
    ObserveStop,
}

pub async fn run(event: HookEvent) -> Result<()> {
    // A claude that hexa itself started (harden's reviewers, the frontier
    // candidate in the loop) carries HEXA_INTERNAL. Its prompts are hexa's,
    // not a person's: sizing them, drafting workplans from them, gating
    // their edits on a recorded gate, all of that is noise at best. The one
    // hook that stays on is pre-bash, which stops destructive commands
    // whoever issues them.
    if std::env::var("HEXA_INTERNAL").map(|v| !v.is_empty()).unwrap_or(false)
        && !matches!(event, HookEvent::PreBash)
    {
        return Ok(());
    }

    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());

    match event {
        HookEvent::SessionStart => session_start(&project_dir).await,
        HookEvent::SessionEnd => session_end(&project_dir).await,
        HookEvent::PreEdit => pre_edit(&project_dir).await,
        HookEvent::PostEdit => post_edit(&project_dir).await,
        HookEvent::PreBash => pre_bash().await,
        HookEvent::PreAgent => pre_agent().await,
        HookEvent::Route => route(&project_dir).await,
        HookEvent::SubagentStart => subagent_start().await,
        HookEvent::SubagentStop => subagent_stop().await,
        HookEvent::ObservePre => observe("PreToolUse").await,
        HookEvent::ObservePost => observe("PostToolUse").await,
        HookEvent::ObserveStop => observe("Stop").await,
    }
}

// ── Event handlers ───────────────────────────────────────────────────

async fn session_start(project_dir: &Path) -> Result<()> {
    let project_json = project_dir.join(".hexa/project.json");

    if !project_json.exists() {
        eprintln!(
            "{} Not a hexa project (no .hexa/project.json). Run `hexa init`.",
            "\u{26a0}".yellow()
        );
        return Ok(());
    }

    let content = std::fs::read_to_string(&project_json)?;
    let project: serde_json::Value = serde_json::from_str(&content)?;

    // The manifest names the project (Cargo.toml, package.json, go.mod).
    // `.hexa/project.json` carries config, not identity.
    let name = project["name"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| crate::commands::loop_cmd::project_name(project_dir));
    let id = project["id"].as_str().unwrap_or("?");

    // Print a compact status banner
    println!("\u{2b21}  hexa \u{2014} {}", name);
    println!("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    if id != "?" {
        println!("  Project: {} ({})", name, id.get(..8).unwrap_or(id));
    }

    // ADR-060: recover context from a previous session's checkpoint.
    let _ = recover_restart_checkpoint().await;
    // ADR-050: the active workplan, from local memory.
    let _ = load_workplan_context(id).await;

    // ADR-2026-03-30-1200: Inject architecture fingerprint into Claude Code
    // context. stdout is picked up as session context — never skip this.
    println!("\n{}", fingerprint_block(id, project_dir, &name).await);
    println!("{}", crate::commands::loop_cmd::status_line(project_dir));
    for l in crate::commands::loop_cmd::awareness_lines(project_dir) {
        println!("  {l}");
    }
    let mut st = SessionState::load_or_new();
    st.project = crate::commands::loop_cmd::project_name(project_dir);
    st.name = session_key();
    let _ = st.save();

    // Check for architecture violations
    let src_dir = project_dir.join("src");
    if src_dir.exists() {
        println!("  Arch:    run `hexa analyze .` to check health");
    }

    // ADR-2026-03-22-1939: Auto-upgrade settings — ensure Agent PreToolUse hook exists
    ensure_agent_hook(project_dir);

    Ok(())
}

/// Generate a minimal architecture fingerprint block from local project files.
///
/// Used when no cached fingerprint exists.
/// Reads go.mod / Cargo.toml / package.json for language detection and any
/// active workplan for objective. Output matches the injection format from
/// ADR-2026-03-30-1200 §3, trimmed to the most essential fields.
fn minimal_fingerprint_block(project_dir: &Path, project_name: &str) -> String {
    minimal_fingerprint_block_inner(project_dir, project_name)
}

fn minimal_fingerprint_block_inner(project_dir: &Path, project_name: &str) -> String {
    let mut language = "unknown".to_string();
    let mut framework = "unknown".to_string();
    let mut output_type = "unknown".to_string();
    let mut objective = String::new();

    // Detect language + framework
    if project_dir.join("Cargo.toml").exists() {
        language = "rust".to_string();
        // Peek at Cargo.toml for binary vs library
        if let Ok(ct) = std::fs::read_to_string(project_dir.join("Cargo.toml")) {
            if ct.contains("[[bin]]") || ct.contains("[package]") {
                output_type = "binary".to_string();
            }
            if ct.contains("axum") {
                framework = "axum".to_string();
                output_type = "web-api".to_string();
            } else if ct.contains("clap") {
                framework = "clap".to_string();
                output_type = "cli".to_string();
            }
        }
    } else if project_dir.join("go.mod").exists() {
        language = "go".to_string();
        if let Ok(gm) = std::fs::read_to_string(project_dir.join("go.mod")) {
            if gm.contains("gin-gonic") {
                framework = "gin".to_string();
                output_type = "web-api".to_string();
            } else {
                framework = "stdlib".to_string();
            }
        }
    } else if project_dir.join("package.json").exists() {
        language = "typescript".to_string();
        if let Ok(pj) = std::fs::read_to_string(project_dir.join("package.json")) {
            if pj.contains("\"react\"") { framework = "react".to_string(); output_type = "web-app".to_string(); }
            else if pj.contains("\"next\"") { framework = "next.js".to_string(); output_type = "web-app".to_string(); }
            else if pj.contains("\"axum\"") { framework = "axum".to_string(); }
        }
    }

    // Extract objective from most recent workplan
    let workplans_dir = project_dir.join("docs/workplans");
    if workplans_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&workplans_dir) {
            let mut files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .collect();
            // Sort by modification time descending — most recent first
            files.sort_by_key(|e| std::cmp::Reverse(
                e.metadata().and_then(|m| m.modified()).ok()
            ));
            'outer: for entry in files.iter().take(3) {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(wp) = serde_json::from_str::<serde_json::Value>(&content) {
                        for field in &["objective", "description", "title"] {
                            if let Some(s) = wp[field].as_str().filter(|s| !s.is_empty()) {
                                objective = s.chars().take(120).collect();
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut block = format!(
        "## Project Architecture Context\n\
         Project: {} | Language: {} | Framework: {} | Output: {}\n",
        project_name, language, framework, output_type
    );
    if !objective.is_empty() {
        block.push_str(&format!("Objective: {}\n", objective));
    }
    let note = "Note: run `hexa analyze .` for the full architecture report.";
    block.push_str(&format!("{}\n---", note));
    block
}

/// Ensure `.claude/settings.json` has the Agent PreToolUse hook (ADR-2026-03-22-1939).
/// If the Agent matcher is missing, inject it automatically on session start.
/// This upgrades existing projects without requiring `hexa init --force`.
fn ensure_agent_hook(project_dir: &std::path::Path) {
    let settings_path = project_dir.join(".claude/settings.json");
    if !settings_path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Quick check: if "pre-agent" is already in the file, nothing to do
    if content.contains("pre-agent") {
        return;
    }

    let mut settings: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Inject the Agent matcher into PreToolUse array
    if let Some(pre_tool_use) = settings
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PreToolUse"))
        .and_then(|p| p.as_array_mut())
    {
        pre_tool_use.push(serde_json::json!({
            "matcher": "Agent",
            "hooks": [{
                "type": "command",
                "command": "hexa hook pre-agent",
                "timeout": 3000
            }]
        }));

        if let Ok(updated) = serde_json::to_string_pretty(&settings) {
            if std::fs::write(&settings_path, &updated).is_ok() {
                println!("  Hooks:   {} Agent enforcement auto-installed (ADR-2026-03-22-1939)", "\u{2713}".green());
            }
        }
    }
}

/// SubagentStart — record the subagent, and say so if a code-writing one is
/// not in its own worktree. The gate is `pre-agent`; this is the receipt.
async fn subagent_start() -> Result<()> {
    let v = hook_payload().clone();
    let agent_type = v["agent_type"].as_str().unwrap_or("").to_string();
    let agent_id = v["agent_id"].as_str().unwrap_or("").to_string();
    let cwd = v["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let isolated = in_worktree(&cwd);
    if !isolated && !agent_type.is_empty() && !is_read_only_agent(&agent_type) {
        eprintln!(
            "[hexa hook] subagent {} ({}) is running in the main checkout, not a worktree; its edits land on this branch",
            agent_id, agent_type
        );
    }
    hexa_exec::local_store::persist_run(&serde_json::json!({
        "kind": "subagent",
        "event": "start",
        "agent_id": agent_id,
        "agent_type": agent_type,
        "cwd": cwd.display().to_string(),
        "in_worktree": isolated,
        "ts": chrono::Utc::now().to_rfc3339(),
    }));
    Ok(())
}

/// SubagentStop — record the stop, then name every worktree branch that
/// holds commits this branch does not, with the command that lands them.
/// Nothing is merged here. A merge is a decision, and the hook prints the
/// facts it needs.
async fn subagent_stop() -> Result<()> {
    let v = hook_payload().clone();
    hexa_exec::local_store::persist_run(&serde_json::json!({
        "kind": "subagent",
        "event": "stop",
        "agent_id": v["agent_id"].as_str().unwrap_or(""),
        "agent_type": v["agent_type"].as_str().unwrap_or(""),
        "ts": chrono::Utc::now().to_rfc3339(),
    }));
    for (path, branch, ahead) in worktrees_with_unmerged_commits() {
        println!(
            "worktree {} on {} holds {} commit{} not on this branch; land them with: hexa dev worktree merge {}",
            path,
            branch,
            ahead,
            if ahead == 1 { "" } else { "s" },
            branch
        );
    }
    Ok(())
}

/// `(path, branch, commits ahead of HEAD)` for every linked worktree whose
/// branch has commits HEAD does not.
fn worktrees_with_unmerged_commits() -> Vec<(String, String, usize)> {
    let git = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new("git").args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let Some(listing) = git(&["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let main = git(&["rev-parse", "--show-toplevel"]).unwrap_or_default();
    let mut out = Vec::new();
    let mut path = String::new();
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = p.to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            if path == main {
                continue;
            }
            let ahead = git(&["rev-list", "--count", &format!("HEAD..{b}")])
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            if ahead > 0 {
                out.push((path.clone(), b.to_string(), ahead));
            }
        }
    }
    out
}

async fn session_end(_project_dir: &PathBuf) -> Result<()> {
    // The session file is the only state; nothing to flush
    // from the daemon's roster. There is no roster.
    //
    // The checkpoint is written here now. Its only trigger used to be a
    // "restart" notification in the daemon's inbox — so in practice it fired
    // almost never, and `recover_restart_checkpoint` had nothing to find.
    // Session end is when a session actually has something worth carrying.
    if let Some(state) = SessionState::load() {
        if !state.agent_id.is_empty() {
            let _ = save_restart_checkpoint(&state);
        }
    }
    Ok(())
}

async fn pre_edit(project_dir: &Path) -> Result<()> {
    let tool_input = tool_input_json();

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(file_path) = input["file_path"].as_str() {
            // Existing hexa boundary check
            validate_boundary_edit(project_dir, file_path)?;

            // ADR-2609131408: another live session has edited this file.
            // Say so before the edit; never block — coordination is the
            // reader's decision, visibility is the tool's job.
            for o in crate::commands::loop_cmd::touched_by_others(project_dir, file_path) {
                println!(
                    "[HEX] {} was edited by session {} (live) under ADR {} · stage {} · last {}. Coordinate before overlapping.",
                    file_path,
                    o.session.get(..8).unwrap_or(&o.session),
                    o.adr,
                    o.stage,
                    o.updated.get(11..16).unwrap_or("?")
                );
            }

            let mode = enforcement_mode(project_dir);
            let state = Some(SessionState::load_or_new());

            // The loop. A prompt the router sized as T2 or T3 is work with a
            // shape, and work with a shape is done under a gate that was
            // written first. No gate recorded means the loop was skipped.
            let sized = state
                .as_ref()
                .and_then(|s| s.last_tier.as_deref())
                .map(|t| t == "T2" || t == "T3")
                .unwrap_or(false);
            if sized {
                let has_gate = crate::commands::loop_cmd::read_loop(project_dir)
                    .and_then(|s| s.get("gate").and_then(|g| g.as_str()).map(|g| !g.is_empty()))
                    .unwrap_or(false);
                if !has_gate {
                    let msg = "No gate recorded for this work. Write the command that must exit 0, then `hexa loop gate '<command>'`, then edit.";
                    if mode == "mandatory" {
                        println!("\u{26d4} {msg}");
                        std::process::exit(2);
                    }
                    println!("\u{26a0}\u{fe0f} {msg}");
                }
            }

            if let Some(ref state) = state {
                // ADR-050: Validate file falls within workplan adapter boundary
                if let Some(ref workplan_id) = state.workplan_id {
                    validate_workplan_boundary(project_dir, file_path, workplan_id)?;
                }

                // ADR-2026-03-23-1700: Enforce adapter boundary via allowed_paths.
                // Only active when allowed_paths is non-empty (set by step-2 worktree setup).
                // Mode: HEXA_BOUNDARY_MODE=mandatory|advisory (default: advisory for safe rollout).
                if !state.allowed_paths.is_empty() && !state.is_path_allowed(file_path) {
                    let boundary_mode = std::env::var("HEXA_BOUNDARY_MODE")
                        .unwrap_or_else(|_| "advisory".to_string());
                    let msg = format!(
                        "BOUNDARY VIOLATION: {} is outside allowed adapter boundary. Allowed: {:?}",
                        file_path, state.allowed_paths
                    );
                    if boundary_mode == "mandatory" {
                        println!("{}", msg);
                        std::process::exit(2);
                    } else {
                        println!("WARNING: {}", msg);
                    }
                }
            }
        }
    }

    Ok(())
}

async fn post_edit(project_dir: &Path) -> Result<()> {
    let tool_input = tool_input_json();
    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(file_path) = input["file_path"].as_str() {
            // ADR-2609131408: the boundary another session needs to see.
            let _ = crate::commands::loop_cmd::touch(project_dir, file_path);
            // Nothing to notify; the edit count is the record
            // to the daemon. The counter is local and stays.
            if let Some(mut state) = SessionState::load() {
                state.edits += 1;
                let _ = state.save();
            }
        }
    }
    Ok(())
}

/// PreAgent — a code-writing subagent runs in its own worktree.
///
/// The harness gives an Agent call `isolation: "worktree"`, which checks the
/// subagent out in a sibling worktree on its own branch. Without it, several
/// subagents edit and commit on the operator's branch at once. Read-only
/// agent types are exempt. In mandatory mode the call is blocked; in advisory
/// mode it is warned about.
///
/// Exit codes: 0 = allow, 2 = block.
async fn pre_agent() -> Result<()> {
    let tool_input = tool_input_json();
    let input: serde_json::Value = match serde_json::from_str(&tool_input) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Can't parse — allow (fail-open)
    };
    let subagent_type = input["subagent_type"].as_str().unwrap_or("");
    if is_read_only_agent(subagent_type) {
        return Ok(());
    }
    let isolated = input["isolation"].as_str() == Some("worktree");
    if isolated {
        return Ok(());
    }
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    let who = if subagent_type.is_empty() { "subagent".to_string() } else { format!("{subagent_type} subagent") };
    if enforcement_mode(&project_dir) == "mandatory" {
        println!(
            "\u{26d4} {who} blocked: it would edit on this branch. Give it its own worktree: isolation: \"worktree\" on the Agent call."
        );
        std::process::exit(2);
    }
    println!(
        "\u{26a0}\u{fe0f} {who} has no worktree of its own; its edits and commits land on this branch. Add isolation: \"worktree\" to the Agent call."
    );
    Ok(())
}

async fn pre_bash() -> Result<()> {
    let tool_input = tool_input_json();

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(command) = input["command"].as_str() {
            // ADR-050: Detect destructive operations
            let destructive = is_destructive_command(command);
            if destructive {
                if let Some(state) = SessionState::load() {
                    let in_ship_phase = state.phase.as_deref() == Some("SHIP");
                    if !in_ship_phase && state.workplan_id.is_some() {
                        // ADR-2026-03-22-1939 P3: use println not eprintln so Claude sees the warning
                        println!(
                            "{} Destructive command outside SHIP phase: `{}`",
                            "\u{26a0}".yellow(),
                            truncate_cmd(command, 60)
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// ADR-2026-03-30-1200: Refresh the architecture fingerprint when key project files have changed.
///
/// Key files: docs/adrs/*.md, docs/workplans/*.json, go.mod, Cargo.toml, package.json.
/// The last generation timestamp is cached in session state
/// on every prompt. When stale, regenerates silently (best-effort) and prints the updated
/// fingerprint block to stdout so Claude Code picks it up as fresh context.
async fn refresh_fingerprint_if_stale(project_dir: &Path) -> Result<()> {
    // Only run when a project id is known
    let project_json = project_dir.join(".hexa/project.json");
    if !project_json.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&project_json)?;
    let project: serde_json::Value = serde_json::from_str(&content)?;
    let project_id = match project["id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return Ok(()),
    };

    // Load the last known fingerprint generation time from session state
    let last_generated: Option<std::time::SystemTime> = SessionState::load()
        .and_then(|s| s.fingerprint_generated_at)
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
        .map(|dt| std::time::UNIX_EPOCH + std::time::Duration::from_secs(dt.timestamp() as u64));

    // Check modification times of key project files
    let key_globs: &[&str] = &["docs/adrs", "docs/workplans", "go.mod", "Cargo.toml", "package.json"];
    let mut latest_mtime: Option<std::time::SystemTime> = None;

    for rel in key_globs {
        let full = project_dir.join(rel);
        if full.is_file() {
            if let Ok(meta) = std::fs::metadata(&full) {
                if let Ok(mtime) = meta.modified() {
                    if latest_mtime.is_none_or(|prev| mtime > prev) {
                        latest_mtime = Some(mtime);
                    }
                }
            }
        } else if full.is_dir() {
            // Check all files one level deep in the directory
            if let Ok(entries) = std::fs::read_dir(&full) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(mtime) = meta.modified() {
                            if latest_mtime.is_none_or(|prev| mtime > prev) {
                                latest_mtime = Some(mtime);
                            }
                        }
                    }
                }
            }
        }
    }

    // Determine if regeneration is needed:
    // - No fingerprint generated yet in this session, OR
    // - A key file is newer than the last generation
    let needs_refresh = match (last_generated, latest_mtime) {
        (None, _) => true,
        (Some(gen), Some(mtime)) => mtime > gen,
        (Some(_), None) => false,
    };

    if !needs_refresh {
        return Ok(());
    }

    // Best-effort regeneration — never block or fail the hook.
    let name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    println!("\n{}", fingerprint_block(&project_id, project_dir, &name).await);
    if let Some(mut state) = SessionState::load() {
        state.fingerprint_generated_at = Some(chrono::Utc::now().to_rfc3339());
        let _ = state.save();
    }

    Ok(())
}

/// The architecture fingerprint for this project, as an injection block.
///
/// Falls back to the minimal in-process block if extraction fails, because
/// blank session context is worse than a thin one.
async fn fingerprint_block(project_id: &str, project_dir: &Path, name: &str) -> String {
    let workplan = SessionState::load()
        .and_then(|s| s.workplan_id)
        .map(|id| project_dir.join(format!("docs/workplans/{}.json", id)))
        .filter(|p| p.exists());
    let fp = hexa_analysis::fingerprint_extractor::FingerprintExtractor::extract(
        project_id,
        project_dir,
        workplan.as_deref(),
    )
    .await;
    let block = fp.to_injection_block();
    if block.trim().is_empty() {
        return minimal_fingerprint_block(project_dir, name);
    }
    block
}

async fn route(project_dir: &Path) -> Result<()> {
    // A prompt the harness itself sent to a model — `hexa harden`'s hunt,
    // verify and fix calls carry HEXA_INTERNAL=1 — is not work intent. It
    // was being sized as a feature and drafted as a workplan, so a planner
    // would have picked up "You are an adversarial code reviewer…" as a task.
    if std::env::var_os("HEXA_INTERNAL").is_some() {
        return Ok(());
    }
    let tool_input = tool_input_json();
    // A host's own notification — a finished background task, a monitor
    // event — arrives on the prompt channel and is not a person asking for
    // work. One was sized as a feature and drafted as a workplan
    // (ADR-2609131611).
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(text) = v["prompt"].as_str().or_else(|| v["content"].as_str()) {
            if is_host_notification(text) {
                return Ok(());
            }
        }
    }

    // ADR-2026-03-30-1200: Refresh architecture fingerprint if key project files have changed
    let _ = refresh_fingerprint_if_stale(project_dir).await;

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        // Claude Code sends the prompt as `prompt`; older hook shims sent `content`.
        if let Some(content) = input["prompt"].as_str().or_else(|| input["content"].as_str()) {
            let lower = content.to_lowercase();

            // Detect hexa-relevant intents and provide context hints
            let hints = classify_prompt(&lower);
            if !hints.is_empty() {
                println!("[HEX] {}", hints.join(", "));
            }

            // ADR-2026-04-11-0227: Three-tier work-intent classifier.
            //
            // Replaces the old passive-warning path with active tier dispatch:
            //   T1Todo      → silent, let Claude's TodoWrite handle it
            //   T2MiniPlan  → one-line suggestion, no auto-invocation
            //   T3Workplan  → auto-invoke `hexa plan draft --background`
            //                 to create a draft stub + surface it in context
            {
                let mut state = SessionState::load_or_new();
                // Every prompt is sized, whatever else is in flight. pre-edit
                // reads the size of the *last* prompt; a stale T3 would block
                // the typo fix that follows a feature.
                let tier = classify_work_intent(&lower);
                state.last_tier = Some(
                    match tier {
                        Tier::T1Todo => "T1",
                        Tier::T2MiniPlan => "T2",
                        Tier::T3Workplan => "T3",
                    }
                    .to_string(),
                );
                let _ = state.save();
                if state.workplan_id.is_none() && state.pending_workplan_draft.is_none() {
                    let mode = enforcement_mode(project_dir);
                    let auto_plan_enabled = auto_plan_enabled(project_dir);

                    // P2.2: Archive stale task.json when on main with a new T2/T3 task.
                    // Worktree branches have a valid task.json — only archive on main.
                    if matches!(tier, Tier::T2MiniPlan | Tier::T3Workplan) {
                        println!("[HEX] {}", crate::commands::loop_cmd::status_line(project_dir));
                        for l in crate::commands::loop_cmd::awareness_lines(project_dir) {
                            println!("[HEX] {l}");
                        }
                        let task_json = project_dir.join(".hexa/task.json");
                        if task_json.exists() {
                            let on_main = std::process::Command::new("git")
                                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                                .output()
                                .ok()
                                .and_then(|o| {
                                    if o.status.success() {
                                        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                                    } else {
                                        None
                                    }
                                })
                                .map(|branch| branch == "main")
                                .unwrap_or(false);

                            if on_main {
                                let history_dir = project_dir.join(".hexa/task-history");
                                let _ = std::fs::create_dir_all(&history_dir);
                                let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
                                let dest = history_dir.join(format!("{}.json", ts));
                                if let Err(e) = std::fs::rename(&task_json, &dest) {
                                    eprintln!("hexa: failed to archive task.json: {}", e);
                                } else {
                                    eprintln!("hexa: archived stale task.json → .hexa/task-history/{}.json", ts);
                                }
                            }
                        }
                    }

                    match tier {
                        Tier::T1Todo => {
                            // No action — let the host agent handle it.
                            // Confirmatory replies fall through silently.
                        }
                        Tier::T2MiniPlan => {
                            // One-line suggestion, no auto-invocation — and
                            // only while no gate is recorded for work in
                            // flight. The status line above already says
                            // where a recorded loop stands; repeating the
                            // instruction on every prompt after it is noise.
                            if !gate_in_flight(project_dir) {
                                println!(
                                    "[HEX] a change with a shape. Write the gate first (the command that must exit 0), record it with `hexa loop gate '<cmd>'`, build to it, then `hexa analyze .`"
                                );
                            }
                        }
                        Tier::T3Workplan => {
                            // Feature-sized intent. In advisory+enabled mode, auto-invoke
                            // `hexa plan draft` in the background. In mandatory mode,
                            // also auto-invoke but still require user approval before
                            // running coders (existing enforcement chain does this).
                            if auto_plan_enabled {
                                match spawn_plan_draft(content) {
                                    Ok(draft_path) => {
                                        println!(
                                            "[HEX] feature-sized task. Decide (ADR in docs/adrs/, `hexa loop adr <ID>`) \u{2192} Gate (`hexa loop gate '<cmd>'`, before the code) \u{2192} Build \u{2192} Harden \u{2192} `hexa analyze .`; drafting a workplan in the background"
                                        );
                                        println!(
                                            "[HEX] draft: {} (run `hexa plan drafts list` to see it, `hexa plan drafts approve <name>` to keep it)",
                                            draft_path
                                        );
                                        state.pending_workplan_draft = Some(draft_path);
                                        let _ = state.save();
                                    }
                                    Err(e) => {
                                        // Fall back to the old warning/block behavior if spawn fails
                                        if mode == "mandatory" {
                                            println!(
                                                "BLOCKED: Cannot proceed without an active workplan. Run: hexa plan create <name> (draft spawn failed: {})",
                                                e
                                            );
                                            std::process::exit(2);
                                        } else {
                                            println!(
                                                "WARNING: No active workplan. Consider: hexa plan create <name> (draft spawn failed: {})",
                                                e
                                            );
                                        }
                                    }
                                }
                            } else if mode == "mandatory" {
                                // Auto-plan disabled but enforcement is mandatory — keep old behavior
                                println!(
                                    "BLOCKED: Cannot proceed without an active workplan. Run: hexa plan create <name>"
                                );
                                std::process::exit(2);
                            } else {
                                // Auto-plan disabled, advisory mode — keep old warning
                                println!(
                                    "WARNING: No active workplan for this work. Consider: hexa plan create <name>"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// ADR-2026-04-11-0227: Spawn `hexa plan draft` in the background with the user prompt.
///
/// Returns the path of the created draft file on success. The draft is
/// a minimal stub quarantined to `docs/workplans/drafts/` — no worktrees,
/// no specs, no coder dispatch. The user (or Claude Code) picks it up
/// via `/hexa-feature-dev` or `hexa plan drafts approve`.
/// Text the host generated to tell the session something happened, rather
/// than a person asking for work.
fn is_host_notification(text: &str) -> bool {
    const MARKERS: [&str; 4] = ["[SYSTEM NOTIFICATION", "<task-notification>", "<system-reminder>", "<local-command-caveat>"];
    let head: String = text.chars().take(400).collect();
    MARKERS.iter().any(|m| head.contains(m))
}

/// A gate is recorded and the loop has not been marked done.
fn gate_in_flight(project_dir: &Path) -> bool {
    crate::commands::loop_cmd::read_loop(project_dir)
        .map(|st| {
            let has_gate = st.get("gate").and_then(|g| g.as_str()).is_some_and(|g| !g.is_empty());
            let done = st.get("stage").and_then(|s| s.as_str()) == Some("done");
            has_gate && !done
        })
        .unwrap_or(false)
}

fn spawn_plan_draft(prompt: &str) -> Result<String> {
    // We run `hexa plan draft --background <prompt>` synchronously here
    // (not via detached spawn) because we need the resulting draft path
    // to surface in the hook output. The draft command itself is fast:
    // it just writes a JSON stub. There's no LLM call on this path.
    let hexa_bin = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("hexa"));

    let output = std::process::Command::new(&hexa_bin)
        .args(["plan", "draft", "--background", prompt])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "hexa plan draft failed: {}",
            stderr.trim()
        ));
    }

    // The draft filename is deterministic (ts + slug) but we re-derive
    // it here by reading the newest file in docs/workplans/drafts/.
    let drafts_dir = Path::new("docs/workplans/drafts");
    let newest = std::fs::read_dir(drafts_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("json")
        })
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .unwrap_or_else(|| "docs/workplans/drafts/".to_string());

    Ok(newest)
}

// ── Boundary validation ──────────────────────────────────────────────

fn validate_boundary_edit(project_dir: &Path, file_path: &str) -> Result<()> {
    let rel = file_path
        .strip_prefix(project_dir.to_string_lossy().as_ref())
        .unwrap_or(file_path)
        .trim_start_matches('/');

    // Detect cross-adapter imports would need AST parsing (hexa analyze does this).
    // Here we do a quick structural check: warn if editing composition-root
    // from a context that suggests adapter work.
    if rel.contains("adapters/primary/") || rel.contains("adapters/secondary/") {
        // Adapters are fine to edit — just can't import each other
    } else if rel.contains("domain/") {
        // Domain should have zero external deps — flag if importing node_modules
    }

    Ok(())
}

/// ADR-050: Check if file being edited falls within the workplan's declared adapter boundary.
/// Loads the workplan JSON, extracts declared `files` from all tasks, and warns/blocks
/// if the edit target isn't in any task's file list.
fn validate_workplan_boundary(project_dir: &Path, file_path: &str, workplan_id: &str) -> Result<()> {
    let rel = file_path
        .strip_prefix(project_dir.to_string_lossy().as_ref())
        .unwrap_or(file_path)
        .trim_start_matches('/');

    // Files outside hexa structure — no enforcement needed
    if detect_hex_layer(rel).is_none() {
        return Ok(());
    }

    // Try to load the workplan JSON to check declared file boundaries
    let workplan_path = project_dir.join("docs/workplans").join(workplan_id);
    let content = match std::fs::read_to_string(&workplan_path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // Can't load workplan — skip boundary check
    };

    let workplan: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    // Collect all declared files from all tiers/steps
    let mut declared_files: Vec<String> = Vec::new();
    if let Some(tiers) = workplan["tiers"].as_object() {
        for (_tier_name, tier) in tiers {
            if let Some(steps) = tier["steps"].as_array() {
                for step in steps {
                    // Single file field
                    if let Some(f) = step["file"].as_str() {
                        declared_files.push(f.to_string());
                    }
                    // Array of files
                    if let Some(files) = step["files"].as_array() {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                declared_files.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // If no files declared in workplan, skip boundary check
    if declared_files.is_empty() {
        return Ok(());
    }

    // Check if the file being edited matches any declared file (prefix match for directories)
    let in_boundary = declared_files.iter().any(|declared| {
        rel == declared || rel.starts_with(declared) || declared.starts_with(rel)
    });

    if !in_boundary {
        let mode = enforcement_mode(project_dir);
        if mode == "mandatory" {
            println!(
                "BLOCKED: File '{}' is outside workplan boundary. Declared files: {:?}",
                rel,
                &declared_files[..declared_files.len().min(5)]
            );
            std::process::exit(2);
        } else {
            println!(
                "WARNING: File '{}' is outside the active workplan's declared boundary.",
                rel
            );
        }
    }

    Ok(())
}

/// Detect which hexa layer a file belongs to.
fn detect_hex_layer(rel_path: &str) -> Option<&'static str> {
    if rel_path.contains("core/domain/") || rel_path.contains("src/domain/") {
        Some("domain")
    } else if rel_path.contains("core/ports/") || rel_path.contains("src/ports/") {
        Some("ports")
    } else if rel_path.contains("core/usecases/") || rel_path.contains("src/usecases/") {
        Some("usecases")
    } else if rel_path.contains("adapters/primary/") {
        Some("primary")
    } else if rel_path.contains("adapters/secondary/") {
        Some("secondary")
    } else if rel_path.contains("composition-root") {
        Some("composition-root")
    } else {
        None
    }
}

// ── Agent Notification Inbox (ADR-060) ───────────────────────────────

/// Save a restart checkpoint so the next session can pick up where this one
/// stopped (ADR-060 step 8).
///
/// A local memory entry
/// now — which also means a checkpoint survives when nothing is running.
fn save_restart_checkpoint(state: &SessionState) -> Result<()> {
    let checkpoint = serde_json::json!({
        "agent_id": state.agent_id,
        "agent_name": state.name,
        "project": state.project,
        "workplan_id": state.workplan_id,
        "phase": state.phase,
        "edits": state.edits,
        "session_id": session_key(),
        "saved_at": chrono::Utc::now().to_rfc3339(),
    });
    let _ = hexa_exec::local_store::memory_put(
        &format!("restart:checkpoint:{}", state.agent_id),
        &checkpoint.to_string(),
    );
    Ok(())
}

// ── ADR-050: Lifecycle helpers ───────────────────────────────────────

/// Recover context from a checkpoint a previous session saved (ADR-060 step 8).
async fn recover_restart_checkpoint() -> Result<()> {
    let Some(mut state) = SessionState::load().filter(|s| !s.agent_id.is_empty()) else {
        return Ok(());
    };
    let key = format!("restart:checkpoint:{}", state.agent_id);
    let Some(raw) = hexa_exec::local_store::memory_get(&key) else {
        return Ok(());
    };
    let Ok(cp) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };

    let field = |k: &str| cp.get(k).and_then(|v| v.as_str()).map(String::from);
    state.workplan_id = field("workplan_id").or(state.workplan_id);
    state.phase = field("phase").or(state.phase);
    let _ = state.save();

    println!("  {} recovered from checkpoint", "\u{21ba}".cyan());
    if let Some(ref wp) = state.workplan_id {
        println!("  Plan:    {}", wp.green());
    }
    // One-shot: a checkpoint consumed is a checkpoint spent, or every future
    // session recovers the same stale context.
    let _ = hexa_exec::local_store::memory_delete(&key);
    Ok(())
}

/// Load the active workplan from local memory into session state (ADR-050).
async fn load_workplan_context(project_id: &str) -> Result<()> {
    let Some(raw) = hexa_exec::local_store::memory_get(&format!("workplan:active:{project_id}"))
    else {
        return Ok(());
    };
    let Ok(wp) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let Some(wp_id) = wp.get("workplan_id").and_then(|v| v.as_str()) else {
        return Ok(());
    };
    if let Some(mut state) = SessionState::load() {
        state.phase = wp.get("phase").and_then(|v| v.as_str()).map(String::from);
        state.workplan_id = Some(wp_id.to_string());
        let _ = state.save();
    }
    println!("  Plan:    {} (active)", wp_id.green());
    Ok(())
}

/// Detect destructive bash commands (ADR-050).
fn is_destructive_command(cmd: &str) -> bool {
    let patterns = [
        "git push --force",
        "git push -f",
        "git reset --hard",
        "git clean -f",
        "rm -rf",
        "rm -r ",
        "drop table",
        "DROP TABLE",
        "git branch -D",
        "git checkout -- .",
        "git restore .",
    ];
    patterns.iter().any(|p| cmd.contains(p))
}

/// Truncate a command string for display.
fn truncate_cmd(cmd: &str, max: usize) -> String {
    if cmd.len() <= max {
        cmd.to_string()
    } else {
        format!("{}...", &cmd[..max])
    }
}

/// Detect short confirmatory responses that likely approve a proposed code change.
/// When Claude proposes work and the user says "yes" / "do it" / "go ahead",
/// that inherits the work classification of the prior exchange.
fn is_confirmatory_response(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    // Only match very short responses — longer prompts are queries, not confirmations
    if trimmed.len() > 30 {
        return false;
    }
    let confirmations = [
        "yes", "yep", "yeah", "yea", "y", "sure", "ok", "okay", "go",
        "go ahead", "do it", "proceed", "continue", "ship it", "lgtm",
        "approved", "let's go", "sounds good", "go for it", "make it so",
        "do that", "yes please", "please do", "please", "correct",
    ];
    confirmations.contains(&trimmed)
}

// ── Prompt classification ────────────────────────────────────────────

fn classify_prompt(prompt: &str) -> Vec<&'static str> {
    let mut hints = Vec::new();

    if prompt.contains("scaffold") || prompt.contains("new project") || prompt.contains("init") {
        hints.push("Relevant: hexa scaffold, hexa init");
    }
    if prompt.contains("architect") || prompt.contains("boundary") || prompt.contains("violation") {
        hints.push("Relevant: hexa analyze");
    }
    if prompt.contains("adr") || prompt.contains("decision record") {
        hints.push("Relevant: hexa adr list/search/status");
    }
    if prompt.contains("agent") || prompt.contains("parallel") || prompt.contains("coordinate") {
        hints.push("Relevant: harness subagents with isolation: \"worktree\"; hexa dev worktree list|merge");
    }
    if prompt.contains("feature") && (prompt.contains("develop") || prompt.contains("implement") || prompt.contains("build")) {
        hints.push("Relevant: /hexa-feature-dev");
    }

    hints
}

// ── ADR-2026-04-11-0227: Three-tier work-intent classifier ─────────────────
//
// The existing `classify_prompt()` emits context hints; this classifier
// decides whether a prompt is a:
//   T1Todo       — small/conversational, let the host agent handle it
//   T2MiniPlan   — work-sized but scoped within one adapter boundary
//   T3Workplan   — feature-sized, cross-adapter, warrants a full workplan
//
// The route() hook dispatches on Tier to decide whether to auto-invoke
// the planner (T3), suggest a mini-plan (T2), or stay silent (T1).

/// Task sizing tiers for intent classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Small change or conversational — no hexa planning artifact needed.
    T1Todo,
    /// Work-sized but contained within one adapter boundary.
    T2MiniPlan,
    /// Feature-sized, cross-adapter — warrants a full workplan.
    T3Workplan,
}

impl Tier {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T1Todo => "T1",
            Tier::T2MiniPlan => "T2",
            Tier::T3Workplan => "T3",
        }
    }
}

/// Classify a user prompt into a work-sizing Tier.
///
/// Uses a weighted keyword scoring heuristic:
///   - Feature-sized verbs + noun subsystems score high (T3)
///   - Generic work verbs without scope signals score medium (T2)
///   - Trivial edits, questions, and conversational replies score T1
///
/// This is a deliberately conservative classifier: when in doubt, it
/// returns a lower tier. False negatives (missed T3 → T2) are cheap
/// (user manually invokes planner); false positives (T1 → T3) would
/// spawn unwanted background agents, so the threshold errs high.
///
/// Implemented as a precedence-ordered Rule table (ADR-2026-04-14-2243):
///   P0 — Escape hatches (always T1, evaluated first)
///   P1 — Constraint signals: questions, trivial edits, confirmatory replies
///   P2 — Scored classification: feature verbs, subsystem nouns, cross-cutting
#[allow(dead_code)]
pub struct ClassifierRule {
    pub label: &'static str,
    pub tier: Tier,
    pub precedence: u8,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}

fn match_escape_hatch(s: &str) -> bool {
    s.is_empty() || s.contains("hexa skip plan") || s.contains("hexa: skip plan")
}

fn match_question(s: &str) -> bool {
    s.ends_with('?')
        || s.starts_with("what ")
        || s.starts_with("why ")
        || s.starts_with("how ")
        || s.starts_with("when ")
        || s.starts_with("where ")
        || s.starts_with("who ")
        || s.starts_with("can you explain")
        || s.starts_with("explain ")
        || s.starts_with("show me ")
}

fn match_trivial_edit(s: &str) -> bool {
    const TRIVIAL: &[&str] = &[
        "fix typo", "fix a typo", "typo in",
        "rename ", "renaming ",
        "add a comment", "add comment",
        "update comment", "update the comment",
        "docstring", "doc comment",
        "one-line", "one line fix",
        "minor tweak", "small tweak", "tiny change",
        "format ", "reformat ", "run fmt", "run rustfmt",
    ];
    TRIVIAL.iter().any(|p| s.contains(p))
}

fn match_confirmatory(s: &str) -> bool {
    is_confirmatory_response(s)
}

fn score_prompt(s: &str) -> i32 {
    let mut score: i32 = 0;

    const FEATURE_VERBS: &[&str] = &[
        "implement", "build a", "build an", "build the", "build support",
        "add support for", "add a new ", "add an ", "design a", "design an",
        "ship ", "deliver ", "roll out",
    ];
    for v in FEATURE_VERBS {
        if s.contains(v) { score += 2; }
    }

    const WORK_VERBS: &[&str] = &[
        "create", "add ", "fix", "refactor", "update", "change",
        "modify", "write", "generate", "scaffold", "wire ", "connect ",
        "remove", "delete", "migrate", "upgrade", "port ",
    ];
    for v in WORK_VERBS {
        if s.contains(v) { score += 1; }
    }

    const SUBSYSTEM_NOUNS: &[&str] = &[
        "feature", "pipeline", "system", "module", "subsystem",
        "adapter", "port", "endpoint", "api", "service", "dashboard",
        "auth", "oauth", "jwt", "database", "schema", "reducer",
        "migration", "cli command", "mcp tool",
    ];
    for n in SUBSYSTEM_NOUNS {
        if s.contains(n) { score += 2; }
    }

    const CROSS_CUTTING: &[&str] = &[
        "end to end", "end-to-end", "integrate ", "integration ",
        "across ", "cross-cutting", "refactor across",
    ];
    for c in CROSS_CUTTING {
        if s.contains(c) { score += 2; }
    }

    if s.len() < 30 { score -= 1; }
    score
}

fn match_t3_score(s: &str) -> bool { score_prompt(s) >= 4 }
fn match_t2_score(s: &str) -> bool { score_prompt(s) >= 1 }

static WORK_INTENT_RULES: &[ClassifierRule] = &[
    // P0 — Escape hatches
    ClassifierRule {
        label: "escape_hatch",
        tier: Tier::T1Todo,
        precedence: 0,
        signals: &["empty input", "hexa skip plan", "hexa: skip plan"],
        matches: match_escape_hatch,
    },
    // P1 — Constraint signals (override scored classification)
    ClassifierRule {
        label: "question",
        tier: Tier::T1Todo,
        precedence: 1,
        signals: &["trailing ?", "what/why/how/when/where/who", "explain", "show me"],
        matches: match_question,
    },
    ClassifierRule {
        label: "trivial_edit",
        tier: Tier::T1Todo,
        precedence: 1,
        signals: &["fix typo", "rename", "add comment", "docstring", "minor tweak", "run fmt"],
        matches: match_trivial_edit,
    },
    ClassifierRule {
        label: "confirmatory",
        tier: Tier::T1Todo,
        precedence: 1,
        signals: &["yes", "ok", "go ahead", "ship it", "lgtm"],
        matches: match_confirmatory,
    },
    // P2 — Scored classification (feature-sized → T3, work-sized → T2)
    ClassifierRule {
        label: "feature_score",
        tier: Tier::T3Workplan,
        precedence: 2,
        signals: &["implement/build/design +2", "subsystem nouns +2", "cross-cutting +2", "score >= 4"],
        matches: match_t3_score,
    },
    ClassifierRule {
        label: "work_score",
        tier: Tier::T2MiniPlan,
        precedence: 2,
        signals: &["create/add/fix/refactor +1", "work verbs +1", "score >= 1"],
        matches: match_t2_score,
    },
];

fn classify_work_intent(prompt: &str) -> Tier {
    let lower = prompt.to_lowercase();
    let trimmed = lower.trim();

    WORK_INTENT_RULES
        .iter()
        .find(|r| (r.matches)(trimmed))
        .map(|r| r.tier)
        .unwrap_or(Tier::T1Todo)
}

// ── Observe (ADR-2026-04-01-2137) ─────────────────────────────────────────────────

/// Non-blocking tool-call observer: reads Claude Code hook JSON from stdin and
/// Records the insight blocks the transcript carries, locally.
///
/// Invoked as a non-blocking hook:
/// ```json
/// { "PreToolUse":  [{ "type": "command", "command": "hexa hook observe-pre",  "blocking": false }] }
/// { "PostToolUse": [{ "type": "command", "command": "hexa hook observe-post", "blocking": false }] }
/// ```
async fn observe(event_type: &str) -> Result<()> {
    // Read Claude Code hook JSON from stdin (non-blocking on missing data).
    let stdin = hook_payload().to_string();
    if stdin.trim().is_empty() {
        return Ok(());
    }

    let hook: serde_json::Value = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Malformed stdin — skip silently
    };

    let session_id = std::env::var("CLAUDE_SESSION_ID")
        .or_else(|_| {
            hook.get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or(std::env::VarError::NotPresent)
        })
        .unwrap_or_default();

    if session_id.is_empty() {
        return Ok(()); // Cannot correlate without session_id
    }

    let tool_name = hook.get("tool_name").and_then(|v| v.as_str()).map(|s| s.to_string());

    // input_json: the tool_input field (PreToolUse) or same (PostToolUse)
    let input_json = hook.get("tool_input").map(|v| v.to_string());

    // result_json: tool_response present in PostToolUse
    let result_json = hook.get("tool_response").map(|v| v.to_string());

    // Resolve agent_id from session state file (best-effort)
    let agent_id = SessionState::load().map(|s| s.agent_id).filter(|s| !s.is_empty());

    // The event POST fed the daemon's live dashboard. What this hook still
    // does — extracting ★ Insight blocks — is local and follows.
    let _ = (&session_id, &agent_id, &tool_name, &input_json);

    // ── Post-commit brain validate (fast subset) ────────────────────
    // After a Bash tool call that looks like a git commit, run the two
    // fast self-consistency checks: binary freshness and MCP↔CLI parity.
    // These complete in <2s and catch drift immediately after a commit.
    // ── Insight extraction (ADR-2026-04-14-2345) ─────────────────────────
    // Scan text-carrying fields for `★ Insight` blocks and persist each
    // to ~/.hexa/insights/<id>.yaml. Non-blocking: failures log to stderr
    // but never halt the hook or block the turn.
    if event_type == "PostToolUse" || event_type == "Stop" {
        let mut candidate_texts: Vec<String> = Vec::new();
        // tool_response is the primary source on PostToolUse; Stop
        // events may carry assistant text under varied field names so
        // also sweep the whole hook payload as a last resort.
        if let Some(s) = result_json.as_deref() {
            candidate_texts.push(s.to_string());
        }
        if event_type == "Stop" {
            if let Some(s) = hook.get("stop_hook_active").and_then(|v| v.as_str()) {
                candidate_texts.push(s.to_string());
            }
            // Best-effort: stringify the whole envelope so assistant
            // text carried in any field (e.g. `message`, `content`,
            // `text`) still gets scanned.
            candidate_texts.push(hook.to_string());
        }

        let turn: usize = hook
            .get("turn")
            .and_then(|v| v.as_u64())
            .map(|u| u as usize)
            .unwrap_or(0);

        let insights_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".hexa/insights");

        let mut wrote_dir = false;
        for text in &candidate_texts {
            let extracted = crate::commands::insight::extract_insights(
                text,
                &session_id,
                turn,
            );
            for ins in extracted {
                if !wrote_dir {
                    std::fs::create_dir_all(&insights_dir).ok();
                    wrote_dir = true;
                }
                let path = insights_dir.join(format!("{}.yaml", sanitize_id(&ins.id)));
                match serde_yaml::to_string(&ins) {
                    Ok(yaml) => {
                        if let Err(e) = std::fs::write(&path, yaml) {
                            eprintln!("insight extractor: write {} failed: {}", path.display(), e);
                        } else {
                            eprintln!("insight: extracted {} -> {:?}", ins.id, ins.route_to);
                        }
                    }
                    Err(e) => eprintln!("insight extractor: yaml serialize failed: {}", e),
                }
            }
        }
    }

    Ok(())
}

/// Constrain insight ids to filesystem-safe characters so writing
/// `~/.hexa/insights/<id>.yaml` can never escape the insights directory
/// or include directory separators.
fn sanitize_id(id: &str) -> String {
    let trimmed: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if trimmed.is_empty() {
        "insight".to_string()
    } else {
        trimmed
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─ ADR-2026-04-11-0227: classify_work_intent tier classifier ─

    #[test]
    fn t1_question_prompts() {
        // Questions are T1 regardless of other signals
        assert_eq!(classify_work_intent("how does the planner work?"), Tier::T1Todo);
        assert_eq!(classify_work_intent("what is HexFlo?"), Tier::T1Todo);
        assert_eq!(classify_work_intent("can you explain the ADR process"), Tier::T1Todo);
        assert_eq!(classify_work_intent("why do workplans need specs first?"), Tier::T1Todo);
        assert_eq!(classify_work_intent("show me the hook router"), Tier::T1Todo);
    }

    #[test]
    fn t1_trivial_edits() {
        // Trivial-edit phrases → T1 regardless of verb weight
        assert_eq!(classify_work_intent("fix typo in README"), Tier::T1Todo);
        assert_eq!(classify_work_intent("rename getCwd to getCurrentWorkingDirectory"), Tier::T1Todo);
        assert_eq!(classify_work_intent("add a comment explaining the regex"), Tier::T1Todo);
        assert_eq!(classify_work_intent("update the docstring on fn foo"), Tier::T1Todo);
        assert_eq!(classify_work_intent("run rustfmt"), Tier::T1Todo);
    }

    #[test]
    fn t1_empty_and_confirmatory() {
        assert_eq!(classify_work_intent(""), Tier::T1Todo);
        assert_eq!(classify_work_intent("   "), Tier::T1Todo);
        assert_eq!(classify_work_intent("yes"), Tier::T1Todo);
        assert_eq!(classify_work_intent("go ahead"), Tier::T1Todo);
        assert_eq!(classify_work_intent("lgtm"), Tier::T1Todo);
    }

    #[test]
    fn t1_opt_out_phrase() {
        // "hexa skip plan" escape hatch always downgrades to T1
        assert_eq!(
            classify_work_intent("implement oauth login hexa skip plan"),
            Tier::T1Todo
        );
        assert_eq!(
            classify_work_intent("hexa skip plan: build the whole pipeline"),
            Tier::T1Todo
        );
    }

    #[test]
    fn t2_small_adapter_work() {
        // Work-sized but scoped to one adapter — should be T2, not T3.
        assert_eq!(
            classify_work_intent("add a helper function to trim whitespace"),
            Tier::T2MiniPlan
        );
        assert_eq!(
            classify_work_intent("refactor the retry loop in fs_adapter"),
            Tier::T2MiniPlan
        );
        assert_eq!(
            classify_work_intent("update the timeout in the llm adapter"),
            Tier::T2MiniPlan
        );
    }

    #[test]
    fn t3_feature_sized_cross_adapter() {
        // Feature-sized verbs + subsystem nouns → T3
        assert_eq!(
            classify_work_intent("implement OAuth login with refresh tokens"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent("add support for a new mcp tool that dispatches swarms"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent("build an end-to-end audit pipeline across all adapters"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent("design a new authentication subsystem with JWT"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent(
                "implement a new feature for spacetimedb module deployment via the CLI"
            ),
            Tier::T3Workplan
        );
    }

    #[test]
    fn t1_regression_false_positives() {
        // P10: regression suite — these should NEVER return T3.
        // These are the kinds of prompts that LOOK like work but are actually
        // small changes, read-only questions, or conversational.
        let t1_prompts = [
            "what files does the route hook read?",
            "how does classify_prompt differ from classify_work_intent?",
            "show me the SessionState struct",
            "explain the three-tier sizing",
            "fix typo in the ADR header",
            "rename the Tier variants to Todo/MiniPlan/Workplan",
            "add a comment to the match arm",
            "reformat the tests module with rustfmt",
            "why is the threshold at score >= 4?",
            "can you explain how the scoring works?",
            "update the docstring on classify_work_intent",
            "yes",
            "lgtm",
            "go ahead",
            "hexa skip plan: implement oauth",
            "?",
            "",
            "",
            "run rustfmt",
            "one-line fix to the regex",
        ];
        for prompt in t1_prompts {
            let tier = classify_work_intent(prompt);
            assert_ne!(
                tier,
                Tier::T3Workplan,
                "prompt '{}' should NOT be T3 (got {:?})",
                prompt,
                tier
            );
        }
    }

    #[test]
    fn cross_tier_regression_trivial_verb_with_t3_nouns() {
        // Cross-tier regression: prompts with T3-triggering nouns (feature,
        // subsystem, pipeline) must stay T1 when the verb is trivial.
        // These would score ≥4 if the TRIVIAL early-return didn't fire.

        // "rename" is trivial even when the target is a feature-level concept
        assert_eq!(
            classify_work_intent("rename the variable that implements the OAuth feature"),
            Tier::T1Todo
        );

        // "add a comment" is trivial even inside a cross-adapter subsystem
        assert_eq!(
            classify_work_intent("add a comment explaining the migration pipeline endpoint"),
            Tier::T1Todo
        );

        // "fix typo" is trivial even when the file lives in a subsystem module
        assert_eq!(
            classify_work_intent("fix typo in the authentication subsystem adapter module"),
            Tier::T1Todo
        );
    }

    #[test]
    fn test_rule_table_invariants() {
        // Structural invariant: the TRIVIAL early-return MUST fire before
        // score-based classification. If the ordering flips, prompts that
        // combine a trivial marker with subsystem nouns false-positive as T3,
        // spawning unwanted workplan drafts.
        //
        // Each tuple: (prompt with trivial marker + T3 nouns, which marker wins)
        let trivial_beats_feature = [
            ("fix typo in the feature pipeline", "fix typo"),
            ("rename the adapter port to something clearer", "rename "),
            ("add a comment explaining the auth subsystem", "add a comment"),
            ("update the docstring on the api endpoint handler", "docstring"),
            ("reformat the migration module", "reformat "),
            ("run rustfmt on the mcp tool adapter", "run rustfmt"),
            ("fix a typo in the database schema docs", "fix a typo"),
            ("minor tweak to the jwt service port", "minor tweak"),
        ];
        for (prompt, trivial_marker) in trivial_beats_feature {
            assert_eq!(
                classify_work_intent(prompt),
                Tier::T1Todo,
                "trivial marker '{}' must precede feature-scoring in: '{}'",
                trivial_marker,
                prompt
            );
        }

        // Positive control: without the trivial marker, the same nouns reach T3.
        assert_eq!(
            classify_work_intent("implement the auth subsystem with jwt"),
            Tier::T3Workplan,
            "positive control: subsystem nouns without trivial marker → T3"
        );
        assert_eq!(
            classify_work_intent("build a new feature for the database migration pipeline"),
            Tier::T3Workplan,
            "positive control: feature + pipeline + database without trivial → T3"
        );
    }

    // ─ Existing helpers (sanity checks) ─

    #[test]
    fn classify_prompt_hints() {
        // Smoke test for the existing hint function.
        assert!(!classify_prompt("please scaffold a new hexa project").is_empty());
        assert!(classify_prompt("just a random question").is_empty());
    }

    #[test]
    fn confirmatory_response_matching() {
        assert!(is_confirmatory_response("yes"));
        assert!(is_confirmatory_response("go ahead"));
        assert!(is_confirmatory_response("lgtm"));
        assert!(!is_confirmatory_response("yes but make it async"));
        assert!(!is_confirmatory_response("this is a much longer response that is not a confirmation"));
    }
}

#[cfg(test)]
mod loop_reminder_tests {
    use super::{gate_in_flight, is_host_notification};

    /// A host's notification is not work intent; a person's prompt that
    /// happens to mention one still is.
    #[test]
    fn a_host_notification_is_not_work_intent() {
        assert!(is_host_notification("[SYSTEM NOTIFICATION - NOT USER INPUT]\nbackground task finished"));
        assert!(is_host_notification("<task-notification>\n<task-id>b1az</task-id>"));
        assert!(is_host_notification("<system-reminder>\ncontext follows"));
        assert!(!is_host_notification("add a task notification to the report"));
        assert!(!is_host_notification("why did the system notification get drafted as a workplan?"));
    }

    fn project_with_loop(body: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hexa-loop-reminder-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".hexa")).unwrap();
        if let Some(b) = body {
            std::fs::write(dir.join(".hexa/loop.json"), b).unwrap();
        }
        dir
    }

    /// The gate reminder repeats only while nothing is recorded: a
    /// recorded gate on a loop that is not done means the work is already
    /// under a gate, and a done loop means there is nothing in flight.
    #[test]
    fn a_recorded_gate_on_an_unfinished_loop_silences_the_reminder() {
        assert!(!gate_in_flight(&project_with_loop(None)), "no loop recorded");
        assert!(!gate_in_flight(&project_with_loop(Some(r#"{"adr":"ADR-1","stage":"decide"}"#))), "no gate yet");
        assert!(gate_in_flight(&project_with_loop(Some(r#"{"adr":"ADR-1","gate":"cargo test","stage":"build"}"#))), "gate in flight");
        assert!(!gate_in_flight(&project_with_loop(Some(r#"{"adr":"ADR-1","gate":"cargo test","stage":"done"}"#))), "loop done");
    }
}
