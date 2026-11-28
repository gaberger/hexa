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

pub mod git_commit;
pub mod punch_list;
pub mod source_gate;
pub mod write_shape;

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
    /// Overlaps with other live sessions already reported to this one, as
    /// `session|file`. The route hook says each once; repeated on every
    /// prompt it was noise the reader learned to skip.
    #[serde(default)]
    reported_overlaps: Vec<String>,
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

/// Tools that only read (ADR-2611221646 §5). A definition is read-only when
/// its `tools` names nothing else: `*`, any other tool, or one hexa does not
/// know is assumed to write.
const READ_TOOLS: &[&str] = &["Read", "Grep", "Glob", "LS", "NotebookRead", "WebFetch", "WebSearch"];

/// Whether a subagent type is read-only: a fixed read-only type, or one whose
/// definition lists `tools` and every one of them only reads
/// (ADR-2611221646 §5). The definition is found as Claude Code finds it, by
/// its frontmatter `name:` (see [`find_definition`]).
fn agent_is_read_only(kind: &str, project_dir: &Path) -> bool {
    if is_read_only_agent(kind) {
        return true;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    definition_grants_only_reads(kind, project_dir, home.as_deref())
}

/// What one agents folder holds for a subagent type.
enum Definition {
    /// No file in the folder has that `name:`.
    Absent,
    /// Exactly one file does: its parsed frontmatter.
    One(serde_yaml::Value),
    /// More than one file claims the name: no definition hexa can trust.
    Ambiguous,
}

/// The `*.md` file in `dir` whose frontmatter `name:` is `kind`. Claude Code
/// names an agent by that field, not by the file's name, so a file without a
/// `name:` matches nothing.
fn find_definition(dir: &Path, kind: &str) -> Definition {
    let Ok(entries) = std::fs::read_dir(dir) else { return Definition::Absent };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") || !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Some(front) = frontmatter(&text) else { continue };
        if front.get("name").and_then(|n| n.as_str()).map(str::trim) == Some(kind) {
            found.push(front);
        }
    }
    match found.len() {
        0 => Definition::Absent,
        1 => found.pop().map_or(Definition::Absent, Definition::One),
        _ => Definition::Ambiguous,
    }
}

/// A project definition is final: the home folder is read only when no
/// project file claims the name. An ambiguous name is no definition.
fn definition_grants_only_reads(kind: &str, project_dir: &Path, home: Option<&Path>) -> bool {
    if kind.trim().is_empty() {
        return false;
    }
    let front = match find_definition(&project_dir.join(".claude/agents"), kind) {
        Definition::One(f) => f,
        Definition::Ambiguous => return false,
        Definition::Absent => match home.map(|h| find_definition(&h.join(".claude/agents"), kind)) {
            Some(Definition::One(f)) => f,
            _ => return false,
        },
    };
    match definition_tools(&front) {
        Some(tools) => tools.iter().all(|t| is_read_tool(t)),
        None => false,
    }
}

/// Whether one `tools` entry, such as `Read` or `WebFetch(domain:x.com)`,
/// names a tool that only reads. The name matches exactly (`read` is not
/// `Read`), and a specifier's brackets must balance and close at the end of
/// the entry. `*` anywhere is every tool. Anything else is unknown, and an
/// unknown tool writes.
fn is_read_tool(entry: &str) -> bool {
    if entry.contains('*') {
        return false;
    }
    let name = match entry.find('(') {
        None if entry.contains(')') => return false,
        None => entry,
        Some(open) if closes_at_end(&entry[open..]) => &entry[..open],
        Some(_) => return false,
    };
    READ_TOOLS.contains(&name)
}

/// Whether the bracket that opens `s` closes at its last character, so the
/// brackets balance and nothing follows them.
fn closes_at_end(s: &str) -> bool {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + c.len_utf8() == s.len();
                }
            }
            _ => {}
        }
    }
    false
}

/// Split a `tools` string on commas and whitespace outside parentheses:
/// `Read, Grep`, `Read Grep` and `WebFetch(domain: x.com) Read` each name
/// their tools.
fn split_tools(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            c if depth == 0 && (c == ',' || c.is_whitespace()) => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// A definition's YAML frontmatter, or `None` when it has none hexa can read.
fn frontmatter(text: &str) -> Option<serde_yaml::Value> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let front: Vec<&str> = lines.take_while(|l| l.trim() != "---").collect();
    serde_yaml::from_str(&front.join("\n")).ok()
}

/// The `tools` list in a definition's frontmatter. `None` — treated as no
/// definition — when there is no (non-empty) `tools` entry, or a list item
/// that is not a string.
fn definition_tools(yaml: &serde_yaml::Value) -> Option<Vec<String>> {
    let tools: Vec<String> = match yaml.get("tools")? {
        serde_yaml::Value::String(s) => split_tools(s),
        serde_yaml::Value::Sequence(seq) => {
            let mut all = Vec::new();
            for v in seq {
                all.extend(split_tools(v.as_str()?));
            }
            all
        }
        _ => return None,
    };
    let tools: Vec<String> = tools.into_iter().filter(|t| !t.is_empty()).collect();
    (!tools.is_empty()).then_some(tools)
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
/// work-intent prompts is enabled for this project. Off unless set.
///
/// Precedence (highest to lowest):
/// 1. `HEXA_AUTO_PLAN` env var — `0`/`false` disables, anything else enables
/// 2. `workplan.auto_invoke.enabled` in `.hexa/project.json` — bool
/// 3. Default: off
///
/// It was on by default. The drafts it wrote were stubs holding only the
/// prompt; they piled up untracked in `docs/workplans/drafts/`, most of them
/// sub-agent briefs, and nobody expanded them. A project that wants them
/// asks.
fn auto_plan_enabled(project_dir: &Path) -> bool {
    if let Ok(val) = std::env::var("HEXA_AUTO_PLAN") {
        return val != "0" && val.to_lowercase() != "false";
    }
    let project_json = project_dir.join(".hexa/project.json");
    std::fs::read_to_string(&project_json)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|p| p["workplan"]["auto_invoke"]["enabled"].as_bool())
        .unwrap_or(false)
}

/// A prompt one agent wrote for another: a sub-agent brief or a delegated
/// task, or any prompt submitted inside a subagent. Not a person's intent,
/// so never drafted as a workplan.
fn is_agent_brief(prompt: &str, payload: &serde_json::Value) -> bool {
    let head = prompt.trim_start().to_lowercase();
    let from_subagent = ["agent_id", "agent_type"]
        .iter()
        .any(|k| payload.get(*k).and_then(|v| v.as_str()).is_some_and(|v| !v.is_empty()));
    from_subagent || head.starts_with("act as the") || head.starts_with("delegated task")
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
    /// On Stop — send the session back while its own work is open: files it touched and did not commit, its checklist, its branch, its ADR (ADR-2611161800 §5)
    Stop,
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
        HookEvent::Stop => stop_until_finished(&project_dir),
    }
}

// ── Event handlers ───────────────────────────────────────────────────

