//! `hexa loop`: where this project's work stands in the loop.
//!
//! Decide (an ADR) → Gate (the command that must exit 0, written before the
//! code) → Build → Harden. The state is `.hexa/loop.json` in the project,
//! committed with the branch, so it travels with the pull request and a
//! reviewer sees which ADR the work is under and which gate proved it. The
//! ADR is the durable record; the loop file points at it and names the gate.
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
    /// Forget the recorded state
    Clear,
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

/// Does `docs/adrs/` hold an ADR whose file name starts with `id`?
fn adr_exists(dir: &Path, id: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir.join("docs").join("adrs")) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.starts_with(id) && name.ends_with(".md")
    })
}

/// One line for the hooks: the state, or what is missing.
pub fn status_line(dir: &Path) -> String {
    let project = project_name(dir);
    match read_loop(dir) {
        Some(st) => {
            let adr = st.get("adr").and_then(|v| v.as_str()).unwrap_or("none");
            let gate = st.get("gate").and_then(|v| v.as_str()).unwrap_or("none");
            let stage = st.get("stage").and_then(|v| v.as_str()).unwrap_or("decide");
            format!("Loop ({project}): stage {stage} · ADR {adr} · gate {gate}")
        }
        None => format!(
            "Loop ({project}): nothing recorded. Decide → Gate → Build → Harden. Record with `hexa loop adr <ID>` and `hexa loop gate '<command>'`."
        ),
    }
}

const STAGES: &[&str] = &["decide", "gate", "build", "harden", "done"];

pub async fn run(action: Option<LoopAction>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    match action.unwrap_or(LoopAction::Show) {
        LoopAction::Show => {
            println!("{} {}", "\u{2b21}".cyan(), status_line(&cwd));
            if let Some(st) = read_loop(&cwd) {
                if let Some(u) = st.get("updated").and_then(|v| v.as_str()) {
                    println!("  updated {u}");
                }
                println!("  file    {}", loop_path(&cwd).display());
            }
        }
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
        LoopAction::Stage { stage } => {
            let stage = stage.to_lowercase();
            if !STAGES.contains(&stage.as_str()) {
                anyhow::bail!("stage must be one of: {}", STAGES.join(", "));
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
