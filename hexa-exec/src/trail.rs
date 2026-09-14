//! The decision trail a run leaves behind it (ADR-2609140928).
//!
//! hexa's memory had a hole in the middle. Before a task an ADR records the
//! decision; after it `hexa memory store lesson:` records what was learned.
//! Between them, inside a single build, harden or twelve-step run, the agent
//! made dozens of choices and every one was gone when the process exited.
//!
//! That is fine while a run succeeds. When it fails — or worse, succeeds for
//! the wrong reason — there is nothing to read. The commit shows the end state
//! and the gate shows pass or fail. Neither shows the step where the run
//! turned, and the only recourse is to run it again and watch, which is a
//! different run.
//!
//! Rows are appended as each decision is made, never summarised at the end. A
//! summary written by the model that made the choices is the model's account of
//! its own reasoning — the mirror test applied to a post-mortem. It is not
//! evidence.

use std::path::{Path, PathBuf};

/// One decision.
///
/// Five fields, tab-separated, because they are short and the file is read by
/// both people and `cut`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// When the decision was made, RFC 3339.
    pub at: String,
    /// Which step of the run this was, counting from one.
    pub step: u32,
    /// What was chosen.
    pub chose: String,
    /// What it was chosen over. This is the field that makes it a decision
    /// rather than a log line: a step with no alternative was not a choice.
    pub over: String,
    /// What came back. A row without this documents nothing.
    pub evidence: String,
}

/// Why a row was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Decision 4. A trail whose evidence column is empty would let a run claim
    /// it was traced when it was not — ADR-2609122048, turned on hexa's own
    /// reasoning.
    NoEvidence,
    /// A tab would split one field into two and shift every later column.
    TabInField(&'static str),
    /// A row about nothing.
    NothingChosen,
}

impl Refused {
    pub fn message(&self) -> String {
        match self {
            Refused::NoEvidence => {
                "a row with no evidence documents nothing; do not write it".to_string()
            }
            Refused::TabInField(f) => format!("field `{f}` contains a tab, which would shift every later column"),
            Refused::NothingChosen => "a row must say what was chosen".to_string(),
        }
    }
}

impl Row {
    /// The row as one tab-separated line, or why it will not be written.
    pub fn to_line(&self) -> Result<String, Refused> {
        if self.chose.trim().is_empty() {
            return Err(Refused::NothingChosen);
        }
        if self.evidence.trim().is_empty() {
            return Err(Refused::NoEvidence);
        }
        for (name, v) in
            [("chose", &self.chose), ("over", &self.over), ("evidence", &self.evidence), ("at", &self.at)]
        {
            if v.contains('\t') {
                return Err(Refused::TabInField(name));
            }
        }
        Ok(format!(
            "{}\t{}\t{}\t{}\t{}",
            self.at,
            self.step,
            one_line(&self.chose),
            one_line(&self.over),
            one_line(&self.evidence)
        ))
    }

    /// Parse one line back. `None` when the line is not five fields.
    pub fn from_line(line: &str) -> Option<Row> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 5 {
            return None;
        }
        Some(Row {
            at: f[0].to_string(),
            step: f[1].parse().ok()?,
            chose: f[2].to_string(),
            over: f[3].to_string(),
            evidence: f[4].to_string(),
        })
    }
}

/// Flatten a value onto one line and cap it, so one long tool output cannot
/// make a trail unreadable.
fn one_line(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 240 {
        flat
    } else {
        format!("{}…", flat.chars().take(239).collect::<String>())
    }
}

/// Where a run's trail lives: in the repository, so it diffs and commits.
fn trail_path(repo_root: &Path, run_id: &str) -> PathBuf {
    repo_root.join("docs").join("trails").join(format!("{}.tsv", safe_id(run_id)))
}

/// A run id reduced to something that is safe as a file name.
fn safe_id(run_id: &str) -> String {
    let s: String = run_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if s.is_empty() { "run".to_string() } else { s }
}

