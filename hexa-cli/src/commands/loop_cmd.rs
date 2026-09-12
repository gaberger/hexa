//! `hexa loop`: where this project's work stands in the loop.
//!
//! Decide (an ADR) → Gate (the command that must exit 0, written before the
//! code) → Build → Harden. The state is one entry in hexa memory per project,
//! and the hooks read it: session start prints it, a feature-sized prompt
//! prints it, and an edit in a feature-sized session with no gate recorded is
//! stopped until the gate is written. `hexa do` and `hexa build` record their
//! gate on their own.

use clap::Subcommand;
use colored::Colorize;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum LoopAction {
    /// Show where the work stands (the default)
    Show,
    /// Record the ADR this work is under
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

/// One line for the hooks: the state, or what is missing.
pub fn status_line(project: &str) -> String {
    match hexa_exec::local_store::loop_state(project) {
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
    let project = project_name(&cwd);
    match action.unwrap_or(LoopAction::Show) {
        LoopAction::Show => {
            println!("{} {}", "\u{2b21}".cyan(), status_line(&project));
            if let Some(st) = hexa_exec::local_store::loop_state(&project) {
                if let Some(u) = st.get("updated").and_then(|v| v.as_str()) {
                    println!("  updated {u}");
                }
            }
        }
        LoopAction::Adr { id } => {
            let stage = hexa_exec::local_store::loop_state(&project)
                .and_then(|s| s.get("stage").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_else(|| "decide".to_string());
            hexa_exec::local_store::loop_update(&project, serde_json::json!({ "adr": id, "stage": stage }))
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&project));
        }
        LoopAction::Gate { command } => {
            let command = command.trim().to_string();
            if command.is_empty() {
                anyhow::bail!("a gate is a command; it cannot be empty");
            }
            hexa_exec::local_store::loop_update(&project, serde_json::json!({ "gate": command, "stage": "gate" }))
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&project));
        }
        LoopAction::Stage { stage } => {
            let stage = stage.to_lowercase();
            if !STAGES.contains(&stage.as_str()) {
                anyhow::bail!("stage must be one of: {}", STAGES.join(", "));
            }
            hexa_exec::local_store::loop_update(&project, serde_json::json!({ "stage": stage }))
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".green(), status_line(&project));
        }
        LoopAction::Clear => {
            let was = hexa_exec::local_store::loop_clear(&project).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} {}", "\u{2b21}".yellow(), if was { "loop state cleared" } else { "nothing was recorded" });
        }
    }
    Ok(())
}
