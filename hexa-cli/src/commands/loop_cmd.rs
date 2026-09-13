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

/// When the process `pid` started, in clock ticks since boot: field 22 of
/// /proc/{pid}/stat, after the parenthesised command name. `None` when
/// there is no such process (or no /proc). A pid is recycled; a pid with
/// its start time is not, so the pair names one process.
pub fn pid_start(pid: u64) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19)?.parse().ok()
}

/// A session is live while the process it recorded exists: the pid, and
/// the start time it was recorded with. A session that ended without a
/// clear leaves its pid behind, and the kernel hands that pid to whatever
/// starts next; the start time tells that process from the session's. A
/// pid of 0 is unknown and counts as ended. An entry from before the start
/// time was recorded is judged on the pid alone.
pub fn pid_alive(pid: u64, start: Option<u64>) -> bool {
    if pid == 0 {
        return false;
    }
    match (pid_start(pid), start) {
        (None, _) => false,
        (Some(now), Some(then)) => now == then,
        (Some(_), None) => true,
    }
}

/// The whole file: `{"sessions": {id: entry}}`, or a flat entry from before
/// ADR-2609131408. `Ok(None)` when there is no file (or an empty one, which
/// holds no session to lose); `Err` when there is a file that cannot be read
/// or parsed. A writer must not mistake the second for the first: treating
/// a file it cannot read as "no sessions" and writing back only its own
/// entry is how every other session's ADR, gate, stage and tasks vanish.
fn read_file_strict(dir: &Path) -> Result<Option<serde_json::Value>, String> {
    let p = loop_path(dir);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", p.display())),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{} is not valid JSON ({e}); leaving it as it is", p.display()))
}

/// The whole file for a reader, which has nothing to lose by treating an
/// unreadable file as absent.
fn read_file(dir: &Path) -> Option<serde_json::Value> {
    read_file_strict(dir).ok().flatten()
}