/// Append one decision, now.
///
/// Best-effort on I/O: a trail that cannot be written must not take the run
/// down with it. A refused row is different — that is a defect in the caller,
/// and it comes back so the caller can be fixed.
pub fn append(repo_root: &Path, run_id: &str, row: &Row) -> Result<(), Refused> {
    let line = row.to_line()?;
    let path = trail_path(repo_root, run_id);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return Ok(());
        }
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
    Ok(())
}

/// Read one trail back.
pub fn read(repo_root: &Path, run_id: &str) -> Vec<Row> {
    let Ok(text) = std::fs::read_to_string(trail_path(repo_root, run_id)) else {
        return Vec::new();
    };
    text.lines().filter_map(Row::from_line).collect()
}

/// Every run that left a trail, newest first, as `(run_id, rows)`.
pub fn runs(repo_root: &Path) -> Vec<(String, usize)> {
    let dir = repo_root.join("docs").join("trails");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, usize)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "tsv"))
        .map(|e| {
            let id = e.path().file_stem().unwrap_or_default().to_string_lossy().to_string();
            let n = std::fs::read_to_string(e.path())
                .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
                .unwrap_or(0);
            (id, n)
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Row {
        Row {
            at: "2026-09-14T12:00:00Z".into(),
            step: 3,
            chose: "repo_read path=src/lib.rs".into(),
            over: "repo_grep, cargo_check, propose_edit".into(),
            evidence: "ok, 240 lines".into(),
        }
    }

    #[test]
    fn a_row_is_five_tab_separated_fields() {
        let line = row().to_line().expect("a good row");
        assert_eq!(line.split('\t').count(), 5, "{line}");
        assert_eq!(Row::from_line(&line).expect("round trip"), row());
    }

    #[test]
    fn a_row_with_no_evidence_is_refused() {
        let mut r = row();
        r.evidence = "   ".into();
        assert_eq!(r.to_line(), Err(Refused::NoEvidence));
    }

    #[test]
    fn a_row_that_chose_nothing_is_refused() {
        let mut r = row();
        r.chose = String::new();
        assert_eq!(r.to_line(), Err(Refused::NothingChosen));
    }

    #[test]
    fn a_tab_inside_a_field_is_refused() {
        let mut r = row();
        r.chose = "a\tb".into();
        assert_eq!(r.to_line(), Err(Refused::TabInField("chose")));
    }

    #[test]
    fn a_newline_is_flattened_rather_than_refused() {
        // Tool output has newlines in it constantly. Refusing them would mean
        // refusing most real rows, which is how a trail ends up empty.
        let mut r = row();
        r.evidence = "line one\nline two".into();
        let line = r.to_line().expect("flattened, not refused");
        assert!(line.contains("line one line two"), "{line}");
        assert_eq!(line.split('\t').count(), 5);
    }

    #[test]
    fn a_very_long_field_is_capped() {
        let mut r = row();
        r.evidence = "x".repeat(5000);
        let line = r.to_line().expect("capped");
        assert!(line.chars().count() < 400, "one long output made the trail unreadable");
        assert!(line.ends_with('…'));
    }

    #[test]
    fn a_run_id_never_escapes_the_trails_directory() {
        let base = Path::new("/repo");
        let p = trail_path(base, "../../etc/passwd");
        assert!(p.starts_with("/repo/docs/trails"), "{}", p.display());
        assert!(!p.to_string_lossy().contains(".."), "{}", p.display());
    }

    #[test]
    fn appending_and_reading_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        append(dir.path(), "run-1", &row()).expect("append");
        let mut second = row();
        second.step = 4;
        second.chose = "cargo_check crate=hexa-core".into();
        append(dir.path(), "run-1", &second).expect("append");

        let rows = read(dir.path(), "run-1");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].step, 4);
        assert_eq!(runs(dir.path()), vec![("run-1".to_string(), 2)]);
    }

    #[test]
    fn a_refused_row_is_never_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut bad = row();
        bad.evidence = String::new();
        assert!(append(dir.path(), "run-2", &bad).is_err());
        assert!(read(dir.path(), "run-2").is_empty(), "a refused row reached the file");
    }
}