async fn session_start(project_dir: &Path) -> Result<()> {
    // Before anything else, and in any directory: a stale binary is what
    // every later line of this session would be read through (ADR-2610270853 §6).
    for line in crate::commands::provenance::session_lines(project_dir).await {
        println!("{} {}", "\u{26a0}".yellow(), line);
    }

    let project_json = project_dir.join(".hexa/project.json");

    if !project_json.exists() {
        eprintln!(
            "{} Not a hexa project (no .hexa/project.json). Run `hexa init`.",
            "\u{26a0}".yellow()
        );
        // What the root's manifest says, so the session is not left to guess
        // (ADR-2611221646 §4). Nothing is said when no manifest names one.
        if let Some(language) = manifest_language(project_dir) {
            println!("Language: {language}");
        }
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
    // ADR-2611160910 finding 2: a new session sees the previous session's
    // checklist, not only its stage, and how to carry it over.
    for l in crate::commands::loop_cmd::previous_lines(project_dir) {
        println!("  {l}");
    }
    for l in awareness_lines(project_dir) {
        println!("  {l}");
    }
    // The code graph, rebuilt in the background when the tree moved (ADR-2611221610 §5).
    println!("  {}", crate::commands::graph::refresh_for_session(project_dir));
    let mut st = SessionState::load_or_new();
    st.project = crate::commands::loop_cmd::project_name(project_dir);
    st.name = session_key();
    // The block is printed once per session, here. The route hook reads
    // this and stays quiet about it.
    st.fingerprint_generated_at = Some(chrono::Utc::now().to_rfc3339());
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

/// The language the root's manifest names: `Cargo.toml`, `go.mod`,
/// `package.json`, then a Python manifest (`pyproject.toml`, `setup.py`,
/// `setup.cfg`) when none of the others is there (ADR-2611221646 §4).
fn manifest_language(project_dir: &Path) -> Option<&'static str> {
    if project_dir.join("Cargo.toml").exists() {
        Some("rust")
    } else if project_dir.join("go.mod").exists() {
        Some("go")
    } else if project_dir.join("package.json").exists() {
        Some(hexa_analysis::analyzer::package_language(project_dir))
    } else if ["pyproject.toml", "setup.py", "setup.cfg"].iter().any(|m| project_dir.join(m).exists()) {
        Some("python")
    } else {
        None
    }
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
    } else if let Some(python) = manifest_language(project_dir) {
        // ADR-2611221646 §4: a Python manifest, and none of the others.
        language = python.to_string();
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
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR").map(PathBuf::from).unwrap_or_else(|_| cwd.clone());
    if !isolated && !agent_type.is_empty() && !agent_is_read_only(&agent_type, &project_dir) {
        eprintln!(
            "[hexa hook] subagent {} ({}) is running in the main checkout, not a worktree; its edits land on this branch",
            agent_id, agent_type
        );
    }
    hexa_exec::ports::RunLog::record(&hexa_exec::run_log(), &serde_json::json!({
        "kind": "subagent",
        "event": "start",
        "agent_id": agent_id,
        "agent_type": agent_type,
        "cwd": cwd.display().to_string(),
        "in_worktree": isolated,
        "ts": chrono::Utc::now().to_rfc3339(),
    }));
    if !agent_id.is_empty() {
        bind_subagent_task(&agent_id, &cwd);
    }
    Ok(())
}

/// ADR-2611051850 §8: tell the ledger a subagent started. A task registered
/// with `--owner <agent_id>`, or a claimed task registered for this
/// subagent's worktree (`--worktree`), becomes `running` and bound to the
/// `agent_id`, so `pre-edit` holds the subagent to its lease. No task, no
/// change: the event carries no task name, so a parent must register first.
fn bind_subagent_task(agent_id: &str, cwd: &Path) {
    use crate::commands::task_ledger;
    use hexa_core::ports::ledger::TaskStatus;
    let Ok(ledger) = task_ledger::read(cwd) else {
        eprintln!("[hexa hook] the task ledger could not be read, so subagent {agent_id} was not bound to a task; see `hexa status`");
        return;
    };
    let here = task_ledger::canonical_lenient(cwd).display().to_string();
    let target = ledger
        .active_for_owner(agent_id)
        .or_else(|| {
            ledger
                .active()
                .find(|t| t.status == TaskStatus::Claimed && t.owner.is_none() && t.worktree.as_deref() == Some(here.as_str()))
        })
        .map(|t| t.id.clone());
    let Some(id) = target else { return };
    let bound = task_ledger::update(cwd, |l| {
        let Some(t) = l.get_mut(&id).filter(|t| t.status.is_active()) else { return Ok(None) };
        t.status = TaskStatus::Running;
        t.owner = Some(agent_id.to_string());
        if t.worktree.is_none() {
            t.worktree = Some(here.clone());
        }
        t.updated = task_ledger::now();
        Ok(Some((t.id.clone(), t.name.clone(), t.leases.join(", "))))
    });
    match bound {
        Ok(Some((id, name, leases))) => eprintln!("[hexa hook] subagent {agent_id} is bound to task {id} ({name}); lease: {leases}"),
        Ok(None) => {}
        Err(e) => eprintln!("[hexa hook] could not bind subagent {agent_id} to task {id}: {e}"),
    }
}

/// ADR-2611051850 §8: a subagent stopped. Its task is closed from facts
/// hexa can read: the task's gate rerun in the task's worktree gives the
/// outcome, and that worktree's HEAD gives the commit. A task with no gate
/// has no outcome hexa can judge, so it stays claimed and the line printed
/// names the `hexa agent report` that closes it: no lease is dropped on a guess.
fn close_subagent_task(agent_id: &str, project_dir: &Path) {
    use crate::commands::{agent, task_ledger};
    use hexa_core::ports::ledger::TaskStatus;
    let Ok(ledger) = task_ledger::read(project_dir) else {
        eprintln!("[hexa hook] the task ledger could not be read; subagent {agent_id}'s task was not closed");
        return;
    };
    let Some(task) = ledger.active_for_owner(agent_id).cloned() else { return };
    let dir = task.worktree.as_deref().map(PathBuf::from).unwrap_or_else(|| project_dir.to_path_buf());
    let commit = std::process::Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|c| !c.is_empty());
    let Some(gate) = task.gate.as_deref().filter(|g| !g.trim().is_empty()) else {
        println!(
            "task {} ({}) has no gate, so hexa cannot judge its outcome and keeps its lease; close it with: hexa agent report --id {} --outcome <passed|failed> --commit {}",
            task.id, task.name, task.id, commit.as_deref().unwrap_or("<sha>")
        );
        return;
    };
    let passed = std::process::Command::new("sh")
        .arg("-c")
        .arg(gate)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let (status, outcome) = if passed { (TaskStatus::Done, "passed") } else { (TaskStatus::Failed, "failed") };
    let note = format!("closed by SubagentStop for {agent_id}: gate `{gate}` {outcome}");
    match agent::close_task(project_dir, &task.id, status, Some(outcome), commit.as_deref(), Some(&note), false) {
        Ok(_) => println!("task {} ({}) closed {outcome} by its gate; leases released", task.id, task.name),
        Err(e) => eprintln!("[hexa hook] could not close task {}: {e}", task.id),
    }
}

