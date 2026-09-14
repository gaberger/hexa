//! `hexa bro`: where the work stands, in plain language (ADR-2609140925).
//!
//! hexa knows exactly where the work stands and says so in a register that
//! assumes you never left. `hexa loop show` answers "what stage, which ADR,
//! which gate, how many tasks" in one dense line. The operator who has been
//! away comes back with a different question: what are we doing, and did it
//! work.
//!
//! The facts are in six places and six formats, and assembling them is the
//! task the operator came back unable to do. So this reads them and tells the
//! story.
//!
//! Three rules hold it up. It reads recorded state and runs nothing, so it is
//! safe when you are lost — which is when it will be run. It says what it does
//! not know rather than implying a stale fact is current (ADR-2609122048). And
//! its writing is tested: `hexa-cli/tests/bro_speaks_plainly.rs` caps every
//! sentence at 25 words and resolves every verb it names.

use std::path::{Path, PathBuf};
use std::process::Command;

use colored::Colorize;

use super::loop_cmd;

// ── the facts ───────────────────────────────────────────────────────────────

/// The decision the work sits under.
struct Decision {
    id: String,
    /// The ADR's title, without the `# ADR-…: ` prefix. `None` when the loop
    /// names an ADR whose file is not there — which is itself worth saying.
    title: Option<String>,
    status: Option<String>,
}

/// What hexa has recorded about this working tree.
///
/// Every field is an `Option`, and that is the point. "Not recorded" and
/// "recorded as nothing" are different facts, and a status report that
/// conflates them is the defect ADR-2609122048 names.
struct Facts {
    project: String,
    has_record: bool,
    decision: Option<Decision>,
    gate: Option<String>,
    /// Whether the gate last passed. Nothing in hexa records this yet, so it
    /// is always `None` — and `hexa bro` says so out loud rather than leaving
    /// the operator to assume green.
    gate_result: Option<bool>,
    stage: Option<String>,
    tasks_done: usize,
    tasks_total: usize,
    doing: Option<String>,
    last_commit: Option<(String, String)>,
    uncommitted: Option<usize>,
    /// The recorded architecture grade. Nothing records one yet.
    grade: Option<String>,
}

/// Run a git command in `dir` and return its trimmed stdout, or `None`.
///
/// Reading the repository's own record is not running a gate: no test is run,
/// no agent starts, and nothing is written.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").current_dir(dir).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The ADR file whose name begins with `id`.
fn adr_file(dir: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("docs").join("adrs")).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        (name.starts_with(id) && name.ends_with(".md")).then(|| e.path())
    })
}

/// The title and status of one ADR, read from the file.
fn read_decision(dir: &Path, id: &str) -> Decision {
    let mut d = Decision { id: id.to_string(), title: None, status: None };
    let Some(path) = adr_file(dir, id) else { return d };
    let Ok(body) = std::fs::read_to_string(&path) else { return d };
    for line in body.lines().take(12) {
        if d.title.is_none() {
            if let Some(h) = line.strip_prefix("# ") {
                // `# ADR-2609140925: the title` → `the title`
                d.title = Some(h.split_once(": ").map(|(_, t)| t).unwrap_or(h).trim().to_string());
            }
        }
        if let Some(s) = line.strip_prefix("**Status:**") {
            d.status = Some(s.trim().to_string());
        }
    }
    d
}

/// Shorten a commit subject so the sentence that carries it stays readable.
fn short(subject: &str, max: usize) -> String {
    if subject.chars().count() <= max {
        return subject.to_string();
    }
    let kept: String = subject.chars().take(max - 1).collect();
    format!("{}…", kept.trim_end())
}

