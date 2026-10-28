//! habit-streak — daily habit check-ins, current and longest streaks.
//!
//! `lib.rs` is the **composition root**: the only place allowed to name a
//! concrete adapter (ADR-2610281300 §6–§7).

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

use std::io::Write;
use std::path::PathBuf;

/// Where the book is kept, and whether that is the default path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPath {
    pub path: PathBuf,
    pub is_default: bool,
}

/// Neither a flag, `HABIT_STREAK_FILE`, `XDG_DATA_HOME` nor `HOME` names a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoDataPath;

pub fn resolve_data_path(flag: Option<PathBuf>, env: &dyn Fn(&str) -> Option<String>) -> Result<DataPath, NoDataPath> {
    if let Some(path) = flag {
        return Ok(DataPath { path, is_default: false });
    }
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    if let Some(p) = set("HABIT_STREAK_FILE") {
        return Ok(DataPath { path: PathBuf::from(p), is_default: false });
    }
    if let Some(x) = set("XDG_DATA_HOME") {
        return Ok(DataPath { path: PathBuf::from(x).join("habit-streak/habits.json"), is_default: true });
    }
    if let Some(h) = set("HOME") {
        return Ok(DataPath { path: PathBuf::from(h).join(".local/share/habit-streak/habits.json"), is_default: true });
    }
    Err(NoDataPath)
}

/// The whole program; `args` are the arguments after the program name.
pub fn run(args: &[String], env: &dyn Fn(&str) -> Option<String>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    use adapters::primary::cli;
    use adapters::secondary::{ChronoCalendar, JsonFileStore};

    let parsed = match cli::parse(args, out, err) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let Ok(data) = resolve_data_path(parsed.file.clone(), env) else {
        let _ = writeln!(
            err,
            "error: cannot choose a data file: neither XDG_DATA_HOME nor HOME is set. Pass --file <path> or set HABIT_STREAK_FILE."
        );
        return 2;
    };
    // A typo in a named path must fail, not read as an empty history
    // (ADR-2610281300 §4). Every command, `add` included: a new file starts
    // at the default path.
    if !data.is_default && !data.path.exists() {
        let _ = writeln!(
            err,
            "error: data file {} does not exist. Check --file and HABIT_STREAK_FILE. Create the file first, or omit --file to use the default path",
            data.path.display()
        );
        return 2;
    }
    let calendar = ChronoCalendar;
    let store = JsonFileStore::new(data.path, data.is_default, &calendar);
    let habits = usecases::Habits::new(&store, &calendar);
    cli::execute(&parsed, &habits, &calendar, out, err)
}