/// ADR-2611051850 §4: refuse (exit 2, reason on stderr) an edit outside the
/// lease of the task this session is bound to. Bound by `HEXA_TASK=<id>`, or
/// by the payload's `agent_id` equalling the task's owner. Unbound: nothing.
/// A bound session whose ledger cannot be read, or whose task does not exist,
/// is refused too: a rule that cannot be checked is not a rule that passed.
fn lease_gate(project_dir: &Path, file_path: &str) {
    use crate::commands::task_ledger;
    use hexa_core::ports::ledger::{lease_decision, Decision};
    let by_env = std::env::var(task_ledger::ENV_TASK).ok().filter(|v| !v.is_empty());
    let agent_id = hook_payload()["agent_id"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    if by_env.is_none() && agent_id.is_none() {
        return;
    }
    let refuse = |why: String| -> ! {
        eprintln!("\u{26d4} lease: {why}");
        crate::commands::events::exit(2);
    };
    let ledger = match task_ledger::read(project_dir) {
        Ok(l) => l,
        Err(e) if by_env.is_some() => refuse(format!("the session is bound to a task but the ledger cannot be read ({e})")),
        Err(e) => {
            println!("[hexa hook] lease check skipped for subagent {}: {e}", agent_id.as_deref().unwrap_or("?"));
            return;
        }
    };
    let task = match (&by_env, &agent_id) {
        (Some(id), _) => match ledger.get(id) {
            Some(t) => t,
            None => refuse(format!("HEXA_TASK={id} names no task in the ledger. `hexa agent list` shows the ids; `hexa agent register` makes one.")),
        },
        (None, Some(a)) => match ledger.active_for_owner(a) {
            Some(t) => t,
            None => return,
        },
        (None, None) => return,
    };
    let abs = if Path::new(file_path).is_absolute() { PathBuf::from(file_path) } else { project_dir.join(file_path) };
    let mut roots: Vec<PathBuf> = vec![project_dir.to_path_buf()];
    roots.extend(task.worktree.as_deref().map(PathBuf::from));
    roots.push(task_ledger::main_root(project_dir));
    let Some(rel) = roots.iter().find_map(|r| task_ledger::relative_to(r, &abs)) else {
        return; // outside the repository: leases govern the repository's files
    };
    if let Decision::Deny(why) = lease_decision(task, &rel, &task_ledger::sealed(project_dir)) {
        refuse(why);
    }
}

/// SubagentStop — record the stop, then name every worktree branch that
/// holds commits this branch does not, with the command that lands them.
/// Nothing is merged here. A merge is a decision, and the hook prints the
/// facts it needs.
async fn subagent_stop() -> Result<()> {
    let v = hook_payload().clone();
    hexa_exec::ports::RunLog::record(&hexa_exec::run_log(), &serde_json::json!({
        "kind": "subagent",
        "event": "stop",
        "agent_id": v["agent_id"].as_str().unwrap_or(""),
        "agent_type": v["agent_type"].as_str().unwrap_or(""),
        "ts": chrono::Utc::now().to_rfc3339(),
    }));
    let agent_id = v["agent_id"].as_str().unwrap_or("");
    if !agent_id.is_empty() {
        let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
        close_subagent_task(agent_id, &project_dir);
    }
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

/// The most times in a row a session is sent back to finish before its stop
/// is let through (ADR-2611161800 §5): a session that cannot finish can still
/// end, and say why.
const STOP_BLOCKS: u64 = 3;

/// Stop — a session does not stop with its own work open. While something it
/// can finish itself is open, answer `block` with the list as the reason, so
/// the agent carries on. Three times in a row at most; then let the stop
/// through and print the list. Commits not pushed are said, never forced.
fn stop_until_finished(project_dir: &Path) -> Result<()> {
    let v = hook_payload();
    let session = v["session_id"].as_str().filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(crate::commands::loop_cmd::session_id);
    let open = crate::commands::finish::session_open(project_dir, &session);
    let counts = project_dir.join(".hexa").join("stop-blocks.json");
    let mut m: serde_json::Map<String, serde_json::Value> =
        std::fs::read_to_string(&counts).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let n = m.get(&session).and_then(|x| x.as_u64()).unwrap_or(0);
    let list = |items: &[crate::commands::finish::Item]| items.iter().map(|i| format!("- {}: {}\n  {}", i.kind, i.what, i.fix)).collect::<Vec<_>>().join("\n");
    let save = |m: &serde_json::Map<String, serde_json::Value>| {
        let _ = std::fs::create_dir_all(project_dir.join(".hexa"));
        let _ = std::fs::write(&counts, serde_json::to_string(m).unwrap_or_default());
    };
    if !open.blocking.is_empty() && n < STOP_BLOCKS {
        m.insert(session, serde_json::json!(n + 1));
        save(&m);
        let mut reason = format!(
            "Your work is not finished: a plan ends with its tasks done, its ADR closed and everything on main (ADR-2611161800). Finish these, or say plainly why one cannot be finished now:\n{}",
            list(&open.blocking)
        );
        if !open.reported.is_empty() {
            reason.push_str(&format!("\nAlso, said and not forced (a push or a pull request can need someone's approval):\n{}", list(&open.reported)));
        }
        println!("{}", serde_json::json!({ "decision": "block", "reason": reason }));
        return Ok(());
    }
    m.remove(&session);
    save(&m);
    if !open.blocking.is_empty() {
        println!("[HEX] stopping with work still open, after {STOP_BLOCKS} times sent back to finish it:\n{}", list(&open.blocking));
    }
    if !open.reported.is_empty() {
        println!("[HEX] {}", list(&open.reported));
    }
    Ok(())
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
            // ADR-2611051850 §4: a session bound to a task is held to its
            // lease. First, so no softer rule below can excuse an edit the
            // lease refuses. An unbound session passes through unchanged.
            lease_gate(project_dir, file_path);

            // Everything below is this project's rules. A file outside it —
            // a shell profile, another repository — is not held to them.
            let path = Path::new(file_path);
            if path.is_absolute() && !path.starts_with(project_dir) {
                return Ok(());
            }

            // The file's graph neighbourhood, once per file per session,
            // before any rule below can stop the edit (ADR-2611221610 §6).
            print_edit_context(project_dir, path);

            // The layer rules, on what the edit would leave.
            validate_boundary_edit(project_dir, file_path, &input).await?;

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

            // The loop (ADR-2611160730 §4). Every edit to project source is
            // done under a gate recorded for the current task, whatever size
            // the router gave the prompt. The rule is `gate_is_current`'s.
            let abs = if path.is_absolute() { path.to_path_buf() } else { project_dir.join(path) };
            source_gate_check(project_dir, &[abs], mode, "this edit", false);

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
                        eprintln!("{}", msg);
                        crate::commands::events::exit(2);
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
            // ADR-2609131408: the boundary another session needs to see. A
            // file outside the project, such as a scratch patch script, is no
            // boundary; the files such a script writes are recorded by
            // pre-bash (ADR-2611160730 §4).
            let path = Path::new(file_path);
            if !path.is_absolute() || path.starts_with(project_dir) {
                let _ = crate::commands::loop_cmd::touch(project_dir, file_path);
            }
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
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    if agent_is_read_only(subagent_type, &project_dir) {
        return Ok(());
    }
    let isolated = input["isolation"].as_str() == Some("worktree");
    if isolated {
        return Ok(());
    }
    let who = if subagent_type.is_empty() { "subagent".to_string() } else { format!("{subagent_type} subagent") };
    if enforcement_mode(&project_dir) == "mandatory" {
        eprintln!(
            "\u{26d4} {who} blocked: it would edit on this branch. Give it its own worktree: isolation: \"worktree\" on the Agent call."
        );
        crate::commands::events::exit(2);
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
            // ADR-050: destructive operations, whoever runs them. The warning
            // was printed to stdout, and only while a legacy workplan id was
            // set, so `git push --force` and `reset --hard` passed in silence.
            let cwd = hook_payload()["cwd"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let found = destructive_ops(command, &cwd);
            if !found.is_empty() {
                let project_dir = std::env::var("CLAUDE_PROJECT_DIR").map(PathBuf::from).unwrap_or_else(|_| cwd.clone());
                let shown = truncate_cmd(command, 80);
                if let Some(op) = found.iter().find(|o| o.protected) {
                    if enforcement_mode(&project_dir) == "mandatory" {
                        eprintln!(
                            "\u{26d4} {} refused: it rewrites the shared history of {}. `{shown}`. Push a branch and open a pull request instead.",
                            op.what, op.branch
                        );
                        crate::commands::events::exit(2);
                    }
                }
                let what: Vec<&str> = found.iter().map(|o| o.what).collect();
                eprintln!("\u{26a0}\u{fe0f} destructive command ({}): `{shown}`", what.join(", "));
            }

            // ADR-2611160730 §4: a Bash command that writes project source is
            // an edit, held to the same gate as Edit/Write. A claude hexa
            // started runs under the gate of the verb that started it.
            let internal = std::env::var("HEXA_INTERNAL").map(|v| !v.is_empty()).unwrap_or(false);
            if !internal {
                let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
                let cwd = hook_payload()["cwd"]
                    .as_str()
                    .filter(|c| !c.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| project_dir.clone()));
                let targets = write_shape::write_targets(command, &cwd, &source_gate::read_script_bounded);
                if !targets.is_empty() {
                    let mode = enforcement_mode(&project_dir);
                    source_gate_check(&project_dir, &targets, mode, &format!("`{}`", truncate_cmd(command, 60)), true);
                }
                // ADR-2611160910 finding 1: committing source is `hexa do
                // gate`; `git commit` is for prose.
                commit_gate_check(&project_dir, command, &cwd, enforcement_mode(&project_dir));
            }
        }
    }

    Ok(())
}

/// Hold writes to project source to the current task's gate
/// (ADR-2611160730 §4). `targets` are the files about to be written; those
/// that are project source in a hexa project (`.hexa/project.json`) and a
/// git repo are checked. With no current gate: exit 2 naming them in
/// mandatory mode, a warning in advisory mode. With `record`, when the write
/// goes ahead its targets are recorded as touched: the files themselves, not
/// the script or command that writes them. (An Edit is recorded by post-edit.)
fn source_gate_check(project_dir: &Path, targets: &[PathBuf], mode: &str, what: &str, record: bool) {
    if !project_dir.join(".hexa/project.json").is_file() {
        return;
    }
    let Some(root) = source_gate::repo_root(project_dir) else { return };
    let files = source_gate::project_source(&root, targets);
    if files.is_empty() {
        return;
    }
    let started = SessionState::load()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s.registered_at).ok());
    let entry = crate::commands::loop_cmd::read_loop(project_dir);
    if let Err(why) = source_gate::gate_is_current(entry.as_ref(), started) {
        let msg = format!(
            "No gate recorded for this work: {what} writes {} and {why}. Write the command that must exit 0, then `hexa loop gate '<command>'` (again after each `hexa loop task done`), then edit — or let `hexa do run` make the edit.",
            files.join(", ")
        );
        if mode == "mandatory" {
            // stderr: on exit 2 it is the only stream Claude Code shows,
            // and a block with no reason reads as a crash.
            eprintln!("\u{26d4} {msg}");
            crate::commands::events::exit(2);
        }
        println!("\u{26a0}\u{fe0f} {msg}");
    }
    if record {
        for f in &files {
            let _ = crate::commands::loop_cmd::touch(project_dir, f);
        }
    }
}

