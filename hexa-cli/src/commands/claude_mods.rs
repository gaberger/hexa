//! Setup's Claude Code mods step (ADR-2610301129 §4): extract the embedded
//! marketplace to `~/.hexa/claude-mods/`, then install its mods through the
//! `ClaudePlugins` port.
//!
//! It returns `()`: nothing here can reach the caller's exit status. Every
//! skip and failure is a printed warning.

use colored::Colorize;
use hexa_core::ports::claude_mods::Step;
use std::path::PathBuf;

/// The mods setup installs, by plugin name.
const MODS: &[&str] = &["question-alert"];

/// Opts a run out, so tests that run setup against a real HOME leave the
/// developer's Claude profile alone.
const OPT_OUT: &str = "HEXA_NO_CLAUDE_MODS";

/// Extract and install hexa's Claude Code mods, printing one line per step.
pub fn install_mods(dry_run: bool) {
    println!("{}", "⬡ Installing Claude Code mods...".cyan());
    if std::env::var_os(OPT_OUT).is_some_and(|v| !v.is_empty()) {
        warn(&format!("Claude Code mods skipped ({OPT_OUT} set)"));
        println!();
        return;
    }
    // The one `~/.hexa` path that ignores HEXA_HOME: the mods are the user's,
    // per machine, and the ADR names `~/.hexa/claude-mods`.
    let Some(dir) = home().map(mods_dir) else {
        warn("Claude Code mods skipped: no home directory");
        println!();
        return;
    };
    if dry_run {
        let ids: Vec<String> = MODS.iter().map(|m| format!("{m}@hexa")).collect();
        println!(
            "  would extract Claude Code mods to {} and install {}",
            dir.display(),
            ids.join(", ")
        );
        println!();
        return;
    }
    // A half-written folder is not registered: worse than none.
    if let Err(e) = crate::assets::Assets::extract_owned("claude-mods/", &dir) {
        warn(&format!(
            "Claude Code mods not extracted to {}: {e}",
            dir.display()
        ));
        println!();
        return;
    }
    let outcomes =
        hexa_core::ports::claude_mods::apply(&*crate::default_claude_plugins(), &dir, MODS);
    if outcomes.is_empty() {
        println!("{}", "✓ Claude Code mods up to date".green());
    }
    for (step, outcome) in &outcomes {
        print_step(step, outcome);
    }
    println!();
}

/// The home directory. A `HOME` set to empty means none: `dirs` would fall
/// back to the password database and write somewhere the caller ruled out.
fn home() -> Option<PathBuf> {
    if std::env::var_os("HOME").is_some_and(|h| h.is_empty()) {
        return None;
    }
    dirs::home_dir().filter(|h| !h.as_os_str().is_empty())
}

fn mods_dir(home: PathBuf) -> PathBuf {
    home.join(".hexa").join("claude-mods")
}

fn print_step(step: &Step, outcome: &Result<(), String>) {
    match (step, outcome) {
        (Step::Skip(reason), _) => warn(&format!("Claude Code mods skipped: {reason}")),
        (Step::AddMarketplace(dir), Ok(())) => {
            println!(
                "{}",
                format!("✓ marketplace hexa registered ({})", dir.display()).green()
            )
        }
        (Step::AddMarketplace(_), Err(e)) => warn(&format!("marketplace hexa not registered: {e}")),
        (Step::Install(id), Ok(())) => {
            println!(
                "{}",
                format!("✓ installed {id} (restart Claude Code to load it)").green()
            )
        }
        (Step::Install(id), Err(e)) => warn(&format!("install {id} failed: {e}")),
    }
}

fn warn(line: &str) {
    println!("{}", format!("⚠ {line}").yellow());
}
