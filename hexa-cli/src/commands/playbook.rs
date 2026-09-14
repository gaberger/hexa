//! `hexa playbook` — inspect, check and learn playbooks (ADR-2609140929).
//!
//! ADR-2609140844 shipped four playbooks and no way to see them. They could be
//! routed to and not listed, printed, or validated. This closes that, and adds
//! the audit the same ADR asked for: the four were written by hand in one
//! sitting, so their ordering is a guess, and hexa records what actually ran.

use clap::Subcommand;
use colored::Colorize;

use crate::playbook::{self, Playbook};

#[derive(Subcommand, Debug)]
pub enum PlaybookAction {
    /// Every playbook hexa ships
    List,
    /// Print one playbook's steps
    Show {
        /// The playbook name, e.g. bug-fix
        name: String,
    },
    /// Validate a playbook file against the rules the shipped ones are held to
    Check {
        /// Path to a playbook JSON file
        file: String,
    },
    /// Draft a playbook from runs that actually happened
    Learn,
}

/// The rules a playbook must satisfy, in the binary rather than only in a test.
///
/// One definition, two callers: this and
/// `hexa-cli/tests/playbooks_are_executable.rs`, which keeps its own vacuity
/// guards so a hollowed-out validator fails the test rather than passing
/// everything.
pub fn faults(pb: &Playbook) -> Vec<String> {
    let mut out = Vec::new();
    if pb.name.trim().is_empty() {
        out.push("no name".into());
    }
    if pb.summary.trim().is_empty() {
        out.push("no summary".into());
    }
    if pb.triggers.is_empty() {
        out.push("no triggers, so nothing can route here".into());
    }
    if pb.steps.len() < 3 {
        out.push(format!("{} step(s) is not a procedure", pb.steps.len()));
    }
    for (i, s) in pb.steps.iter().enumerate() {
        if s.title.trim().is_empty() {
            out.push(format!("step {}: no title", i + 1));
        }
        if s.run.trim().is_empty() {
            out.push(format!("step {}: nothing to run", i + 1));
        }
        if s.done_when.trim().is_empty() {
            out.push(format!("step {}: no condition that proves it finished", i + 1));
        }
    }
    let proves = pb.steps.iter().any(|s| {
        s.run.starts_with("hexa verify") || s.run.contains("--evidence") || s.run.contains("--gate")
    });
    if !proves {
        out.push("no proof step: no `hexa verify`, no --evidence, no --gate".into());
    }
    match pb.steps.last() {
        Some(last) if last.run.starts_with("hexa analyze") => {}
        Some(last) => out.push(format!("ends on `{}`, not the architecture grade", last.run)),
        None => out.push("no steps at all".into()),
    }
    out
}

/// Below this, a draft is a guess about a guess.
const ENOUGH_RUNS: usize = 5;

pub async fn run(action: PlaybookAction) -> anyhow::Result<()> {
    match action {
        PlaybookAction::List => {
            let books = playbook::load()?;
            println!("{} {} playbook(s)\n", "⬡ playbooks".cyan().bold(), books.len());
            for pb in &books {
                println!("  {:<16} {}", pb.name.bold(), pb.summary.dimmed());
                println!("  {:<16} {} steps · triggers: {}", "", pb.steps.len(), pb.triggers.join(", ").dimmed());
            }
        }
        PlaybookAction::Show { name } => {
            let books = playbook::load()?;
            match books.iter().find(|p| p.name == name) {
                Some(pb) => print!("{}", playbook::render(pb)),
                None => {
                    let names: Vec<&str> = books.iter().map(|p| p.name.as_str()).collect();
                    println!("  {} no playbook called `{}`.", "·".dimmed(), name);
                    println!("    There are: {}", names.join(", "));
                }
            }
        }
        PlaybookAction::Check { file } => {
            let body = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("cannot read {file}: {e}"))?;
            let pb: Playbook = serde_json::from_str(&body)
                .map_err(|e| anyhow::anyhow!("{file} is not a playbook: {e}"))?;
            let faults = faults(&pb);
            if faults.is_empty() {
                println!("  {} {} is a valid playbook", "✓".green().bold(), file);
                return Ok(());
            }
            println!("  {} {} has {} fault(s):", "✗".red().bold(), file, faults.len());
            for f in &faults {
                println!("    {}", f);
            }
            anyhow::bail!("{} did not pass", file);
        }
        PlaybookAction::Learn => learn().await?,
    }
    Ok(())
}

/// Draft a playbook from runs that actually happened.
///
/// It drafts. It never installs. A tool that learns a procedure from history
/// and then silently starts handing it out has closed a loop nobody asked it
/// to close.
async fn learn() -> anyhow::Result<()> {
    let dir = std::env::current_dir()?;
    let runs = hexa_exec::trail::runs(&dir);

    // Decision 7: too little history is a result, not an error.
    if runs.len() < ENOUGH_RUNS {
        println!(
            "  {} {} recorded run(s); {} are needed to draft from.",
            "·".dimmed(),
            runs.len(),
            ENOUGH_RUNS
        );
        println!("    Trails appear under docs/trails/ as agent loops run.");
        return Ok(());
    }

    // What actually happened, in order, across the runs.
    let mut sequence: Vec<String> = Vec::new();
    for (id, _) in runs.iter().take(20) {
        for row in hexa_exec::trail::read(&dir, id) {
            let head = row.chose.split_whitespace().next().unwrap_or("").to_string();
            if !head.is_empty() && sequence.last() != Some(&head) {
                sequence.push(head);
            }
        }
    }

    println!("{} {} run(s) read, {} decision(s) in sequence", "⬡ learn".cyan().bold(), runs.len().min(20), sequence.len());
    println!("  {}", "This is a description of what happened, not a rule about what should.".dimmed());
    println!("\n  The shape of recent runs:");
    for (i, s) in sequence.iter().take(20).enumerate() {
        println!("    {}. {}", i + 1, s);
    }
    println!(
        "\n  {} hexa does not draft a playbook file from this yet.",
        "·".dimmed()
    );
    println!("    A tool-level trail records which tool ran, not which hexa verb.");
    println!("    Turning one into the other is a mapping nobody has earned yet.");
    println!("    Write the playbook by hand and check it with `hexa playbook check`.");
    Ok(())
}