/// Print the compact graph context for an edit to `path`, the first time
/// this session edits it (ADR-2611221610 §6). Silent with no graph, for a file
/// the graph does not hold, and for a file already shown.
fn print_edit_context(project_dir: &Path, path: &Path) {
    let abs = if path.is_absolute() { path.to_path_buf() } else { project_dir.join(path) };
    let root = source_gate::canonical_lenient(project_dir);
    let abs = source_gate::canonical_lenient(&abs);
    let Ok(rel) = abs.strip_prefix(&root) else { return };
    let rel = rel.to_string_lossy().replace('\\', "/");
    if !project_dir.join(hexa_graph::GRAPH_FILE).is_file() && !project_dir.join(hexa_graph::LEGACY_GRAPH_FILE).is_file() {
        return;
    }
    let key: String = session_key().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let seen_file = project_dir.join(".hexa/graph/seen").join(key);
    let seen = std::fs::read_to_string(&seen_file).unwrap_or_default();
    if seen.lines().any(|l| l == rel) {
        return;
    }
    let lines = crate::commands::graph::edit_context(project_dir, &rel);
    if lines.is_empty() {
        return;
    }
    for l in &lines {
        println!("{l}");
    }
    println!();
    if let Some(d) = seen_file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&seen_file, format!("{seen}{rel}\n"));
}

