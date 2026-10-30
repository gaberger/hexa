//! `hexa gap`: the dead ends hexa knows, how often they are hit, and the pull
//! request that closes one (ADR-2610301500).
//!
//! A guide is printed where a dead end is reached, and each print appends a
//! hit to `~/.hexa/gaps.jsonl`. A dead end with no guide is recorded with
//! `hexa gap add`. Nothing here touches the network or a repository.

use clap::Subcommand;
use colored::Colorize;
use hexa_core::ports::guide::{self, Guide, Hit};
use hexa_exec::ports::GapLog;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub enum GapAction {
    /// Each gap with its hits and the last project it was hit in, the most hit first
    List,
    /// Record a dead end hexa has no guide for: what is inconsistent, the route around it, where the fix belongs
    Add {
        /// kebab-case, e.g. spec-seal-after-rename
        id: String,
        /// What is inconsistent
        #[arg(long)]
        what: String,
        /// One step of the route around it, in order; repeat for each step
        #[arg(long = "around")]
        around: Vec<String>,
        /// Where in hexa the fix belongs: `<path> — <what to change>`; repeat for each
        #[arg(long = "fix")]
        fix: Vec<String>,
    },
    /// Print the pull request that closes a gap: a title line, then the body. Opens nothing
    Pr { id: String },
}

/// Print `g` where its dead end is reached, and record the hit. Recording is
/// best-effort: the dead end's own error is what the caller reports.
pub fn hit(g: &Guide, project_dir: &Path) {
    print!("\n{}", g.render());
    let record = serde_json::json!({
        "id": g.id,
        "project": crate::commands::loop_cmd::project_name(project_dir),
        "at": chrono::Utc::now().to_rfc3339(),
    });
    if let Err(e) = hexa_exec::gap_log().record(&record) {
        eprintln!("  {} the gap was not recorded: {e}", "!".yellow());
    }
}

/// The records in `~/.hexa/gaps.jsonl`: hits, and guides added by hand.
fn read() -> (Vec<Hit>, Vec<Guide>) {
    let str_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let list_of = |v: &serde_json::Value, k: &str| -> Vec<String> {
        v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let mut hits = Vec::new();
    let mut added = Vec::new();
    for r in hexa_exec::gap_log().all() {
        if r.get("kind").and_then(|k| k.as_str()) == Some("guide") {
            match (str_of(&r, "id"), str_of(&r, "what")) {
                (Some(id), Some(what)) => added.push(Guide { id, what, around: list_of(&r, "around"), fix: list_of(&r, "fix") }),
                _ => eprintln!("  {} gaps.jsonl: a guide record without id or what: {r}", "!".yellow()),
            }
            continue;
        }
        match (str_of(&r, "id"), str_of(&r, "project")) {
            (Some(id), Some(project)) => hits.push(Hit { id, project, at: str_of(&r, "at").unwrap_or_default() }),
            _ => eprintln!("  {} gaps.jsonl: a record that is neither a hit nor a guide: {r}", "!".yellow()),
        }
    }
    (hits, added)
}

pub async fn run(action: GapAction) -> anyhow::Result<()> {
    match action {
        GapAction::List => list(),
        GapAction::Add { id, what, around, fix } => add(Guide { id, what, around, fix }),
        GapAction::Pr { id } => pr(&id),
    }
}

fn list() -> anyhow::Result<()> {
    let (hits, added) = read();
    let unlisted: Vec<String> = added.iter().map(|g| g.id.clone()).collect();
    let rows = guide::rank(&hits, &unlisted);
    let what_of = |id: &str| {
        guide::find(id).or_else(|| added.iter().find(|g| g.id == id).cloned()).map(|g| g.what).unwrap_or_default()
    };
    println!("{} the dead ends hexa knows, the most hit first", "\u{2b21} gap".cyan().bold());
    for r in &rows {
        let mark = if r.unlisted { "  unlisted" } else { "" };
        println!("  {}  {} hit(s), last in {}{}", r.id.bold(), r.count, r.last_project, mark.yellow());
        println!("      {}", what_of(&r.id).dimmed());
    }
    for g in guide::guides().iter().filter(|g| !rows.iter().any(|r| r.id == g.id)) {
        println!("  {}  0 hits", g.id.bold());
        println!("      {}", g.what.dimmed());
    }
    for g in added.iter().filter(|g| !rows.iter().any(|r| r.id == g.id)) {
        println!("  {}  0 hits{}", g.id.bold(), "  unlisted".yellow());
        println!("      {}", g.what.dimmed());
    }
    println!("\n  `hexa gap pr <id>` writes the pull request that closes one.");
    Ok(())
}

fn add(g: Guide) -> anyhow::Result<()> {
    g.validate().map_err(|e| anyhow::anyhow!(e))?;
    if guide::find(&g.id).is_some() {
        anyhow::bail!("`{}` is a listed guide already; `hexa gap pr {}` writes its pull request", g.id, g.id);
    }
    let cwd = std::env::current_dir()?;
    let record = serde_json::json!({ "kind": "guide", "id": g.id, "what": g.what, "around": g.around, "fix": g.fix });
    hexa_exec::gap_log().record(&record).map_err(|e| anyhow::anyhow!("not recorded: {e}"))?;
    let hit = serde_json::json!({
        "id": g.id,
        "project": crate::commands::loop_cmd::project_name(&cwd),
        "at": chrono::Utc::now().to_rfc3339(),
    });
    hexa_exec::gap_log().record(&hit).map_err(|e| anyhow::anyhow!("the guide was recorded, its hit was not: {e}"))?;
    println!("  {} recorded {} (unlisted). `hexa gap pr {}` writes the pull request that lists it.", "\u{2713}".green(), g.id, g.id);
    Ok(())
}

fn pr(id: &str) -> anyhow::Result<()> {
    let (hits, added) = read();
    let unlisted: Vec<String> = added.iter().map(|g| g.id.clone()).collect();
    let (g, is_unlisted) = match guide::find(id) {
        Some(g) => (g, false),
        None => match added.into_iter().rev().find(|g| g.id == id) {
            Some(g) => (g, true),
            None => anyhow::bail!("no gap `{id}`: `hexa gap list` shows the ones hexa knows"),
        },
    };
    let rows = guide::rank(&hits, &unlisted);
    print!("{}", guide::pr_text(&g, rows.iter().find(|r| r.id == id), is_unlisted));
    Ok(())
}
