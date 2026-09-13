//! `hexa loop`: where this project's work stands in the loop.
//!
//! Decide (an ADR) → Gate (the command that must exit 0, written before the
//! code) → Build → Harden. The state is `.hexa/loop.json` in the project,
//! committed with the branch, so it travels with the pull request and a
//! reviewer sees which ADR the work is under and which gate proved it. The
//! ADR is the durable record; the loop file points at it and names the gate.
//!
//! The file also carries the checklist: the steps the work is made of, each
//! `todo`, `doing` or `done`, checked off with `hexa loop task done N`. The
//! status line the hooks print says how many are done and which is in
//! progress, so the feedback is there at session start and on every
//! feature-sized prompt.
//!
//! The hooks read it: session start prints it, a feature-sized prompt prints
//! it, and an edit in a feature-sized session with no gate recorded is stopped
//! until the gate is written. `hexa do`, `hexa build` and `hexa harden` record
//! their gate on their own.

use clap::Subcommand;
use colored::Colorize;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum LoopAction {
    /// Show where the work stands (the default)
    Show,
    /// Record the ADR this work is under; it must exist in docs/adrs/
    Adr {
        /// ADR id, e.g. ADR-2609121400
        id: String,
    },
    /// Record the gate: the command that must exit 0
    Gate {
        /// A shell command, e.g. "cargo test --test add"
        command: String,
    },
    /// Record the stage: decide, gate, build, harden or done
    Stage {
        stage: String,
    },
    /// Record the evidence: a command whose stdout is appended to the ADR when
    /// the stage is marked done (ADR-2609131341)
    Evidence {
        /// A shell command, e.g. "cargo test --test instrument -- --ignored --nocapture"
        command: String,
    },
    /// Forget the recorded state
    Clear,
    /// The checklist: the steps this work is made of, checked off as they land
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum TaskAction {
    /// Add a step to the end of the list
    Add {
        /// What the step is, e.g. "JunOS braces parser"; may begin with a dash
        #[arg(allow_hyphen_values = true)]
        title: String,
    },
    /// Mark step N as the one being worked on
    Start {
        n: usize,
    },
    /// Check step N off; the next unstarted step becomes the one in progress
    Done {
        n: usize,
    },
    /// Uncheck step N
    Undo {
        n: usize,
    },
    /// Remove step N
    Rm {
        n: usize,
    },
}

/// The name a project is recorded under: `.hexa/project.json` `name`, else the
/// directory name.
pub fn project_name(dir: &Path) -> String {
    let from_config = std::fs::read_to_string(dir.join(".hexa").join("project.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        .filter(|n| !n.is_empty());
    from_config.unwrap_or_else(|| {
        dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".to_string())
    })
}

/// Where the loop lives: `.hexa/loop.json` in the project.
fn loop_path(dir: &Path) -> PathBuf {
    dir.join(".hexa").join("loop.json")
}

/// Who is running hexa (ADR-2609131408 §2). The contract is hexa's own:
/// a host sets `HEXA_SESSION_ID` and `HEXA_SESSION_PID` for the commands
/// it runs. Claude Code's `CLAUDE_CODE_SESSION_ID` / `CLAUDE_PID` are
/// recognised natively. A plain terminal is its POSIX session — stable for
/// the life of the terminal, and its leader is a pid whose liveness can be
/// checked. Resolved from `env` and `posix_session` so the precedence is
/// testable without touching the process environment.
pub fn resolve_session(env: &dyn Fn(&str) -> Option<String>, posix_session: Option<u64>) -> (String, u64) {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    let pid_of = |k: &str| get(k).and_then(|v| v.parse::<u64>().ok());
    if let Some(id) = get("HEXA_SESSION_ID") {
        return (id, pid_of("HEXA_SESSION_PID").or(posix_session).unwrap_or(0));
    }
    if let Some(id) = get("CLAUDE_CODE_SESSION_ID").or_else(|| get("CLAUDE_SESSION_ID")) {
        return (id, pid_of("CLAUDE_PID").or(posix_session).unwrap_or(0));
    }
    match posix_session {
        Some(sid) => (format!("local:{sid}"), sid),
        None => ("local".to_string(), 0),
    }
}

/// The POSIX session id of this process: field 6 of /proc/self/stat, after
/// the parenthesised command name. `None` where /proc is not available.
fn posix_session() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(3)?.parse().ok()
}

pub fn session_id() -> String {
    resolve_session(&|k| std::env::var(k).ok(), posix_session()).0
}

fn session_pid() -> u64 {
    resolve_session(&|k| std::env::var(k).ok(), posix_session()).1
}

/// A session is live while its process exists. A pid of 0 is unknown and
/// counts as ended.
pub fn pid_alive(pid: u64) -> bool {
    pid != 0 && Path::new(&format!("/proc/{pid}")).exists()
}

/// The whole file: `{"sessions": {id: entry}}`, or a flat entry from before
/// ADR-2609131408.
fn read_file(dir: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(loop_path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// The entries by session. A flat pre-ADR file is the entry of whoever asks.
fn entries(file: &serde_json::Value, asking: &str) -> serde_json::Map<String, serde_json::Value> {
    if let Some(m) = file.get("sessions").and_then(|v| v.as_object()) {
        return m.clone();
    }
    let mut m = serde_json::Map::new();
    if file.as_object().is_some_and(|o| !o.is_empty()) {
        m.insert(asking.to_string(), file.clone());
    }
    m
}

/// One session's recorded state, if any.
pub fn read_entry(dir: &Path, session: &str) -> Option<serde_json::Value> {
    read_file(dir).and_then(|f| entries(&f, session).get(session).cloned())
}

/// This session's recorded state, if any.
pub fn read_loop(dir: &Path) -> Option<serde_json::Value> {
    read_entry(dir, &session_id())
}

fn write_entries(dir: &Path, m: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let p = loop_path(dir);
    if m.is_empty() {
        if p.is_file() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&serde_json::json!({ "sessions": m })).map_err(|e| e.to_string())? + "\n";
    std::fs::write(p, text).map_err(|e| e.to_string())
}

/// Merge `patch` into one session's entry and write the file. Only in a
/// project that has a `.hexa/` directory; elsewhere there is nothing to
/// record into.
pub fn update_entry(dir: &Path, session: &str, pid: u64, patch: serde_json::Value) -> Result<serde_json::Value, String> {
    if !dir.join(".hexa").is_dir() {
        return Err(format!("{} has no .hexa/ directory; run `hexa init .` first", dir.display()));
    }
    let mut m = read_file(dir).map(|f| entries(&f, session)).unwrap_or_default();
    let mut state = m.get(session).cloned().unwrap_or_else(|| serde_json::json!({}));
    if let (Some(obj), Some(p)) = (state.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            obj.insert(k.clone(), v.clone());
        }
        obj.insert("pid".to_string(), serde_json::json!(pid));
        obj.insert("updated".to_string(), serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
    }
    m.insert(session.to_string(), state.clone());
    write_entries(dir, m)?;
    Ok(state)
}

/// Merge `patch` into this session's entry and write it.
pub fn update_loop(dir: &Path, patch: serde_json::Value) -> Result<serde_json::Value, String> {
    update_entry(dir, &session_id(), session_pid(), patch)
}

/// Remove one session's entry; the file goes with the last one. Returns
/// whether there was an entry.
pub fn clear_entry(dir: &Path, session: &str) -> Result<bool, String> {
    let Some(f) = read_file(dir) else { return Ok(false) };
    let mut m = entries(&f, session);
    let was = m.remove(session).is_some();
    write_entries(dir, m)?;
    Ok(was)
}

fn clear_loop(dir: &Path) -> Result<bool, String> {
    clear_entry(dir, &session_id())
}

/// Another session's entry, as the awareness lines show it.
#[derive(Debug, Clone, PartialEq)]
pub struct Other {
    pub session: String,
    pub alive: bool,
    pub adr: String,
    pub gate: String,
    pub stage: String,
    pub files: Vec<String>,
    pub updated: String,
}

/// Every session but `session`, liveness judged by `alive`.
pub fn others_of(dir: &Path, session: &str, alive: &dyn Fn(u64) -> bool) -> Vec<Other> {
    let Some(f) = read_file(dir) else { return Vec::new() };
    let mut out: Vec<Other> = entries(&f, session)
        .iter()
        .filter(|(id, _)| id.as_str() != session)
        .map(|(id, e)| Other {
            session: id.clone(),
            alive: alive(e.get("pid").and_then(|v| v.as_u64()).unwrap_or(0)),
            adr: e.get("adr").and_then(|v| v.as_str()).unwrap_or("none").to_string(),
            gate: e.get("gate").and_then(|v| v.as_str()).unwrap_or("none").to_string(),
            stage: e.get("stage").and_then(|v| v.as_str()).unwrap_or("decide").to_string(),
            files: e
                .get("files")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            updated: e.get("updated").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        })
        .collect();
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

pub fn others(dir: &Path) -> Vec<Other> {
    others_of(dir, &session_id(), &pid_alive)
}

/// How many files a session's entry remembers.
const TOUCHED_CAP: usize = 40;

/// Record that `session` edited `path`: the boundary another session needs
/// to see. Deduplicated, most recent last, capped.
pub fn touch_as(dir: &Path, session: &str, pid: u64, path: &str) -> Result<(), String> {
    let rel = Path::new(path).strip_prefix(dir).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string());
    let mut files: Vec<String> = read_entry(dir, session)
        .and_then(|e| e.get("files").and_then(|v| v.as_array()).cloned())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    files.retain(|f| f != &rel);
    files.push(rel);
    if files.len() > TOUCHED_CAP {
        files.drain(..files.len() - TOUCHED_CAP);
    }
    update_entry(dir, session, pid, serde_json::json!({ "files": files })).map(|_| ())
}

pub fn touch(dir: &Path, path: &str) -> Result<(), String> {
    touch_as(dir, &session_id(), session_pid(), path)
}

/// The live other sessions that have touched `path`.
pub fn touched_by_others_of(dir: &Path, session: &str, path: &str, alive: &dyn Fn(u64) -> bool) -> Vec<Other> {
    let rel = Path::new(path).strip_prefix(dir).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string());
    others_of(dir, session, alive).into_iter().filter(|o| o.alive && o.files.iter().any(|f| f == &rel)).collect()
}

pub fn touched_by_others(dir: &Path, path: &str) -> Vec<Other> {
    touched_by_others_of(dir, &session_id(), path, &pid_alive)
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// One line per other session, for the hooks and `hexa loop`: live ones
/// with what they are under and where they have been; ended ones marked.
pub fn awareness_lines(dir: &Path) -> Vec<String> {
    others(dir)
        .iter()
        .map(|o| {
            let files = if o.files.is_empty() {
                String::new()
            } else {
                let shown: Vec<&str> = o.files.iter().rev().take(5).map(String::as_str).collect();
                let more = o.files.len().saturating_sub(5);
                format!(" · touched {}{}", shown.join(", "), if more > 0 { format!(" +{more}") } else { String::new() })
            };
            if o.alive {
                format!("also here: session {} · stage {} · ADR {} · gate {}{}", short(&o.session), o.stage, o.adr, o.gate, files)
            } else {
                format!("ended: session {} · stage {} · ADR {}{}", short(&o.session), o.stage, o.adr, files)
            }
        })
        .collect()
}

/// The ADR file in `docs/adrs/` whose name starts with `id`.
fn adr_path(dir: &Path, id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir.join("docs").join("adrs")).ok()?;
    entries.flatten().map(|e| e.path()).find(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        name.starts_with(id) && name.ends_with(".md")
    })
}

/// Does `docs/adrs/` hold an ADR whose file name starts with `id`?
fn adr_exists(dir: &Path, id: &str) -> bool {
    adr_path(dir, id).is_some()
}

/// Run the evidence command and append its stdout to the ADR under
/// `## Evidence`, with the command, the commit and the time. A failing
/// command appends nothing and is an error: evidence comes from a run that
/// succeeded (ADR-2609131341).
pub fn record_evidence(dir: &Path, adr_id: &str, command: &str) -> Result<PathBuf, String> {
    let path = adr_path(dir, adr_id).ok_or_else(|| format!("no {adr_id} in docs/adrs/ to append evidence to"))?;
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run the evidence command: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!(
            "evidence command failed ({}); nothing appended to {}\n{}",
            out.status,
            path.display(),
            tail.join("\n")
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = match git(&["rev-parse", "--short", "HEAD"]) {
        Some(sha) if git(&["status", "--porcelain"]).map_or(true, |s| s.is_empty()) => sha,
        Some(sha) => format!("{sha} with uncommitted changes"),
        None => "no commit".to_string(),
    };
    let mut text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.contains("\n## Evidence\n") {
        text.push_str("\n## Evidence\n");
    }
    text.push_str(&format!(
        "\n`{}` at {} on {}:\n\n```text\n{}\n```\n",
        command,
        commit,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
        stdout
    ));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// One line for the hooks: the state, or what is missing.
pub fn status_line(dir: &Path) -> String {
    let project = project_name(dir);
    match read_loop(dir) {
        Some(st) => {
            let adr = st.get("adr").and_then(|v| v.as_str()).unwrap_or("none");
            let gate = st.get("gate").and_then(|v| v.as_str()).unwrap_or("none");
            let stage = st.get("stage").and_then(|v| v.as_str()).unwrap_or("decide");
            let mut line = format!("Loop ({project}): stage {stage} · ADR {adr} · gate {gate}");
            let tasks = task_list(&st);
            if !tasks.is_empty() {
                let done = tasks.iter().filter(|t| t.status == "done").count();
                line.push_str(&format!(" · tasks {done}/{}", tasks.len()));
                if let Some(t) = tasks.iter().find(|t| t.status == "doing") {
                    line.push_str(&format!(" · doing {} {}", t.n, t.title));
                }
            }
            line
        }
        None => format!(
            "Loop ({project}): nothing recorded. Decide → Gate → Build → Harden. Record with `hexa loop adr <ID>` and `hexa loop gate '<command>'`."
        ),
    }
}

const STAGES: &[&str] = &["decide", "gate", "build", "harden", "done"];

/// One step of the work. `status` is `todo`, `doing` or `done`.
#[derive(Debug, Clone)]
pub struct Task {
    pub n: usize,
    pub title: String,
    pub status: String,
}

/// The checklist in a loop state, numbered from 1.
fn task_list(state: &serde_json::Value) -> Vec<Task> {
    state
        .get("tasks")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(i, t)| Task {
                    n: i + 1,
                    title: t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    status: t.get("status").and_then(|v| v.as_str()).unwrap_or("todo").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The checklist as lines: `[x]` done, `[>]` in progress, `[ ]` to do.
fn checklist(state: &serde_json::Value) -> Vec<String> {
    let tasks = task_list(state);
    let mut out: Vec<String> = tasks
        .iter()
        .map(|t| {
            let mark = match t.status.as_str() {
                "done" => "[x]",
                "doing" => "[>]",
                _ => "[ ]",
            };
            format!("{mark} {} {}", t.n, t.title)
        })
        .collect();
    if !tasks.is_empty() {
        let done = tasks.iter().filter(|t| t.status == "done").count();
        out.push(format!("{done} of {} done", tasks.len()));
    }
    out
}

fn write_tasks(dir: &Path, tasks: Vec<serde_json::Value>) -> anyhow::Result<()> {
    update_loop(dir, serde_json::json!({ "tasks": tasks })).map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

fn tasks_json(dir: &Path) -> Vec<serde_json::Value> {
    read_loop(dir)
        .and_then(|s| s.get("tasks").and_then(|t| t.as_array()).cloned())
        .unwrap_or_default()
}

fn run_task(dir: &Path, action: TaskAction) -> anyhow::Result<()> {
    let mut tasks = tasks_json(dir);
    let index = |n: usize, len: usize| -> anyhow::Result<usize> {
        if n == 0 || n > len {
            anyhow::bail!("no step {n}; the list has {len}");
        }
        Ok(n - 1)
    };
    match action {
        TaskAction::Add { title } => {
            let title = title.trim().to_string();
            if title.is_empty() {
                anyhow::bail!("a step needs a title");
            }
            // The first step added to an empty list is the one in progress.
            let status = if tasks.is_empty() { "doing" } else { "todo" };
            tasks.push(serde_json::json!({ "title": title, "status": status }));
        }
        TaskAction::Start { n } => {
            let i = index(n, tasks.len())?;
            for t in tasks.iter_mut() {
                if t.get("status").and_then(|v| v.as_str()) == Some("doing") {
                    t["status"] = serde_json::json!("todo");
                }
            }
            tasks[i]["status"] = serde_json::json!("doing");
        }
        TaskAction::Done { n } => {
            let i = index(n, tasks.len())?;
            tasks[i]["status"] = serde_json::json!("done");
            tasks[i]["done_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
            // The next unstarted step becomes the one in progress, unless one is.
            let any_doing = tasks.iter().any(|t| t.get("status").and_then(|v| v.as_str()) == Some("doing"));
            if !any_doing {
                if let Some(next) = tasks.iter_mut().find(|t| t.get("status").and_then(|v| v.as_str()) == Some("todo")) {
                    next["status"] = serde_json::json!("doing");
                }
            }
        }
        TaskAction::Undo { n } => {
            let i = index(n, tasks.len())?;
            tasks[i]["status"] = serde_json::json!("todo");
            if let Some(obj) = tasks[i].as_object_mut() {
                obj.remove("done_at");
            }
        }
        TaskAction::Rm { n } => {
            let i = index(n, tasks.len())?;
            tasks.remove(i);
        }
    }
    write_tasks(dir, tasks)?;
    println!("{} {}", "\u{2b21}".green(), status_line(dir));
    for l in checklist(&read_loop(dir).unwrap_or_default()) {
        println!("  {l}");
    }
    Ok(())
}

pub async fn run(action: Option<LoopAction>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    match action.unwrap_or(LoopAction::Show) {
        LoopAction::Show => {
            println!("{} {}", "\u{2b21}".cyan(), status_line(&cwd));
            if let Some(st) = read_loop(&cwd) {
                for l in checklist(&st) {
                    println!("  {l}");
                }
                if let Some(e) = st.get("evidence").and_then(|v| v.as_str()) {
                    println!("  evidence {e}");
                }
                if let Some(u) = st.get("updated").and_then(|v| v.as_str()) {
                    println!("  updated {u}");
                }
                println!("  file    {}", loop_path(&cwd).display());
            }
            for l in awareness_lines(&cwd) {
                println!("  {l}");
            }
        }
        LoopAction::Task { action } => run_task(&cwd, action)?,
        LoopAction::Adr { id } => {
            if !adr_exists(&cwd, &id) {
                anyhow::bail!(
                    "no {id} in docs/adrs/. The ADR is the record a reviewer reads; write it first, then record it here."
                );
            }
            let stage = read_loop(&cwd)
                .and_then(|s| s.get("stage").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_else(|| "decide".to_string());
            update_loop(&cwd, serde_json::json!({ "adr": id, "stage": stage })).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&cwd));
        }
        LoopAction::Gate { command } => {
            let command = command.trim().to_string();
            if command.is_empty() {
                anyhow::bail!("a gate is a command; it cannot be empty");
            }
            update_loop(&cwd, serde_json::json!({ "gate": command, "stage": "gate" })).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&cwd));
        }
        LoopAction::Evidence { command } => {
            let command = command.trim().to_string();
            if command.is_empty() {
                anyhow::bail!("evidence is a command; it cannot be empty");
            }
            update_loop(&cwd, serde_json::json!({ "evidence": command })).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&cwd));
            println!("  evidence {command}");
        }
        LoopAction::Stage { stage } => {
            let stage = stage.to_lowercase();
            if !STAGES.contains(&stage.as_str()) {
                anyhow::bail!("stage must be one of: {}", STAGES.join(", "));
            }
            // Done means measured: the evidence command runs now, and its
            // output lands in the ADR before the stage is recorded.
            if stage == "done" {
                if let Some(st) = read_loop(&cwd) {
                    let adr = st.get("adr").and_then(|v| v.as_str());
                    let evidence = st.get("evidence").and_then(|v| v.as_str());
                    if let (Some(adr), Some(cmd)) = (adr, evidence) {
                        let path = record_evidence(&cwd, adr, cmd).map_err(|e| anyhow::anyhow!(e))?;
                        println!("  evidence appended to {}", path.display());
                    }
                }
            }
            update_loop(&cwd, serde_json::json!({ "stage": stage })).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&cwd));
        }
        LoopAction::Clear => {
            let was = clear_loop(&cwd).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".yellow(), if was { "this session's loop state cleared" } else { "nothing was recorded for this session" });
        }
    }
    Ok(())
}

#[cfg(test)]
mod evidence_tests {
    use super::record_evidence;

    fn project() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hexa-evidence-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".hexa")).unwrap();
        std::fs::create_dir_all(dir.join("docs/adrs")).unwrap();
        std::fs::write(dir.join("docs/adrs/ADR-1-a-decision.md"), "# ADR-1: a decision\n\n**Status:** Accepted\n").unwrap();
        dir
    }

    /// ADR-2609131341: the evidence command's stdout lands under `## Evidence`
    /// with the command and the time; a second run appends without a second
    /// heading; a failing command appends nothing and is an error.
    #[test]
    fn evidence_is_appended_to_the_adr_once_per_done_and_never_from_a_failing_run() {
        let dir = project();
        let path = record_evidence(&dir, "ADR-1", "printf 'precision 0.833\\nrecall 1.000\\n'").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\n## Evidence\n"), "{text}");
        assert!(text.contains("`printf 'precision 0.833\\nrecall 1.000\\n'` at no commit on "), "{text}");
        assert!(text.contains("```text\nprecision 0.833\nrecall 1.000\n```\n"), "{text}");

        record_evidence(&dir, "ADR-1", "echo second").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("## Evidence").count(), 1, "one heading, two entries: {text}");
        assert!(text.contains("```text\nsecond\n```"), "{text}");

        let before = text.clone();
        let err = record_evidence(&dir, "ADR-1", "echo broken >&2; exit 3").unwrap_err();
        assert!(err.contains("failed") && err.contains("broken"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "a failing run appends nothing");

        assert!(record_evidence(&dir, "ADR-9", "true").is_err(), "no such ADR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod sessions_see_each_other {
    use super::*;

    fn project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hexa-sessions-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".hexa")).unwrap();
        dir
    }

    fn live(pid: u64) -> bool {
        pid == 1
    }

    /// The contract is hexa's own; Claude Code's variables are one host's
    /// spelling of it; a terminal is its POSIX session.
    #[test]
    fn the_session_is_hexas_own_variable_then_the_hosts_then_the_terminal() {
        let env = |vars: &'static [(&'static str, &'static str)]| move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(resolve_session(&env(&[("HEXA_SESSION_ID", "codex-7"), ("HEXA_SESSION_PID", "4242"), ("CLAUDE_CODE_SESSION_ID", "c1")]), Some(9)), ("codex-7".to_string(), 4242));
        assert_eq!(resolve_session(&env(&[("CLAUDE_CODE_SESSION_ID", "c1"), ("CLAUDE_PID", "77")]), Some(9)), ("c1".to_string(), 77));
        assert_eq!(resolve_session(&env(&[("CLAUDE_SESSION_ID", "c2")]), Some(9)), ("c2".to_string(), 9), "no pid from the host: the terminal's session leader");
        assert_eq!(resolve_session(&env(&[("HEXA_SESSION_ID", "")]), Some(9)), ("local:9".to_string(), 9), "empty is unset");
        assert_eq!(resolve_session(&env(&[]), None), ("local".to_string(), 0));
    }

    #[test]
    fn two_sessions_record_two_adrs_and_each_reads_its_own() {
        let dir = project();
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A", "gate": "cargo test a", "stage": "build"})).unwrap();
        update_entry(&dir, "bbbb", 2, serde_json::json!({"adr": "ADR-B", "stage": "gate"})).unwrap();
        assert_eq!(read_entry(&dir, "aaaa").unwrap()["adr"], "ADR-A");
        assert_eq!(read_entry(&dir, "bbbb").unwrap()["adr"], "ADR-B");
        let o = others_of(&dir, "aaaa", &live);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].session, "bbbb");
        assert!(!o[0].alive, "pid 2 is not live in this test");
        let o = others_of(&dir, "bbbb", &live);
        assert!(o[0].alive && o[0].adr == "ADR-A" && o[0].gate == "cargo test a", "{:?}", o);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flat_file_from_before_reads_as_the_asking_session_and_migrates_on_write() {
        let dir = project();
        std::fs::write(dir.join(".hexa/loop.json"), r#"{"adr":"ADR-OLD","gate":"cargo test","stage":"done"}"#).unwrap();
        assert_eq!(read_entry(&dir, "aaaa").unwrap()["adr"], "ADR-OLD");
        assert!(others_of(&dir, "aaaa", &live).is_empty());
        update_entry(&dir, "aaaa", 1, serde_json::json!({"stage": "build"})).unwrap();
        let file: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".hexa/loop.json")).unwrap()).unwrap();
        assert_eq!(file["sessions"]["aaaa"]["adr"], "ADR-OLD", "{file}");
        assert_eq!(file["sessions"]["aaaa"]["stage"], "build");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn touched_files_deduplicate_and_are_seen_from_the_other_session_only_while_it_is_live() {
        let dir = project();
        let f = dir.join("src/domain/x.rs").display().to_string();
        touch_as(&dir, "aaaa", 1, &f).unwrap();
        touch_as(&dir, "aaaa", 1, &f).unwrap();
        touch_as(&dir, "aaaa", 1, "tests/y.rs").unwrap();
        let e = read_entry(&dir, "aaaa").unwrap();
        assert_eq!(e["files"], serde_json::json!(["src/domain/x.rs", "tests/y.rs"]), "relative, deduplicated, in order: {e}");
        assert_eq!(touched_by_others_of(&dir, "bbbb", &f, &live).len(), 1, "seen while live");
        assert!(touched_by_others_of(&dir, "bbbb", &f, &|_| false).is_empty(), "not seen once ended");
        assert!(touched_by_others_of(&dir, "aaaa", &f, &live).is_empty(), "never one's own");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_removes_one_entry_and_the_file_only_when_empty() {
        let dir = project();
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A"})).unwrap();
        update_entry(&dir, "bbbb", 1, serde_json::json!({"adr": "ADR-B"})).unwrap();
        assert!(clear_entry(&dir, "aaaa").unwrap());
        assert!(dir.join(".hexa/loop.json").is_file(), "the other entry keeps the file");
        assert!(read_entry(&dir, "aaaa").is_none());
        assert_eq!(read_entry(&dir, "bbbb").unwrap()["adr"], "ADR-B");
        assert!(clear_entry(&dir, "bbbb").unwrap());
        assert!(!dir.join(".hexa/loop.json").exists(), "the last entry takes the file with it");
        assert!(!clear_entry(&dir, "bbbb").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