/// Hold a hand `git commit` of project source to `hexa do gate`
/// (ADR-2611160910 finding 1). The usage eval's first baseline found that no
/// commit went through hexa: the model ran `git add <f> && git commit` every
/// time, and nothing refused it. A `git commit` in `command` whose staged or
/// included paths hold project source is refused (exit 2) in mandatory mode
/// and warned about in advisory mode, with the `hexa do gate` command that
/// commits those files filled in: the paths, the gate (`gate.full` from
/// `.hexa/project.json`, else this session's recorded gate) and the message.
///
/// Never refused: a commit of prose only (docs, ADRs, specs, markdown), a
/// commit while git replays existing commits (`MERGE_HEAD` and the like), a
/// commit in another repository, and hexa's own commits (`hexa …` commands
/// are not `git commit`; a claude hexa started carries `HEXA_INTERNAL`, and
/// the caller skips this for it).
fn commit_gate_check(project_dir: &Path, command: &str, cwd: &Path, mode: &str) {
    if !command.contains("commit") || !project_dir.join(".hexa/project.json").is_file() {
        return;
    }
    let Some(root) = source_gate::repo_root(project_dir) else { return };
    for plan in git_commit::commits(command, cwd) {
        if source_gate::repo_root(&plan.dir).as_deref() != Some(root.as_path()) {
            continue;
        }
        if git_commit::replay_in_progress(&plan.dir) {
            continue;
        }
        let files = source_gate::project_source(&root, &git_commit::included_paths(&root, &plan));
        if files.is_empty() {
            continue;
        }
        let gate = crate::commands::loop_cmd::full_gate(project_dir).or_else(|| {
            crate::commands::loop_cmd::read_loop(project_dir)
                .and_then(|e| e.get("gate").and_then(|g| g.as_str()).map(str::to_string))
                .filter(|g| !g.trim().is_empty())
        });
        let message = plan.message.clone().unwrap_or_else(|| "<what the change does>".to_string());
        let evidence = gate.clone().unwrap_or_else(|| "<the command that must pass>".to_string());
        let suggestion = format!(
            "hexa do gate --files {} -e {} -m {}",
            files.join(" "),
            git_commit::sh_quote(&evidence),
            git_commit::sh_quote(&message)
        );
        let record = if gate.is_none() {
            " No gate is recorded: write the command that must pass and record it first with `hexa loop gate '<command>'`."
        } else {
            ""
        };
        // Under test.remote the gate runs on the remote runner (ADR-2611221610 §9).
        let remote = hexa_exec::test_exec::test_config(&root).ok().and_then(|c| c.remote).is_some();
        let where_ = if remote {
            "It pushes a candidate commit, runs the gate on the remote runner (test.remote), and commits those files only if it exits 0 there."
        } else {
            "It runs the gate on the change in your tree and commits only those files, and only if it passes."
        };
        // The tests-first path, in every project (ADR-2611231400 §7).
        let red = " A tests-first commit (tests that must fail before the code) is the same command with `--expect-red`; `--dry-run` shows what either would do.";
        let msg = format!(
            "`git commit` would commit project source by hand ({}). Commit it through the gate instead: `{suggestion}`. {where_}{red}{record} `git commit` is for docs, ADRs and other prose.",
            files.join(", ")
        );
        if mode == "mandatory" {
            eprintln!("\u{26d4} {msg}");
            crate::commands::events::exit(2);
        }
        println!("\u{26a0}\u{fe0f} {msg}");
        return;
    }
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
    let input: serde_json::Value = serde_json::from_str(&tool_input_json()).unwrap_or(serde_json::Value::Null);
    // Claude Code sends the prompt as `prompt`; older hook shims sent `content`.
    let prompt = input["prompt"].as_str().or_else(|| input["content"].as_str());
    // A host's own notification — a finished background task, a monitor
    // event — arrives on the prompt channel and is not a person asking for
    // work. One was sized as a feature and drafted as a workplan
    // (ADR-2609131611).
    if prompt.is_some_and(is_host_notification) {
        return Ok(());
    }

    let mut state = SessionState::load_or_new();

    // The project block is session-start's, printed once. It was printed
    // here too, on every prompt whose key files had changed — which, with
    // a draft written per feature-sized prompt, was nearly every prompt.
    // Only a session whose start hook never ran gets it here, once.
    if state.fingerprint_generated_at.is_none() {
        if let Some((id, name)) = project_identity(project_dir) {
            println!("{}", fingerprint_block(&id, project_dir, &name).await);
            println!("[HEX] {}", crate::commands::loop_cmd::status_line(project_dir));
            for l in awareness_lines(project_dir) {
                println!("[HEX] {l}");
            }
        }
        state.fingerprint_generated_at = Some(chrono::Utc::now().to_rfc3339());
    }

    // What changed and matters, after that: another live session touching a
    // file this one touched, said once per file.
    for l in new_overlaps(project_dir, &mut state.reported_overlaps) {
        println!("[HEX] {l}");
    }

    let Some(content) = prompt else {
        let _ = state.save();
        return Ok(());
    };
    let lower = content.to_lowercase();

    let hints = route_hints(&lower);
    if !hints.is_empty() {
        println!("[HEX] Relevant: {}", hints.join("; "));
    }

    // ADR-2026-04-11-0227: every prompt is sized. pre-edit reads the size
    // of the *last* prompt; a stale T3 would block the typo fix that
    // follows a feature.
    let tier = classify_work_intent(&lower);
    state.last_tier = Some(tier.as_str().to_string());

    if matches!(tier, Tier::T2MiniPlan | Tier::T3Workplan) {
        archive_stale_task_json(project_dir);
    }

    if tier == Tier::T3Workplan {
        // A feature-sized prompt with no gate recorded: the one thing worth
        // saying before the work starts. With a gate in flight, nothing.
        if !gate_in_flight(project_dir) {
            println!(
                "[HEX] feature-sized task and no loop gate recorded. Design (`hexa design brainstorm \"<idea>\"`, optional) \u{2192} Decide (ADR in docs/adrs/, `hexa loop adr <ID>`) \u{2192} Gate (`hexa loop gate '<cmd>'`, before the code) \u{2192} Build \u{2192} Harden \u{2192} `hexa analyze .`"
            );
        }
        let drafting = auto_plan_enabled(project_dir)
            && state.workplan_id.is_none()
            && state.pending_workplan_draft.is_none()
            && !is_agent_brief(content, hook_payload());
        if drafting {
            match spawn_plan_draft(project_dir, content) {
                Ok(draft_path) => {
                    println!(
                        "[HEX] workplan draft: {draft_path} (`hexa plan drafts approve <name>` keeps it, `hexa plan drafts clear --name <name>` drops it)"
                    );
                    state.pending_workplan_draft = Some(draft_path);
                }
                Err(e) => eprintln!("hexa: workplan draft failed: {e}"),
            }
        }
    }

    let _ = state.save();
    Ok(())
}