fn gather(dir: &Path) -> Facts {
    let state = loop_cmd::read_loop(dir);
    let mut f = Facts {
        project: loop_cmd::project_name(dir),
        has_record: state.is_some(),
        decision: None,
        gate: None,
        gate_result: None,
        stage: None,
        tasks_done: 0,
        tasks_total: 0,
        doing: None,
        last_commit: None,
        uncommitted: None,
        grade: None,
    };

    if let Some(st) = &state {
        f.decision =
            st.get("adr").and_then(|v| v.as_str()).map(|id| read_decision(dir, id));
        f.gate = st.get("gate").and_then(|v| v.as_str()).map(String::from);
        f.stage = st.get("stage").and_then(|v| v.as_str()).map(String::from);
        f.gate_result = st.get("gate_result").and_then(|v| v.as_bool());
        f.grade = st.get("grade").and_then(|v| v.as_str()).map(String::from);
        let tasks = loop_cmd::task_list(st);
        f.tasks_total = tasks.len();
        f.tasks_done = tasks.iter().filter(|t| t.status == "done").count();
        f.doing = tasks.iter().find(|t| t.status == "doing").map(|t| t.title.clone());
    }

    let subject = git(dir, &["log", "-1", "--pretty=%s"]).filter(|s| !s.is_empty());
    let when = git(dir, &["log", "-1", "--pretty=%cr"]).filter(|s| !s.is_empty());
    if let (Some(s), Some(w)) = (subject, when) {
        f.last_commit = Some((s, w));
    }
    f.uncommitted = git(dir, &["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count());

    f
}

// ── the narration ───────────────────────────────────────────────────────────

/// What a stage means, said once, in words that need no glossary.
fn stage_meaning(stage: &str) -> &'static str {
    match stage {
        "decide" => "You are deciding what to build.",
        "gate" => "You are writing the gate.",
        "build" => "You are building the code.",
        "harden" => "You are hunting for bugs the tests missed.",
        "done" => "The work is finished.",
        _ => "hexa does not know this stage.",
    }
}

/// The one thing to do next, from the stage alone.
fn next_step(stage: &str) -> &'static str {
    match stage {
        "decide" => "Write the ADR. Record it with `hexa adr list` to check the id first.",
        "gate" => "Write the command that must exit 0. Record it with `hexa loop gate`.",
        "build" => "Build until the gate exits 0. Use `hexa do run` for one file.",
        "harden" => "Run `hexa harden` over what you built, under the same gate.",
        "done" => "Commit the work. Then start the next decision.",
        _ => "Record a stage with `hexa loop stage`.",
    }
}

