//! `hexa trail` — read a run's decisions back (ADR-2609140928).
//!
//! A trail nothing can read is a file, not a record.

use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand, Debug)]
pub enum TrailAction {
    /// Every run that left a trail, newest first
    List,
    /// One run's decisions, in the order they were made
    Show {
        /// The run id, as `hexa trail list` prints it
        run_id: String,
    },
}

pub async fn run(action: TrailAction) -> anyhow::Result<()> {
    let dir = std::env::current_dir()?;
    match action {
        TrailAction::List => {
            let runs = hexa_exec::trail::runs(&dir);
            if runs.is_empty() {
                // Absence is a result. No run has been traced here yet, which
                // is different from something being wrong.
                println!("  {} no run has left a trail here yet.", "·".dimmed());
                println!("    One appears under docs/trails/ the next time an agent loop runs.");
                return Ok(());
            }
            println!("{} {} run(s)\n", "⬡ trails".cyan().bold(), runs.len());
            for (id, rows) in runs {
                println!("  {:<28} {} decision(s)", id, rows);
            }
        }
        TrailAction::Show { run_id } => {
            let rows = hexa_exec::trail::read(&dir, &run_id);
            if rows.is_empty() {
                println!("  {} no trail for `{}`.", "·".dimmed(), run_id);
                println!("    `hexa trail list` shows the runs that have one.");
                return Ok(());
            }
            println!("{} {} — {} decision(s)\n", "⬡ trail".cyan().bold(), run_id, rows.len());
            for r in rows {
                println!("  {} {}", format!("[{}]", r.step).dimmed(), r.chose.bold());
                if !r.over.is_empty() {
                    println!("      over: {}", r.over.dimmed());
                }
                println!("      →     {}", r.evidence);
            }
        }
    }
    Ok(())
}
