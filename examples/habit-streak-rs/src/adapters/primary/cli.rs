//! The command line (ADR-2610281300 §7–§8): parse, run one use case, print.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::ports::{
    AddOutcome, CheckOutcome, DateText, HabitCommands, NameError, RemoveOutcome, UncheckOutcome, UseCaseError,
};

#[derive(Parser)]
#[command(name = "habit-streak", about = "Record daily habit check-ins; see current and longest streaks")]
struct Cli {
    /// The data file (default: $XDG_DATA_HOME or ~/.local/share, habit-streak/habits.json)
    #[arg(long, global = true)]
    file: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start tracking a habit
    Add { habit: String },
    /// Record a check-in, today unless --date is given
    Check {
        habit: String,
        #[arg(long)]
        date: Option<String>,
    },
    /// Remove a check-in
    Uncheck {
        habit: String,
        #[arg(long)]
        date: String,
    },
    /// Current and longest streaks, for every habit or the one named
    Status { habit: Option<String> },
    /// Stop tracking a habit and delete its check-ins
    Remove {
        habit: String,
        /// Required: confirms the removal
        #[arg(long)]
        yes: bool,
    },
}

/// A parsed command line: the global `--file`, and the command.
pub struct Parsed {
    pub file: Option<PathBuf>,
    command: Command,
}

/// `Err` holds the exit code and has already printed its message.
pub fn parse(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<Parsed, i32> {
    let argv = std::iter::once("habit-streak".to_string()).chain(args.iter().cloned());
    match Cli::try_parse_from(argv) {
        Ok(cli) => {
            if let Command::Remove { yes: false, .. } = cli.command {
                let _ = writeln!(err, "error: remove deletes a habit and its history; confirm with --yes");
                return Err(1);
            }
            Ok(Parsed { file: cli.file, command: cli.command })
        }
        Err(e) => {
            use clap::error::ErrorKind;
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                let _ = write!(out, "{e}");
                return Err(0);
            }
            let text = e.to_string();
            let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("error: invalid arguments");
            let line = if first.starts_with("error:") { first.to_string() } else { format!("error: {first}") };
            let _ = writeln!(err, "{line} (see habit-streak --help)");
            Err(1)
        }
    }
}

fn name_of(raw: &str) -> String {
    raw.trim().to_string()
}

/// One line, and the exit code: 1 for the person's mistake, 2 for the file's.
fn failure(e: &UseCaseError, dates: &dyn DateText) -> (String, i32) {
    match e {
        UseCaseError::InvalidName(NameError::Empty) => ("error: a habit name cannot be empty".into(), 1),
        UseCaseError::InvalidName(NameError::ControlCharacter) => {
            ("error: a habit name cannot contain control characters".into(), 1)
        }
        UseCaseError::UnknownHabit { name, known } if known.is_empty() => {
            (format!("error: no habit \"{name}\"; there are no habits yet. Add one with: habit-streak add {name}"), 1)
        }
        UseCaseError::UnknownHabit { name, known } => {
            (format!("error: no habit \"{name}\"; known habits: {}", known.join(", ")), 1)
        }
        UseCaseError::FutureDate { day, today } => (
            format!(
                "error: {} is after today ({}); a check-in cannot be in the future",
                dates.format(*day),
                dates.format(*today)
            ),
            1,
        ),
        UseCaseError::Store(s) => (format!("error: {s}. The file was not changed."), 2),
    }
}

fn day_arg(text: &str, dates: &dyn DateText) -> Result<crate::ports::Day, (String, i32)> {
    dates.parse(text).ok_or_else(|| (format!("error: invalid date \"{text}\"; expected YYYY-MM-DD"), 1))
}

/// Run the parsed command; the exit code.
pub fn execute(parsed: &Parsed, habits: &dyn HabitCommands, dates: &dyn DateText, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let result: Result<Vec<String>, (String, i32)> = (|| match &parsed.command {
        Command::Add { habit } => match habits.add(habit).map_err(|e| failure(&e, dates))? {
            AddOutcome::Added { .. } => Ok(vec![format!("added habit \"{}\"", name_of(habit))]),
            AddOutcome::AlreadyExists => Ok(vec![format!("habit \"{}\" already exists; nothing changed", name_of(habit))]),
        },
        Command::Check { habit, date } => {
            let day = date.as_deref().map(|d| day_arg(d, dates)).transpose()?;
            match habits.check(habit, day).map_err(|e| failure(&e, dates))? {
                CheckOutcome::Recorded(d) => Ok(vec![format!("checked in \"{}\" on {}", name_of(habit), dates.format(d))]),
                CheckOutcome::AlreadyRecorded(d) => Ok(vec![format!(
                    "\"{}\" was already checked in on {}; nothing changed",
                    name_of(habit),
                    dates.format(d)
                )]),
            }
        }
        Command::Uncheck { habit, date } => {
            let day = day_arg(date, dates)?;
            match habits.uncheck(habit, day).map_err(|e| failure(&e, dates))? {
                UncheckOutcome::Removed(d) => {
                    Ok(vec![format!("removed the check-in of \"{}\" on {}", name_of(habit), dates.format(d))])
                }
                UncheckOutcome::NotRecorded(d) => Ok(vec![format!(
                    "\"{}\" had no check-in on {}; nothing changed",
                    name_of(habit),
                    dates.format(d)
                )]),
            }
        }
        Command::Remove { habit, .. } => match habits.remove(habit).map_err(|e| failure(&e, dates))? {
            RemoveOutcome::RemovedHabit => Ok(vec![format!("removed habit \"{}\"", name_of(habit))]),
        },
        Command::Status { habit } => {
            let lines = habits.status(habit.as_deref()).map_err(|e| failure(&e, dates))?;
            if lines.is_empty() {
                return Ok(vec!["No habits yet. Add one with: habit-streak add <name>".to_string()]);
            }
            let width = lines.iter().map(|l| l.name.chars().count()).max().unwrap_or(0);
            Ok(lines
                .iter()
                .map(|l| {
                    let pad = " ".repeat(width - l.name.chars().count());
                    let mark = if l.current > 0 && !l.checked_today { " (not checked today)" } else { "" };
                    format!("{}{pad}  current {}  longest {}{mark}", l.name, l.current, l.longest)
                })
                .collect())
        }
    })();
    match result {
        Ok(lines) => {
            // A closed pipe (`status | head -1`) ends the command quietly.
            for l in lines {
                if writeln!(out, "{l}").is_err() {
                    break;
                }
            }
            0
        }
        Err((line, code)) => {
            let _ = writeln!(err, "{line}");
            code
        }
    }
}
