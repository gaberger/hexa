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

/// The recorded loop state, if any.
pub fn read_loop(dir: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(loop_path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Merge `patch` into the loop state and write it. Only in a project that has
/// a `.hexa/` directory; elsewhere there is nothing to record into.
pub fn update_loop(dir: &Path, patch: serde_json::Value) -> Result<serde_json::Value, String> {
    if !dir.join(".hexa").is_dir() {
        return Err(format!("{} has no .hexa/ directory; run `hexa init .` first", dir.display()));
    }
    let mut state = read_loop(dir).unwrap_or_else(|| serde_json::json!({}));
    if let (Some(obj), Some(p)) = (state.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            obj.insert(k.clone(), v.clone());
        }
        obj.insert("updated".to_string(), serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
    }
    let text = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())? + "\n";
    std::fs::write(loop_path(dir), text).map_err(|e| e.to_string())?;
    Ok(state)
}

/// Remove the loop file. Returns whether there was one.
fn clear_loop(dir: &Path) -> Result<bool, String> {
    let p = loop_path(dir);
    if !p.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    Ok(true)
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
            println!("{} {}", "\u{2b21}".yellow(), if was { "loop state cleared" } else { "nothing was recorded" });
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