/// The project's id and name from `.hexa/project.json`, if it is a hexa project.
fn project_identity(project_dir: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(project_dir.join(".hexa/project.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let id = v["id"].as_str().unwrap_or("?").to_string();
    let name = v["name"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| crate::commands::loop_cmd::project_name(project_dir));
    Some((id, name))
}

/// P2.2: archive a stale `.hexa/task.json` when on main with a new T2/T3
/// task. Worktree branches have a valid task.json — only archive on main.
fn archive_stale_task_json(project_dir: &Path) {
    let task_json = project_dir.join(".hexa/task.json");
    if !task_json.exists() {
        return;
    }
    let on_main = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "main");
    if !on_main {
        return;
    }
    let history_dir = project_dir.join(".hexa/task-history");
    let _ = std::fs::create_dir_all(&history_dir);
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    match std::fs::rename(&task_json, history_dir.join(format!("{ts}.json"))) {
        Ok(()) => eprintln!("hexa: archived stale task.json → .hexa/task-history/{ts}.json"),
        Err(e) => eprintln!("hexa: failed to archive task.json: {e}"),
    }
}

/// Most files named in one awareness or overlap line.
const FILES_SHOWN: usize = 5;

/// A touched-file entry worth showing another session: a path inside this
/// repository. `touch` records a file inside the project relative to it and
/// anything else absolute, so an absolute path is outside — a scratch
/// script, another checkout. Build output and temp folders are never a
/// boundary either.
fn is_repo_file(f: &str) -> bool {
    const NOISE: [&str; 6] = ["tmp", "temp", "scratchpad", "target", "node_modules", ".."];
    !f.is_empty()
        && !f.starts_with('/')
        && !f.starts_with('~')
        && !f.split('/').any(|seg| NOISE.contains(&seg))
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn files_list(files: &[&str]) -> String {
    let shown: Vec<&str> = files.iter().take(FILES_SHOWN).copied().collect();
    let more = files.len().saturating_sub(FILES_SHOWN);
    format!("{}{}", shown.join(", "), if more > 0 { format!(" +{more}") } else { String::new() })
}

/// One line per other *live* session, with the repository files it has
/// touched. Ended sessions are history, and a scratch path is nobody's
/// boundary; both filled the session-start block with hundreds of
/// characters no one could act on.
fn awareness_lines(project_dir: &Path) -> Vec<String> {
    crate::commands::loop_cmd::others(project_dir)
        .into_iter()
        .filter(|o| o.alive)
        .map(|o| {
            let files: Vec<&str> = o.files.iter().rev().map(String::as_str).filter(|f| is_repo_file(f)).collect();
            let touched = if files.is_empty() { String::new() } else { format!(" · touched {}", files_list(&files)) };
            let gate: String = if o.gate.chars().count() > 80 {
                format!("{}\u{2026}", o.gate.chars().take(80).collect::<String>())
            } else {
                o.gate.clone()
            };
            let run = o.running.as_ref().map(|r| format!(" · {r}")).unwrap_or_default();
            format!("also here: session {}{run} · stage {} · ADR {} · gate {gate}{touched}", short_id(&o.session), o.stage, o.adr)
        })
        .collect()
}

/// Overlaps not yet reported: for each other live session, the repository
/// files both it and this session have touched. Each `session|file` pair is
/// added to `reported`, so it is said once.
fn new_overlaps(project_dir: &Path, reported: &mut Vec<String>) -> Vec<String> {
    let mine: Vec<String> = crate::commands::loop_cmd::read_loop(project_dir)
        .and_then(|st| st.get("files").and_then(|f| f.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|f| f.as_str().map(str::to_string))
        .filter(|f| is_repo_file(f))
        .collect();
    if mine.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for o in crate::commands::loop_cmd::others(project_dir).into_iter().filter(|o| o.alive) {
        let fresh: Vec<&str> = o
            .files
            .iter()
            .rev()
            .map(String::as_str)
            .filter(|f| mine.iter().any(|m| m == f))
            .filter(|f| !reported.contains(&format!("{}|{f}", o.session)))
            .collect();
        if fresh.is_empty() {
            continue;
        }
        lines.push(format!(
            "session {} (live · stage {} · ADR {}) has also touched {}. Coordinate before overlapping.",
            short_id(&o.session),
            o.stage,
            o.adr,
            files_list(&fresh)
        ));
        reported.extend(fresh.iter().map(|f| format!("{}|{f}", o.session)));
    }
    lines
}

/// Text the host generated to tell the session something happened, rather
/// than a person asking for work.
fn is_host_notification(text: &str) -> bool {
    // A message from another Claude session arrives wrapped the same way: it
    // is a peer's note, not a person asking this session for work.
    const MARKERS: [&str; 5] =
        ["[SYSTEM NOTIFICATION", "<task-notification>", "<system-reminder>", "<local-command-caveat>", "<cross-session-message"];
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

/// ADR-2026-04-11-0227: run `hexa plan draft --background <prompt>` in the
/// project and return the path of the draft it wrote, which it prints. The
/// path was found by taking the newest file in the drafts folder, which is
/// another session's draft whenever two prompts land in the same moment.
fn spawn_plan_draft(project_dir: &Path, prompt: &str) -> Result<String> {
    let hexa_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hexa"));
    let output = std::process::Command::new(&hexa_bin)
        .args(["plan", "draft", "--background", prompt])
        .current_dir(project_dir)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("hexa plan draft failed: {}", stderr.trim()));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("hexa plan draft wrote no draft"))
}

// ── Boundary validation ──────────────────────────────────────────────

/// The file as the edit would leave it: a Write's `content`, or the file
/// with each `old_string` → `new_string` of an Edit or MultiEdit applied in
/// order. `None` when an `old_string` is not in the file: the tool refuses
/// that edit itself, so there is nothing to judge.
fn edit_result(before: &str, input: &serde_json::Value) -> Option<String> {
    if let Some(c) = input["content"].as_str() {
        return Some(c.to_string());
    }
    let apply = |text: String, e: &serde_json::Value| -> Option<String> {
        let old = e["old_string"].as_str()?;
        let new = e["new_string"].as_str().unwrap_or("");
        if old.is_empty() || !text.contains(old) {
            return None;
        }
        Some(if e["replace_all"].as_bool() == Some(true) { text.replace(old, new) } else { text.replacen(old, new, 1) })
    };
    match input["edits"].as_array() {
        Some(edits) => edits.iter().try_fold(before.to_string(), apply),
        None => apply(before.to_string(), input),
    }
}

/// Hold an edit to the analyzer's layer rules (`hexa analyze`'s, from
/// hexa-analysis), on the content the edit would leave. Only an import the
/// edit *adds* counts: a file that already breaks a rule can still be
/// edited, and its old violations are `hexa analyze`'s to report. In
/// mandatory mode a new violation is refused with exit 2, naming the rule
/// and the line; in advisory mode it is a warning on stderr.
///
/// This had a branch per layer and nothing in any of them, and returned Ok
/// for a domain file importing an adapter.
async fn validate_boundary_edit(project_dir: &Path, file_path: &str, input: &serde_json::Value) -> Result<()> {
    if !project_dir.join(".hexa/project.json").is_file() {
        return Ok(());
    }
    let abs = if Path::new(file_path).is_absolute() { PathBuf::from(file_path) } else { project_dir.join(file_path) };
    let Ok(rel) = abs.strip_prefix(project_dir) else { return Ok(()) };
    let rel = rel.to_string_lossy().to_string();
    let before = std::fs::read_to_string(&abs).unwrap_or_default();
    let Some(after) = edit_result(&before, input) else { return Ok(()) };

    let ast = hexa_analysis::default_ast();
    let check = |src: String| {
        let rel = rel.clone();
        let ast = ast.clone();
        async move { hexa_analysis::analyzer::source_violations(project_dir, &rel, &src, &*ast).await }
    };
    // The analyzer could not read the edit: say so, and let the edit
    // through. `hexa analyze` grades the file once it is written.
    let after_v = match check(after).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("hexa: boundary check skipped for {rel}: {e}");
            return Ok(());
        }
    };
    if after_v.is_empty() {
        return Ok(());
    }
    // Line numbers shift under an edit; an import is the same one when its
    // rule and its target are.
    let old: Vec<(String, String)> =
        check(before).await.unwrap_or_default().into_iter().map(|v| (v.rule, v.edge.to_file)).collect();
    let added: Vec<_> = after_v
        .iter()
        .filter(|v| !old.iter().any(|(rule, to)| *rule == v.rule && *to == v.edge.to_file))
        .collect();
    if added.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = added
        .iter()
        .map(|v| {
            format!(
                "  {rel} line {}: {} (`{}`: {} \u{2192} {})",
                v.edge.line, v.rule, v.edge.import_path, v.edge.from_layer, v.edge.to_layer
            )
        })
        .collect();
    let msg = format!(
        "This edit adds {} layer violation{}:\n{}\nImport through a port instead; `hexa analyze .` shows the rules.",
        added.len(),
        if added.len() == 1 { "" } else { "s" },
        lines.join("\n")
    );
    if enforcement_mode(project_dir) == "mandatory" {
        eprintln!("\u{26d4} {msg}");
        crate::commands::events::exit(2);
    }
    eprintln!("\u{26a0}\u{fe0f} {msg}");
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
            eprintln!(
                "BLOCKED: File '{}' is outside workplan boundary. Declared files: {:?}",
                rel,
                &declared_files[..declared_files.len().min(5)]
            );
            crate::commands::events::exit(2);
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

/// The key a restart checkpoint is stored under.
///
/// Project AND agent id (ADR-2609211200). Two sessions in two repositories can
/// carry the same agent id, and a checkpoint holds the open workplan, the stage
/// and the edit count — so keying on the agent id alone let a session in project
/// B resume into project A's state. Memory is project-scoped now, which
/// separates them already; this keeps them apart in the shared store too, where
/// `$HEXA_HOME` is set to one directory for every project.
fn checkpoint_key(project: &str, agent_id: &str) -> String {
    let slug = |s: &str| {
        let out: String = s
            .trim()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        let out = out.trim_matches('-').to_string();
        if out.is_empty() { "unknown".to_string() } else { out }
    };
    format!("restart:checkpoint:{}:{}", slug(project), slug(agent_id))
}

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
    let _ = hexa_exec::ports::MemoryStore::put(&hexa_exec::memory(), hexa_exec::ports::MemoryScope::Project, 
        &checkpoint_key(&state.project, &state.agent_id),
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
    let key = checkpoint_key(&state.project, &state.agent_id);
    let Some(raw) = hexa_exec::ports::MemoryStore::get(&hexa_exec::memory(), hexa_exec::ports::MemoryScope::Project, &key) else {
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
    let _ = hexa_exec::ports::MemoryStore::delete(&hexa_exec::memory(), hexa_exec::ports::MemoryScope::Project, &key);
    Ok(())
}

/// Load the active workplan from local memory into session state (ADR-050).
async fn load_workplan_context(project_id: &str) -> Result<()> {
    let Some(raw) = hexa_exec::ports::MemoryStore::get(&hexa_exec::memory(), hexa_exec::ports::MemoryScope::Project, &format!("workplan:active:{project_id}"))
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

/// One destructive operation found in a shell command.
#[derive(Debug, PartialEq)]
struct DestructiveOp {
    what: &'static str,
    /// A force-push whose target is main or master.
    protected: bool,
    branch: String,
}

/// `cmd` without the bodies of heredocs that only WRITE text. `cat > t.sh
/// <<'EOF' … EOF` makes a file; the lines in it are data, not commands run now,
/// so a force-push inside a test script it writes is not a force-push. A
/// heredoc whose line has a shell on it (`bash <<EOF`, `cat <<EOF | sh`) is
/// fed to that shell and stays, because there its body is run.
fn without_written_heredocs(cmd: &str) -> String {
    use std::sync::OnceLock;
    static START: OnceLock<regex::Regex> = OnceLock::new();
    let start = START.get_or_init(|| regex::Regex::new(r#"<<-?\s*(?:'([^']+)'|"([^"]+)"|\\?([A-Za-z_][A-Za-z0-9_]*))"#).unwrap());
    const SHELLS: [&str; 9] = ["bash", "sh", "zsh", "dash", "ksh", "source", ".", "eval", "xargs"];
    let mut out = String::new();
    let mut lines = cmd.lines();
    while let Some(line) = lines.next() {
        out.push_str(line);
        out.push('\n');
        let Some(c) = start.captures(line) else { continue };
        let tag = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let runs = line
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '|' | ';' | '&' | '(' | ')'))
            .any(|w| SHELLS.contains(&w.rsplit('/').next().unwrap_or(w)));
        if runs {
            continue;
        }
        // Drop the body, up to and including the terminator line.
        for body in lines.by_ref() {
            if body.trim() == tag {
                break;
            }
        }
    }
    out
}

/// The destructive operations in `cmd`, a shell command line run in `cwd`:
/// force-push, `reset --hard`, `clean -f`, `rm -rf` outside a temp folder,
/// and `branch -D`. Each `&&`/`||`/`;`/`|` segment is read on its own, with
/// git's global options (`-C <dir>`, `-c k=v`) skipped. A force-push names
/// its target branch: a refspec's destination, or the current branch of the
/// directory it runs in when it names none.
fn destructive_ops(cmd: &str, cwd: &Path) -> Vec<DestructiveOp> {
    let mut found = Vec::new();
    let cmd = &without_written_heredocs(cmd);
    let normalized = cmd.replace("&&", "\n").replace("||", "\n").replace([';', '|'], "\n");
    for segment in normalized.lines() {
        let toks: Vec<String> = segment
            .split_whitespace()
            .map(|t| t.trim_matches(|c| c == '"' || c == '\'').to_string())
            .collect();
        let mut i = 0;
        while i < toks.len() && (matches!(toks[i].as_str(), "sudo" | "command" | "exec" | "time" | "env") || is_assignment(&toks[i])) {
            i += 1;
        }
        let Some(prog) = toks.get(i) else { continue };
        let args = &toks[i + 1..];
        match prog.rsplit('/').next().unwrap_or(prog) {
            "git" => {
                let mut j = 0;
                let mut dir = cwd.to_path_buf();
                while j < args.len() && args[j].starts_with('-') {
                    if matches!(args[j].as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
                        if args[j] == "-C" {
                            if let Some(d) = args.get(j + 1) {
                                dir = cwd.join(d);
                            }
                        }
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                let Some(sub) = args.get(j) else { continue };
                let rest = &args[j + 1..];
                let short_has = |c: char| rest.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains(c));
                match sub.as_str() {
                    "push" => {
                        let positional: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
                        let refspecs = positional.get(1..).unwrap_or(&[]);
                        let force = short_has('f')
                            || rest.iter().any(|a| a == "--force" || a.starts_with("--force-with-lease") || a == "--mirror")
                            || refspecs.iter().any(|r| r.starts_with('+'));
                        if !force {
                            continue;
                        }
                        let dests: Vec<String> = if refspecs.is_empty() {
                            vec![current_branch(&dir).unwrap_or_default()]
                        } else {
                            refspecs
                                .iter()
                                .map(|r| {
                                    let d = r.trim_start_matches('+');
                                    let d = d.rsplit(':').next().unwrap_or(d).trim_start_matches("refs/heads/");
                                    if d == "HEAD" { current_branch(&dir).unwrap_or_default() } else { d.to_string() }
                                })
                                .collect()
                        };
                        let branch = dests.iter().find(|d| matches!(d.as_str(), "main" | "master")).cloned();
                        found.push(DestructiveOp { what: "force-push", protected: branch.is_some(), branch: branch.unwrap_or_default() });
                    }
                    "reset" if rest.iter().any(|a| a == "--hard") => found.push(op("reset --hard")),
                    "clean" if short_has('f') || rest.iter().any(|a| a == "--force") => found.push(op("clean -f")),
                    "branch" if rest.iter().any(|a| a == "-D") || (rest.iter().any(|a| a == "--delete") && rest.iter().any(|a| a == "--force")) => {
                        found.push(op("branch -D"))
                    }
                    _ => {}
                }
            }
            "rm" => {
                let flags: Vec<&String> = args.iter().filter(|a| a.starts_with('-')).collect();
                let short = |c: &[char]| flags.iter().any(|f| !f.starts_with("--") && f.contains(c));
                let recursive = short(&['r', 'R']) || flags.iter().any(|f| *f == "--recursive");
                let force = short(&['f']) || flags.iter().any(|f| *f == "--force");
                let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
                if recursive && force && targets.iter().any(|t| !is_temp_path(t)) {
                    found.push(op("rm -rf"));
                }
            }
            _ => {}
        }
    }
    found
}

fn op(what: &'static str) -> DestructiveOp {
    DestructiveOp { what, protected: false, branch: String::new() }
}

fn is_assignment(t: &str) -> bool {
    t.split_once('=').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// A path in a temp folder, where deleting is what the folder is for.
fn is_temp_path(t: &str) -> bool {
    let tmpdir = std::env::var("TMPDIR").unwrap_or_default();
    ["/tmp/", "/private/tmp/", "/var/folders/", "/private/var/folders/", "$TMPDIR", "${TMPDIR"].iter().any(|p| t.starts_with(p))
        || (!tmpdir.is_empty() && t.starts_with(&tmpdir))
}

fn current_branch(dir: &Path) -> Option<String> {
    let o = std::process::Command::new("git").args(["rev-parse", "--abbrev-ref", "HEAD"]).current_dir(dir).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
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

/// One route hint: shown when the prompt has one of `words` (and, when
/// `with` is not empty, one of `with` too), each a whole word or phrase.
/// `targets` are the verbs (`hexa …`) and skills (`/…`) the text names; a
/// test runs each verb's `--help` and finds each skill in `assets/skills`.
pub struct RouteHint {
    pub words: &'static [&'static str],
    pub with: &'static [&'static str],
    pub text: &'static str,
    pub targets: &'static [&'static str],
}

pub const ROUTE_HINTS: &[RouteHint] = &[
    RouteHint {
        words: &["scaffold", "new project", "init"],
        with: &[],
        text: "hexa scaffold or hexa init",
        targets: &["hexa scaffold", "hexa init"],
    },
    RouteHint {
        words: &["architecture", "architectural", "boundary", "boundaries", "layer violation", "layer violations", "hexagonal"],
        with: &[],
        text: "hexa analyze",
        targets: &["hexa analyze"],
    },
    RouteHint {
        words: &["adr", "adrs", "decision record", "decision records"],
        with: &[],
        text: "hexa adr list, hexa adr search",
        targets: &["hexa adr list", "hexa adr search"],
    },
    RouteHint {
        words: &["subagent", "subagents", "parallel", "worktree", "worktrees"],
        with: &[],
        text: "isolation: \"worktree\" on the Agent call; hexa worktree list",
        targets: &["hexa worktree list"],
    },
    RouteHint {
        words: &["feature"],
        with: &["implement", "build", "develop", "add"],
        text: "/hexa-workflow (the gate-first loop)",
        targets: &["/hexa-workflow"],
    },
];

/// Most hints on the one `Relevant:` line.
const MAX_HINTS: usize = 3;

/// Is `needle` in `hay` as a whole word or phrase? Both lower case. A
/// substring match fired the scaffold hint on "scaffolded" and the init
/// hint on "definitely".
fn contains_word(hay: &str, needle: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    hay.match_indices(needle).any(|(i, m)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + m.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

/// The hints for a lower-cased prompt, at most [`MAX_HINTS`].
fn route_hints(prompt: &str) -> Vec<&'static str> {
    ROUTE_HINTS
        .iter()
        .filter(|h| h.words.iter().any(|w| contains_word(prompt, w)))
        .filter(|h| h.with.is_empty() || h.with.iter().any(|w| contains_word(prompt, w)))
        .map(|h| h.text)
        .take(MAX_HINTS)
        .collect()
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

    // ─ ADR-2609211200: a checkpoint belongs to one project ─

    #[test]
    fn two_projects_sharing_an_agent_id_get_different_checkpoints() {
        assert_ne!(
            checkpoint_key("alpha", "agent-7"),
            checkpoint_key("beta", "agent-7"),
            "a checkpoint carries the open workplan and stage; project B must not resume into A's"
        );
        assert_eq!(checkpoint_key("alpha", "agent-7"), "restart:checkpoint:alpha:agent-7");
    }

    #[test]
    fn a_checkpoint_key_survives_a_project_named_like_a_path() {
        // Keys are `:`-delimited and read back by prefix. A project name holding
        // a separator would split one key into another's namespace.
        let key = checkpoint_key("/home/u/work: alpha", "agent-7");
        assert_eq!(key.matches(':').count(), 3, "exactly the three the key defines: {key}");
        assert_eq!(checkpoint_key("", "agent-7"), "restart:checkpoint:unknown:agent-7");
    }

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
    fn route_hints_match_whole_words() {
        // (prompt, the hint texts it must produce)
        let table: &[(&str, &[&str])] = &[
            ("please scaffold a new hexa project", &["hexa scaffold or hexa init"]),
            ("the scaffolded files", &[]),
            ("this is definitely fine", &[]),
            ("run hexa init here", &["hexa scaffold or hexa init"]),
            ("re-init the repo", &["hexa scaffold or hexa init"]),
            ("check the architecture", &["hexa analyze"]),
            ("the architect said so", &[]),
            ("read ADR-081 first", &["hexa adr list, hexa adr search"]),
            ("the gradr padre", &[]),
            ("act as the implementation sub-agent", &[]),
            ("run two subagents in parallel", &["isolation: \"worktree\" on the Agent call; hexa worktree list"]),
            ("implement the feature", &["/hexa-workflow (the gate-first loop)"]),
            ("featured articles to implement", &[]),
            ("just a random question", &[]),
        ];
        for (prompt, want) in table {
            assert_eq!(&route_hints(&prompt.to_lowercase()), want, "{prompt}");
        }
    }

    #[test]
    fn route_hints_are_capped_at_three() {
        let all = "scaffold the architecture, read the adr, run parallel worktrees, implement the feature";
        assert_eq!(route_hints(all).len(), MAX_HINTS);
    }

    #[test]
    fn destructive_ops_are_read_per_segment() {
        let cwd = std::env::temp_dir();
        let whats = |c: &str| destructive_ops(c, &cwd).iter().map(|o| o.what).collect::<Vec<_>>();
        assert_eq!(whats("git push --force origin feat"), ["force-push"]);
        assert_eq!(whats("git -C ../x push -f origin feat"), ["force-push"]);
        assert_eq!(whats("git push origin +feat"), ["force-push"]);
        assert_eq!(whats("cargo test && git reset --hard origin/main"), ["reset --hard"]);
        assert_eq!(whats("git clean -fdx"), ["clean -f"]);
        assert_eq!(whats("sudo rm -rf ./build"), ["rm -rf"]);
        assert_eq!(whats("rm -r -f build"), ["rm -rf"]);
        assert_eq!(whats("git branch -D old"), ["branch -D"]);
        // A heredoc that only WRITES a script is data, not a command run now; one a
        // shell reads is still a command.
        let wrote = "cat > t.sh <<'EOF'\ngit -C rel push -qf origin main\nrm -rf build\nEOF\nls";
        assert!(destructive_ops(wrote, &cwd).is_empty(), "a written script was read as commands: {:?}", destructive_ops(wrote, &cwd));
        let tee = "tee t.sh <<EOF >/dev/null\ngit push --force origin main\nEOF";
        assert!(destructive_ops(tee, &cwd).is_empty());
        for run in ["bash <<'EOF'\ngit push -f origin main\nEOF", "sh -s <<EOF\ngit push -f origin main\nEOF", "cat <<EOF | bash\ngit push -f origin main\nEOF"] {
            let ops = destructive_ops(run, &cwd);
            assert!(ops.iter().any(|o| o.protected), "a heredoc a shell runs must still be read: {run:?} -> {ops:?}");
        }
        let after = "cat > t.sh <<'EOF'\necho hi\nEOF\ngit push -f origin main";
        assert!(destructive_ops(after, &cwd).iter().any(|o| o.protected), "a command after the heredoc is still read");
        for safe in ["git push origin feat", "git branch -d merged", "rm -rf /tmp/x", "rm -rf $TMPDIR/x", "git clean -n", "echo git push --force-ish", "grep -rf pattern"] {
            assert!(whats(safe).iter().all(|w| *w != "force-push" && *w != "rm -rf" && *w != "clean -f"), "{safe}: {:?}", whats(safe));
        }
        let main = destructive_ops("git push -f origin HEAD:refs/heads/main", &cwd);
        assert!(main[0].protected && main[0].branch == "main", "{main:?}");
        assert!(!destructive_ops("git push -f origin mainline", &cwd)[0].protected);
    }

    #[test]
    fn an_edit_result_applies_write_edit_and_multiedit() {
        use serde_json::json as j;
        assert_eq!(edit_result("a", &j!({"content": "b"})).as_deref(), Some("b"));
        assert_eq!(edit_result("a a", &j!({"old_string": "a", "new_string": "b"})).as_deref(), Some("b a"));
        assert_eq!(edit_result("a a", &j!({"old_string": "a", "new_string": "b", "replace_all": true})).as_deref(), Some("b b"));
        assert_eq!(edit_result("a", &j!({"old_string": "z", "new_string": "b"})), None);
        let multi = j!({"edits": [{"old_string": "a", "new_string": "b"}, {"old_string": "b", "new_string": "c"}]});
        assert_eq!(edit_result("a", &multi).as_deref(), Some("c"));
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