/// The exclusive hold a writer keeps for its read-modify-write, released when
/// the handle drops. Advisory, on a sibling lock file rather than on
/// `loop.json` itself, because `loop.json` is replaced by rename on every
/// write and removed when the last session clears: a lock on it would name
/// an inode the next writer never opens. Every session's hooks and harness
/// write this file, so two writers without the hold lose each other's updates.
fn lock_for_write(dir: &Path) -> Result<std::fs::File, String> {
    let p = dir.join(".hexa").join("loop.lock");
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&p)
        .map_err(|e| format!("cannot open {}: {e}", p.display()))?;
    f.lock().map_err(|e| format!("cannot lock {}: {e}", p.display()))?;
    Ok(f)
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
    // Written whole and renamed into place, so a reader never sees the file
    // truncated or half-written between two sessions' writes.
    let tmp = p.with_file_name(format!("loop.json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("cannot replace {}: {e}", p.display()))
}

/// Apply `edit` to one session's entry and write the file, the read, the
/// edit and the write all under the writer's hold, so what `edit` sees is
/// what is on disk when its result lands. Only in a project that has a
/// `.hexa/` directory; elsewhere there is nothing to record into.
fn edit_entry(
    dir: &Path,
    session: &str,
    pid: u64,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> Result<serde_json::Value, String> {
    if !dir.join(".hexa").is_dir() {
        return Err(format!("{} has no .hexa/ directory; run `hexa init .` first", dir.display()));
    }
    let _hold = lock_for_write(dir)?;
    let mut m = read_file_strict(dir)?.map(|f| entries(&f, session)).unwrap_or_default();
    let mut state = m.get(session).cloned().unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = state.as_object_mut() {
        edit(obj);
        obj.insert("pid".to_string(), serde_json::json!(pid));
        obj.insert("pid_start".to_string(), serde_json::json!(pid_start(pid)));
        obj.insert("updated".to_string(), serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
    }
    m.insert(session.to_string(), state.clone());
    write_entries(dir, m)?;
    Ok(state)
}

/// Merge `patch` into one session's entry and write the file.
pub fn update_entry(dir: &Path, session: &str, pid: u64, patch: serde_json::Value) -> Result<serde_json::Value, String> {
    edit_entry(dir, session, pid, |obj| {
        if let Some(p) = patch.as_object() {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
    })
}

/// Merge `patch` into this session's entry and write it.
pub fn update_loop(dir: &Path, patch: serde_json::Value) -> Result<serde_json::Value, String> {
    update_entry(dir, &session_id(), session_pid(), patch)
}

/// Remove one session's entry; the file goes with the last one. Returns
/// whether there was an entry.
pub fn clear_entry(dir: &Path, session: &str) -> Result<bool, String> {
    if !dir.join(".hexa").is_dir() {
        return Ok(false);
    }
    let _hold = lock_for_write(dir)?;
    let Some(f) = read_file_strict(dir)? else { return Ok(false) };
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
    /// The run in flight, as `running_line` renders it (ADR-2609131611).
    pub running: Option<String>,
}

/// Every session but `session`, liveness judged by `alive` from the pid and
/// the start time it was recorded with.
pub fn others_of(dir: &Path, session: &str, alive: &dyn Fn(u64, Option<u64>) -> bool) -> Vec<Other> {
    let Some(f) = read_file(dir) else { return Vec::new() };
    let mut out: Vec<Other> = entries(&f, session)
        .iter()
        .filter(|(id, _)| id.as_str() != session)
        .map(|(id, e)| Other {
            session: id.clone(),
            alive: alive(e.get("pid").and_then(|v| v.as_u64()).unwrap_or(0), e.get("pid_start").and_then(|v| v.as_u64())),
            adr: e.get("adr").and_then(|v| v.as_str()).unwrap_or("none").to_string(),
            gate: e.get("gate").and_then(|v| v.as_str()).unwrap_or("none").to_string(),
            stage: e.get("stage").and_then(|v| v.as_str()).unwrap_or("decide").to_string(),
            files: e
                .get("files")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            updated: e.get("updated").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            running: running_line(e),
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
/// to see. Deduplicated, most recent last, capped. The list is read and
/// extended under the writer's hold: a host runs independent edits at once,
/// so one session's hooks touch concurrently, and a list built from a read
/// taken before the hold would put back a list missing the other's file.
pub fn touch_as(dir: &Path, session: &str, pid: u64, path: &str) -> Result<(), String> {
    let rel = Path::new(path).strip_prefix(dir).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string());
    edit_entry(dir, session, pid, |obj| {
        let mut files: Vec<String> = obj
            .get("files")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        files.retain(|f| f != &rel);
        files.push(rel);
        if files.len() > TOUCHED_CAP {
            files.drain(..files.len() - TOUCHED_CAP);
        }
        obj.insert("files".to_string(), serde_json::json!(files));
    })
    .map(|_| ())
}

pub fn touch(dir: &Path, path: &str) -> Result<(), String> {
    touch_as(dir, &session_id(), session_pid(), path)
}

/// The live other sessions that have touched `path`.
pub fn touched_by_others_of(dir: &Path, session: &str, path: &str, alive: &dyn Fn(u64, Option<u64>) -> bool) -> Vec<Other> {
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
            // The run is the thing that is happening, so it leads
            // (ADR-2609131611 §1).
            let run = o.running.as_ref().map(|r| format!(" · {r}")).unwrap_or_default();
            if o.alive {
                format!("also here: session {}{} · stage {} · ADR {} · gate {}{}", short(&o.session), run, o.stage, o.adr, o.gate, files)
            } else {
                format!("ended: session {}{} · stage {} · ADR {}{}", short(&o.session), run, o.stage, o.adr, files)
            }
        })
        .collect()
}

/// The one ADR file in `docs/adrs/` named `<id>.md` or `<id>-<slug>.md`.
/// A shortened id that is only a prefix of a real one matches nothing, and
/// so does an id that names more than one file: `read_dir` order is
/// unspecified, and evidence must not land in whichever came first.
fn adr_path(dir: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() {
        return None;
    }
    let entries = std::fs::read_dir(dir.join("docs").join("adrs")).ok()?;
    let exact = format!("{id}.md");
    let slugged = format!("{id}-");
    let mut matches = entries.flatten().map(|e| e.path()).filter(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        name == exact || (name.starts_with(&slugged) && name.ends_with(".md"))
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

/// Does `docs/adrs/` hold exactly one ADR with the id `id`?
fn adr_exists(dir: &Path, id: &str) -> bool {
    adr_path(dir, id).is_some()
}

/// Run the evidence command and append its stdout to the ADR under
/// `## Evidence`, with the command, the commit and the time. A failing
/// command appends nothing and is an error: evidence comes from a run that
/// succeeded (ADR-2609131341).
///
/// The ADR is shared by every session in the checkout, so its read, edit
/// and write happen under the writer's hold, taken only once the command
/// has finished: a command runs for minutes, and a hold across it would
/// stall every other session's hooks for as long. Without the hold, two
/// sessions marking done under one ADR each read the text before the
/// other's block and the second write puts back a file without the first.
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
    let _hold = lock_for_write(dir)?;
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
    let block = format!(
        "\n`{}` at {} on {}:\n\n```text\n{}\n```\n",
        command,
        commit,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
        stdout
    );
    let heading = "\n## Evidence\n";
    match text.find(heading) {
        None => {
            text.push_str(heading);
            text.push_str(&block);
        }
        Some(start) => match next_section_after(&text, start + heading.len()) {
            None => text.push_str(&block),
            Some(mut at) => {
                // Keep the blank line that separates the sections on the
                // heading's side of the new block.
                if text[..at].ends_with("\n\n") {
                    at -= 1;
                    text.insert_str(at, &block);
                } else {
                    text.insert_str(at, &format!("{block}\n"));
                }
            }
        },
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The byte offset of the first `## ` heading line at or after `from`, or
/// `None` when the section runs to the end of the file. Lines inside a
/// fenced code block do not count: evidence is verbatim stdout, and stdout
/// may itself begin a line with `## `.
fn next_section_after(text: &str, from: usize) -> Option<usize> {
    let mut in_fence = false;
    let mut pos = from;
    for line in text[from..].split_inclusive('\n') {
        if line.starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence && line.starts_with("## ") {
            return Some(pos);
        }
        pos += line.len();
    }
    None
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
            if let Some(r) = running_line(&st) {
                line.push_str(&format!(" · {r}"));
            }
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

/// `running harden/verify 4m12s: 3 claims, default refute` while a harness
/// run is in flight in this session (ADR-2609131427); nothing otherwise.
pub fn running_line(state: &serde_json::Value) -> Option<String> {
    running_line_at(state, chrono::Utc::now())
}

/// Two heartbeats of `hexa_exec::adversarial::HEARTBEAT`. Every phase writes
/// at least once a heartbeat, so a record older than this is not being
/// written by anything: the process is gone, or wedged below its own
/// reporting. Either way it is not news, and reporting it as live on a
/// number that nothing increments is worse than saying nothing
/// (ADR-2609131611 §2).
const STALE_AFTER_SECS: i64 = 60;

/// `running harden/verify 4m12s: 3 claims` while a run reports; once it
/// stops, `stalled harden/verify, last seen 14:29`; nothing when cleared.
pub fn running_line_at(state: &serde_json::Value, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let r = state.get("running")?.as_object()?;
    let get = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("?");
    let at = |k: &str| {
        r.get(k)
            .and_then(|v| v.as_str())
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
    };
    let where_ = format!("{}/{}", get("verb"), get("phase"));
    if let Some(updated) = at("updated") {
        if (now - updated).num_seconds() > STALE_AFTER_SECS {
            return Some(format!("stalled {}, last seen {}", where_, updated.format("%H:%M")));
        }
    }
    let elapsed = at("started")
        .map(|t| {
            let secs = (now - t).num_seconds().max(0);
            format!("{}m{:02}s", secs / 60, secs % 60)
        })
        .unwrap_or_else(|| "?".to_string());
    Some(format!("running {} {}: {}", where_, elapsed, get("message")))
}

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
    use super::{adr_exists, lock_for_write, record_evidence};

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

    /// Evidence is filed under `## Evidence` even when another section
    /// follows it. Without the fix the block landed at the end of the file,
    /// under whichever heading came last, and the heading-exists check kept
    /// a fresh `## Evidence` from being added, so nothing said so.
    #[test]
    fn evidence_lands_under_its_heading_not_under_the_section_that_follows_it() {
        let dir = project();
        let path = dir.join("docs/adrs/ADR-1-a-decision.md");
        std::fs::write(
            &path,
            "# ADR-1: a decision\n\n## Evidence\n\n`echo first` at no commit on 2026-09-13 00:00 UTC:\n\n```text\nfirst\n## not a heading\n```\n\n## Consequences\n\nNone yet.\n",
        )
        .unwrap();

        record_evidence(&dir, "ADR-1", "echo second").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let evidence = text.find("\n## Evidence\n").unwrap();
        let second = text.find("```text\nsecond\n```").unwrap();
        let consequences = text.find("\n## Consequences\n").unwrap();
        assert!(evidence < second && second < consequences, "evidence filed under the wrong section:\n{text}");
        assert_eq!(text.matches("## Evidence").count(), 1, "{text}");
        assert!(text.contains("```text\nfirst\n## not a heading\n```\n"), "earlier block left intact: {text}");
        assert!(text.contains("```\n\n## Consequences\n\nNone yet.\n"), "the following section is untouched and still last: {text}");
        assert!(text.ends_with("None yet.\n"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two sessions in one checkout mark done under the same ADR. The ADR is
    /// shared, not per-session, so the second to write must see the first's
    /// block. Here session A has read the ADR under the writer's hold and is
    /// about to write; session B's command finishes meanwhile. Without the
    /// fix B read the text before A's block and wrote the ADR back without
    /// it. With it, B waits for the hold, reads what A landed, and both
    /// blocks are there.
    #[test]
    fn two_sessions_marking_done_under_one_adr_keep_both_blocks_of_evidence() {
        let dir = project();
        let path = dir.join("docs/adrs/ADR-1-a-decision.md");

        // Session A: read under the hold, about to write.
        let hold = lock_for_write(&dir).unwrap();
        let read_by_a = std::fs::read_to_string(&path).unwrap();

        // Session B: its command finishes and it records.
        let b = {
            let dir = dir.clone();
            std::thread::spawn(move || record_evidence(&dir, "ADR-1", "echo from-b"))
        };
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), read_by_a, "B wrote the ADR while A held the writer's hold");

        // A's write lands, then its hold goes.
        std::fs::write(&path, format!("{read_by_a}\n## Evidence\n\n`echo from-a` at no commit on 2026-09-13 00:00 UTC:\n\n```text\nfrom-a\n```\n")).unwrap();
        drop(hold);

        b.join().unwrap().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("```text\nfrom-a\n```"), "A's evidence was discarded by B's write:\n{text}");
        assert!(text.contains("```text\nfrom-b\n```"), "B's evidence is missing:\n{text}");
        assert_eq!(text.matches("## Evidence").count(), 1, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An id must name one ADR exactly. A prefix of a real id
    /// (`ADR-2609131` against `-1341`, `-1408`, `-1427`) is rejected rather
    /// than resolved to whichever file `read_dir` yields first, and so is an
    /// id that two files carry. The full id still resolves, with or without
    /// a slug.
    #[test]
    fn adr_id_must_match_exactly_one_file_not_a_prefix_of_several() {
        let dir = project();
        let adrs = dir.join("docs/adrs");
        for name in ["ADR-2609131341-evidence.md", "ADR-2609131408-sessions.md", "ADR-2609131427-harness.md"] {
            std::fs::write(adrs.join(name), format!("# {name}\n")).unwrap();
        }
        let before: Vec<String> = ["ADR-2609131341-evidence.md", "ADR-2609131408-sessions.md", "ADR-2609131427-harness.md"]
            .iter()
            .map(|n| std::fs::read_to_string(adrs.join(n)).unwrap())
            .collect();

        assert!(!adr_exists(&dir, "ADR-2609131"), "a prefix of three ids is not an ADR");
        assert!(!adr_exists(&dir, "ADR-26091314"), "a prefix of two ids is not an ADR");
        assert!(!adr_exists(&dir, "ADR-260913140"), "a prefix of one id is still not that id");
        assert!(!adr_exists(&dir, ""), "the empty id names nothing");
        let err = record_evidence(&dir, "ADR-2609131", "echo stray").unwrap_err();
        assert!(err.contains("ADR-2609131"), "{err}");
        let after: Vec<String> = ["ADR-2609131341-evidence.md", "ADR-2609131408-sessions.md", "ADR-2609131427-harness.md"]
            .iter()
            .map(|n| std::fs::read_to_string(adrs.join(n)).unwrap())
            .collect();
        assert_eq!(before, after, "an ambiguous id mutates no ADR");

        assert!(adr_exists(&dir, "ADR-2609131408"), "the full id resolves");
        let path = record_evidence(&dir, "ADR-2609131408", "echo exact").unwrap();
        assert!(path.ends_with("ADR-2609131408-sessions.md"), "{}", path.display());
        assert!(!std::fs::read_to_string(adrs.join("ADR-2609131341-evidence.md")).unwrap().contains("exact"));

        std::fs::write(adrs.join("ADR-7.md"), "# ADR-7\n").unwrap();
        assert!(adr_exists(&dir, "ADR-7"), "an id without a slug resolves");

        std::fs::write(adrs.join("ADR-7-second-copy.md"), "# ADR-7 again\n").unwrap();
        assert!(!adr_exists(&dir, "ADR-7"), "an id carried by two files is ambiguous");
        assert!(record_evidence(&dir, "ADR-7", "true").is_err(), "and records nowhere");
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

    fn live(pid: u64, _start: Option<u64>) -> bool {
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

    /// While a harness runs, the loop says which verb and phase, for how long,
    /// and what it last reported; when the run ends the line goes.
    #[test]
    fn the_status_line_shows_what_is_running_and_for_how_long() {
        let started = (chrono::Utc::now() - chrono::Duration::seconds(272)).to_rfc3339();
        let st = serde_json::json!({ "running": { "verb": "harden", "phase": "verify", "message": "3 claims, default refute", "started": started } });
        let line = running_line(&st).unwrap();
        assert!(line.starts_with("running harden/verify 4m32s: 3 claims") || line.starts_with("running harden/verify 4m33s: 3 claims"), "{line}");
        assert!(running_line(&serde_json::json!({ "running": null })).is_none());
        assert!(running_line(&serde_json::json!({ "adr": "ADR-1" })).is_none());
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
        assert!(touched_by_others_of(&dir, "bbbb", &f, &|_, _| false).is_empty(), "not seen once ended");
        assert!(touched_by_others_of(&dir, "aaaa", &f, &live).is_empty(), "never one's own");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pid is recycled. A session that ended without a clear leaves its
    /// entry with its pid; once the kernel gives that pid to another process
    /// the entry must not count as live, and its file list must not warn
    /// anyone. The start time recorded with the pid tells the two apart.
    #[test]
    fn a_reused_pid_does_not_make_an_ended_session_live() {
        let dir = project();
        let me = u64::from(std::process::id());
        let f = dir.join("src/domain/x.rs").display().to_string();

        // The session that is still here: its pid, with its start time.
        touch_as(&dir, "live", me, &f).unwrap();
        let e = read_entry(&dir, "live").unwrap();
        assert_eq!(e["pid_start"], serde_json::json!(pid_start(me)), "the start time is recorded with the pid: {e}");

        // The session that ended: the same pid, recorded with a start time
        // that is not this process's.
        let stale_start = pid_start(me).unwrap() + 1;
        let mut m = entries(&read_file(&dir).unwrap(), "gone");
        m.insert("gone".to_string(), serde_json::json!({"adr": "ADR-GONE", "pid": me, "pid_start": stale_start, "files": ["src/domain/x.rs"]}));
        write_entries(&dir, m).unwrap();

        let by_id: std::collections::HashMap<String, bool> =
            others_of(&dir, "bbbb", &pid_alive).into_iter().map(|o| (o.session, o.alive)).collect();
        assert_eq!(by_id["live"], true, "the session whose process this is");
        assert_eq!(by_id["gone"], false, "pid {me} was recycled; the session that recorded it has ended");
        let warned: Vec<String> = touched_by_others_of(&dir, "bbbb", &f, &pid_alive).into_iter().map(|o| o.session).collect();
        assert_eq!(warned, vec!["live".to_string()], "only the live session's files warn");

        assert!(!pid_alive(0, None));
        assert!(pid_alive(me, None), "an entry from before the start time was recorded is judged on the pid");
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

    /// A file that does not parse, as one looks mid-write or after a hand
    /// edit went wrong, is not "no sessions". A writer that read it as such
    /// would put back a file holding only itself, and every other session's
    /// ADR, gate, stage and tasks would be gone. It refuses instead, and the
    /// file is as it was.
    #[test]
    fn a_file_that_does_not_parse_is_refused_not_replaced_with_one_session() {
        let dir = project();
        let path = dir.join(".hexa/loop.json");
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A", "gate": "cargo test a", "stage": "build"})).unwrap();
        let whole = std::fs::read_to_string(&path).unwrap();
        let half = &whole[..whole.len() / 2];
        std::fs::write(&path, half).unwrap();

        let err = update_entry(&dir, "bbbb", 2, serde_json::json!({"adr": "ADR-B"})).unwrap_err();
        assert!(err.contains("loop.json"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), half, "the file is as it was");
        assert!(touch_as(&dir, "bbbb", 2, "src/x.rs").is_err(), "the hook's touch is the same write");
        assert!(clear_entry(&dir, "bbbb").is_err(), "so is a clear");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), half);

        std::fs::write(&path, &whole).unwrap();
        update_entry(&dir, "bbbb", 2, serde_json::json!({"adr": "ADR-B"})).unwrap();
        assert_eq!(read_entry(&dir, "aaaa").unwrap()["adr"], "ADR-A", "once the file is whole again, aaaa is still in it");
        assert_eq!(read_entry(&dir, "bbbb").unwrap()["adr"], "ADR-B");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A write replaces the file whole rather than truncating it in place.
    /// Truncate-then-write leaves a window in which a reader gets an empty
    /// or partial file; a fresh file renamed over the old one never does.
    /// The inode is the tell: in place keeps it, replacement changes it, and
    /// no stray temp file is left beside it.
    #[test]
    fn a_write_replaces_the_file_rather_than_truncating_it_in_place() {
        use std::os::unix::fs::MetadataExt;
        let dir = project();
        let path = dir.join(".hexa/loop.json");
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A"})).unwrap();
        let before = std::fs::metadata(&path).unwrap().ino();
        update_entry(&dir, "bbbb", 2, serde_json::json!({"adr": "ADR-B"})).unwrap();
        let after = std::fs::metadata(&path).unwrap().ino();
        assert_ne!(before, after, "loop.json was rewritten in place, so a reader can see it truncated");
        let stray: Vec<String> = std::fs::read_dir(dir.join(".hexa"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("loop.json."))
            .collect();
        assert!(stray.is_empty(), "temp file left beside loop.json: {stray:?}");
        assert_eq!(read_entry(&dir, "aaaa").unwrap()["adr"], "ADR-A");
        assert_eq!(read_entry(&dir, "bbbb").unwrap()["adr"], "ADR-B");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sessions write at once: each session's hook on every edit, and a
    /// harness reporting progress. Nothing is lost between them: every
    /// session's entry is there at the end with every file it touched, and no
    /// reader ever sees a file it cannot parse.
    #[test]
    fn sessions_writing_at_once_lose_no_entries_and_no_touched_files() {
        let dir = project();
        let sessions = 8;
        let rounds = 50;
        let handles: Vec<_> = (0..sessions)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let s = format!("s{i}");
                    update_entry(&dir, &s, 1, serde_json::json!({"adr": format!("ADR-{i}"), "gate": "cargo test"})).unwrap();
                    for r in 0..rounds {
                        touch_as(&dir, &s, 1, &format!("src/{i}/{r}.rs")).unwrap();
                        let text = std::fs::read_to_string(dir.join(".hexa/loop.json")).unwrap();
                        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok(), "a reader saw a partial file: {text:?}");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        for i in 0..sessions {
            let s = format!("s{i}");
            let e = read_entry(&dir, &s).unwrap_or_else(|| panic!("session {s} was dropped by another session's write"));
            assert_eq!(e["adr"], format!("ADR-{i}"), "{e}");
            assert_eq!(e["gate"], "cargo test", "{e}");
            let files: Vec<&str> = e["files"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
            assert_eq!(files.len(), TOUCHED_CAP, "{s} lost touched files: {files:?}");
            assert_eq!(files.last().copied(), Some(format!("src/{i}/{}.rs", rounds - 1).as_str()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One session's hooks touch at once: a host runs independent edits in
    /// parallel, and each edit's post-edit hook is its own process. Every
    /// touched file is in the list at the end. Without the list being built
    /// under the hold, two touches read the same list and the second write
    /// puts back a list without the first's file.
    #[test]
    fn one_sessions_parallel_touches_lose_no_files() {
        let dir = project();
        let hooks = 4;
        let per_hook = TOUCHED_CAP / hooks;
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A", "gate": "cargo test a"})).unwrap();
        let go = std::sync::Arc::new(std::sync::Barrier::new(hooks));
        let handles: Vec<_> = (0..hooks)
            .map(|h| {
                let dir = dir.clone();
                let go = go.clone();
                std::thread::spawn(move || {
                    go.wait();
                    for n in 0..per_hook {
                        touch_as(&dir, "aaaa", 1, &format!("src/{h}/{n}.rs")).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let e = read_entry(&dir, "aaaa").unwrap();
        assert_eq!(e["adr"], "ADR-A", "{e}");
        let mut files: Vec<&str> = e["files"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        files.sort_unstable();
        let mut expected: Vec<String> = (0..hooks).flat_map(|h| (0..per_hook).map(move |n| format!("src/{h}/{n}.rs"))).collect();
        expected.sort_unstable();
        assert_eq!(files, expected, "a parallel touch lost a file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A clear reads the map, drops its own entry and, when nothing is left,
    /// removes the file. Another session's first write can land between that
    /// read and the removal, and the removal then takes the newcomer's entry
    /// with it, with no error on either side. So the clear takes the writer's
    /// hold for the whole of read, drop and remove: while another session is
    /// mid-write the clear waits, and what it removes is what is there once
    /// that write has landed.
    #[test]
    fn a_clear_waits_for_a_writer_mid_write_and_does_not_take_its_entry_with_the_file() {
        let dir = project();
        let path = dir.join(".hexa/loop.json");
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A"})).unwrap();

        // Session bbbb has read the file under its hold and is about to
        // write its first entry into it.
        let hold = lock_for_write(&dir).unwrap();
        let cleared = {
            let dir = dir.clone();
            std::thread::spawn(move || clear_entry(&dir, "aaaa"))
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(path.is_file(), "the clear removed the file while another session held the writer's hold");
        assert_eq!(read_entry(&dir, "aaaa").unwrap()["adr"], "ADR-A", "the clear wrote while another session held the writer's hold");

        // bbbb's write lands, then its hold goes.
        let mut m = entries(&read_file(&dir).unwrap(), "bbbb");
        m.insert("bbbb".to_string(), serde_json::json!({"adr": "ADR-B", "pid": 1}));
        write_entries(&dir, m).unwrap();
        drop(hold);

        assert!(cleared.join().unwrap().unwrap(), "aaaa had an entry to clear");
        assert!(path.is_file(), "bbbb's entry was taken with the file");
        assert!(read_entry(&dir, "aaaa").is_none());
        assert_eq!(read_entry(&dir, "bbbb").unwrap()["adr"], "ADR-B", "bbbb's entry is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod visible_run {
    use super::*;

    fn at(mins: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::minutes(mins)).to_rfc3339()
    }

    fn project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hexa-visible-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".hexa")).unwrap();
        dir
    }

    /// ADR-2609131611 §1: another session's run is on its awareness line,
    /// ahead of its ADR, so a second terminal sees what is happening.
    #[test]
    fn another_sessions_run_is_on_its_awareness_line_first() {
        let dir = project();
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A", "gate": "cargo test", "stage": "harden"})).unwrap();
        update_entry(
            &dir,
            "aaaa",
            1,
            serde_json::json!({"running": {"verb": "harden", "phase": "fix", "message": "fixing: the lock", "started": at(4), "updated": at(0)}}),
        )
        .unwrap();
        let o = others_of(&dir, "bbbb", &|_, _| true);
        let line = format!(
            "also here: session {}{} · stage {} · ADR {} · gate {}",
            &o[0].session[..4.min(o[0].session.len())],
            o[0].running.as_ref().map(|r| format!(" · {r}")).unwrap_or_default(),
            o[0].stage,
            o[0].adr,
            o[0].gate
        );
        assert!(line.contains("running harden/fix 4m"), "{line}");
        assert!(line.find("running").unwrap() < line.find("ADR-A").unwrap(), "the run leads: {line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR-2609131611 §2: a record nothing is writing any more is stalled,
    /// with when it was last seen — never live on a climbing number.
    #[test]
    fn a_record_that_stopped_reporting_is_stalled_not_running() {
        let now = chrono::Utc::now();
        let running = |updated_mins_ago: i64| {
            serde_json::json!({"running": {"verb": "harden", "phase": "fix", "message": "fixing: the lock",
                "started": (now - chrono::Duration::minutes(20)).to_rfc3339(),
                "updated": (now - chrono::Duration::minutes(updated_mins_ago)).to_rfc3339()}})
        };
        let fresh = running_line_at(&running(0), now).unwrap();
        assert!(fresh.starts_with("running harden/fix 20m"), "{fresh}");
        let stale = running_line_at(&running(30), now).unwrap();
        assert!(stale.starts_with("stalled harden/fix, last seen "), "{stale}");
        assert!(!stale.contains("20m"), "a stalled run does not report elapsed time: {stale}");
        // Exactly at the bound is still running; past it is not.
        let edge = running_line_at(
            &serde_json::json!({"running": {"verb": "v", "phase": "p", "message": "m",
                "started": (now - chrono::Duration::seconds(90)).to_rfc3339(),
                "updated": (now - chrono::Duration::seconds(STALE_AFTER_SECS)).to_rfc3339()}}),
            now,
        )
        .unwrap();
        assert!(edge.starts_with("running "), "{edge}");
        assert!(running_line_at(&serde_json::json!({"running": null}), now).is_none());
        assert!(running_line_at(&serde_json::json!({"adr": "ADR-A"}), now).is_none());
    }

    /// A cleared run is on neither line.
    #[test]
    fn a_cleared_run_shows_on_neither_line() {
        let dir = project();
        update_entry(&dir, "aaaa", 1, serde_json::json!({"adr": "ADR-A", "running": {"verb": "harden", "phase": "fix", "message": "m", "started": at(1), "updated": at(0)}})).unwrap();
        assert!(others_of(&dir, "bbbb", &|_, _| true)[0].running.is_some());
        update_entry(&dir, "aaaa", 1, serde_json::json!({"running": serde_json::Value::Null})).unwrap();
        assert!(others_of(&dir, "bbbb", &|_, _| true)[0].running.is_none());
        assert!(!awareness_lines(&dir).iter().any(|l| l.contains("running")), "{:?}", awareness_lines(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