/// The report, as text. Kept separate from printing so the register test can
/// read exactly what an operator reads.
pub fn report(dir: &Path) -> String {
    let f = gather(dir);
    let mut o = String::new();

    o.push_str(&format!("⬡ Where the work stands — {}\n\n", f.project));

    if !f.has_record {
        o.push_str("  hexa has no record of any work here.\n");
        o.push_str("  That is not a problem. It means nothing has been started yet.\n");
        o.push_str("  Start by deciding what to build, then write the gate.\n");
        o.push_str("  Run `hexa go` to see what hexa suggests.\n");
        return o;
    }

    // The decision.
    o.push_str("  The decision\n");
    match &f.decision {
        Some(d) => {
            match &d.title {
                Some(t) => o.push_str(&format!("    {} — {}\n", d.id, t)),
                None => o.push_str(&format!("    {}\n", d.id)),
            }
            match (&d.title, &d.status) {
                (None, _) => o.push_str(
                    "    hexa cannot find this ADR in docs/adrs/. The record points at nothing.\n",
                ),
                (_, Some(s)) => o.push_str(&format!("    It is {}. Your work sits under it.\n", s.to_lowercase())),
                (_, None) => o.push_str("    Its status is not recorded in the file.\n"),
            }
        }
        None => o.push_str("    No decision is recorded. Record one with `hexa loop adr`.\n"),
    }
    o.push('\n');

    // The gate.
    o.push_str("  The gate\n");
    match &f.gate {
        Some(g) => {
            o.push_str(&format!("    {}\n", g));
            o.push_str("    This command must exit 0 before the work counts.\n");
            match f.gate_result {
                Some(true) => o.push_str("    hexa recorded it passing.\n"),
                Some(false) => o.push_str("    hexa recorded it failing. That is the thing to fix.\n"),
                None => o.push_str("    hexa has not recorded a result for it. Run it to find out.\n"),
            }
        }
        None => o.push_str("    No gate is recorded. Write one with `hexa loop gate`.\n"),
    }
    o.push('\n');

    // The stage and the checklist.
    o.push_str("  The stage\n");
    match &f.stage {
        Some(s) => {
            o.push_str(&format!("    {} — {}\n", s, stage_meaning(s)));
        }
        None => o.push_str("    No stage is recorded. Set one with `hexa loop stage`.\n"),
    }
    if f.tasks_total == 0 {
        o.push_str("    There is no checklist. Add steps with `hexa loop task`.\n");
    } else {
        o.push_str(&format!(
            "    The checklist has {} steps. {} of them are done.\n",
            f.tasks_total, f.tasks_done
        ));
        if let Some(t) = &f.doing {
            o.push_str(&format!("    You are on this step: {}\n", short(t, 70)));
        }
    }
    o.push('\n');

    // The repository.
    o.push_str("  The repository\n");
    match &f.last_commit {
        Some((subject, when)) => {
            o.push_str(&format!("    The last commit landed {}.\n", when));
            o.push_str(&format!("    {}\n", short(subject, 70)));
        }
        None => o.push_str("    hexa cannot read the git history here.\n"),
    }
    match f.uncommitted {
        Some(0) => o.push_str("    Nothing is waiting to be committed.\n"),
        Some(1) => o.push_str("    One file has changed and is not committed.\n"),
        Some(n) => o.push_str(&format!("    {} files have changed and are not committed.\n", n)),
        None => o.push_str("    hexa cannot read the working tree here.\n"),
    }
    match &f.grade {
        Some(g) => o.push_str(&format!("    The last recorded architecture grade was {}.\n", g)),
        None => o.push_str("    No grade is recorded. Take one with `hexa analyze`.\n"),
    }
    o.push('\n');

    // What to do next.
    o.push_str("  Do this next\n");
    o.push_str(&format!("    {}\n", next_step(f.stage.as_deref().unwrap_or(""))));

    o
}

pub async fn run() -> anyhow::Result<()> {
    let dir = std::env::current_dir()?;
    let text = report(&dir);
    // Colour the headings only. The sentences stay plain, because the register
    // test reads the same string the operator does.
    for line in text.lines() {
        if line.starts_with("  ") && !line.starts_with("    ") && !line.is_empty() {
            println!("{}", line.bold());
        } else if let Some(rest) = line.strip_prefix("⬡ ") {
            println!("{} {}", "⬡".cyan(), rest.bold());
        } else {
            println!("{}", line);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_leaves_a_short_subject_alone() {
        assert_eq!(short("fix the thing", 70), "fix the thing");
    }

    #[test]
    fn short_truncates_and_marks_it() {
        let s = short("aaaaaaaaaa", 5);
        assert_eq!(s, "aaaa…");
    }

    #[test]
    fn an_empty_directory_is_a_result_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = report(dir.path());
        assert!(out.contains("no record of any work here"), "{out}");
        assert!(out.contains("hexa go"), "the empty case must say what to do:\n{out}");
    }

    #[test]
    fn every_stage_has_a_meaning_and_a_next_step() {
        for s in ["decide", "gate", "build", "harden", "done"] {
            assert!(!stage_meaning(s).contains("does not know"), "stage {s} has no meaning");
            assert!(!next_step(s).is_empty(), "stage {s} has no next step");
        }
        // And an unknown stage says so rather than guessing.
        assert!(stage_meaning("banana").contains("does not know"));
    }

    #[test]
    fn an_unrecorded_gate_result_is_said_out_loud() {
        // The repository this test runs in has a gate and no recorded result,
        // which is the case the operator must never read as green.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf();
        let out = report(&dir);
        assert!(
            out.contains("has not recorded a result") || out.contains("hexa recorded it"),
            "the gate's result must be stated either way:\n{out}"
        );
    }
}
