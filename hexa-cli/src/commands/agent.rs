//! `hexa agent register|report|list|release` — the task ledger's verbs
//! (ADR-2611051850, phases 2 and 3).
//!
//! hexa keeps the record and the rules; the harness owns the processes. An
//! agent (or whoever runs it) claims a task and the paths it will edit with
//! `register`, edits inside that lease, and closes the task with `report`.
//! hexa never spawns, supervises or kills an agent.
//!
//! Every verb prints JSON when stdout is not a terminal, and with `--json`.
//! Every refusal is an object with `code`, `message`, `hint`, `field` and
//! `retryable`, and exits 1. Every write takes `--dry-run`: all checks run,
//! nothing is written.

use clap::{Subcommand, ValueEnum};
use hexa_core::ports::ledger::{
    find_overlap, never_leasable, normalize_lease, Ledger, LedgerError, Task, TaskStatus,
};
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::commands::lease_layers::{self, Arch};
use crate::commands::{run_record, task_ledger};

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Killed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Killed => "killed",
        }
    }
}

#[derive(Subcommand)]
pub enum AgentAction {
    /// Claim a task and the paths it may edit; prints the task id. Refuses a lease that overlaps another task's, a sealed path, or a credential
    #[command(after_help = "\
Layer slices: a lease whose first segment is domain, ports, usecases or adapters and that is not \
an existing path resolves through the layer map to the paths it covers; a slice covering no file is refused.
Contracts: a lease on domain or ports files is refused while a live lease covers a file that imports one \
(Rust `crate::` paths, TS/JS relative imports, read from the files now). A use-case or adapter lease \
records the contract files it imports as `frozen`, and waits while another task leases one of them.
Binding: a session is held to its lease when HEXA_TASK=<id> is in its environment, \
or when its Claude Code agent_id equals --owner. An unbound session is not constrained.
Never leasable: any path a docs/specs/*.json lists as sealed, and credential-looking names \
(.runner, .env*, *.pem, *.key, *.p12, id_rsa*, credentials*, .netrc, *token*, *secret*).
Errors are JSON objects with code, message, hint, field, retryable.")]
    Register {
        /// A short name for the task
        #[arg(long)]
        task: String,
        /// A repo-relative file or directory this task may edit, or a layer slice such as
        /// `usecases/billing` or `adapters/secondary/postgres` (repeatable; at least one)
        #[arg(long, num_args = 1.., action = clap::ArgAction::Append, value_name = "PATH")]
        lease: Vec<String>,
        /// The worktree the task works in
        #[arg(long, value_name = "DIR")]
        worktree: Option<String>,
        /// The command that must exit 0 for the task to count as done
        #[arg(long, value_name = "CMD")]
        gate: Option<String>,
        /// The agent label that is bound to this task: a session id, or the Claude Code agent_id
        #[arg(long)]
        owner: Option<String>,
        /// Check everything and write nothing
        #[arg(long)]
        dry_run: bool,
        /// Print JSON (the default when stdout is not a terminal)
        #[arg(long)]
        json: bool,
    },
    /// Close a task with its outcome and commit, and release its leases
    Report {
        /// The task id `register` printed
        #[arg(long)]
        id: String,
        #[arg(long, value_enum)]
        outcome: Outcome,
        /// The commit the task produced
        #[arg(long, value_name = "SHA")]
        commit: Option<String>,
        /// Free text kept on the task
        #[arg(long)]
        note: Option<String>,
        /// Check everything and write nothing
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// The ledger: every task with its leases, status and run record
    List {
        /// Only tasks that still hold their leases (claimed or running)
        #[arg(long)]
        active: bool,
        #[arg(long)]
        json: bool,
    },
    /// Free a claimed task without an outcome (a harness that crashed before it could report)
    Release {
        #[arg(long)]
        id: String,
        /// Check everything and write nothing
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: AgentAction) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    match action {
        AgentAction::Register {
            task,
            lease,
            worktree,
            gate,
            owner,
            dry_run,
            json,
        } => {
            let json = want_json(json);
            let req = Register {
                name: task,
                leases: lease,
                worktree,
                gate,
                owner,
            };
            finish(register(&cwd, &req, dry_run), json)
        }
        AgentAction::Report {
            id,
            outcome,
            commit,
            note,
            dry_run,
            json,
        } => {
            let json = want_json(json);
            finish(
                report(
                    &cwd,
                    &id,
                    outcome,
                    commit.as_deref(),
                    note.as_deref(),
                    dry_run,
                ),
                json,
            )
        }
        AgentAction::List { active, json } => {
            let json = want_json(json);
            finish(list(&cwd, active), json)
        }
        AgentAction::Release { id, dry_run, json } => {
            let json = want_json(json);
            finish(release(&cwd, &id, dry_run), json)
        }
    }
}

fn want_json(flag: bool) -> bool {
    flag || !std::io::stdout().is_terminal()
}

/// Print the result or the refusal; a refusal exits 1 with nothing else on stderr in JSON mode.
fn finish(r: Result<Value, LedgerError>, json: bool) -> anyhow::Result<()> {
    match r {
        Ok(v) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print!("{}", render_text(&v));
            }
            Ok(())
        }
        Err(e) => {
            if json {
                let mut v = serde_json::to_value(&e)?;
                v["ok"] = json!(false);
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                eprintln!("\u{2717} {}\n  hint: {}", e.message, e.hint);
                eprintln!(
                    "  code: {}  field: {}  retryable: {}",
                    e.code,
                    e.field.as_deref().unwrap_or("-"),
                    e.retryable
                );
            }
            crate::commands::events::exit(1);
        }
    }
}

fn render_text(v: &Value) -> String {
    if let Some(tasks) = v.get("tasks").and_then(Value::as_array) {
        let mut s = format!("{} task(s), {} active\n", tasks.len(), v["count"]["active"]);
        for t in tasks {
            s += &format!(
                "  {}  {:<8}  {}  leases: {}\n",
                t["id"].as_str().unwrap_or("?"),
                t["status"].as_str().unwrap_or("?"),
                t["name"].as_str().unwrap_or("?"),
                t["leases"]
                    .as_array()
                    .map(|a| a
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            );
        }
        return s;
    }
    let mut s = String::new();
    if v["dry_run"] == true {
        s += "dry run: nothing written\n";
    }
    s += &serde_json::to_string_pretty(v).unwrap_or_default();
    s + "\n"
}

pub struct Register {
    pub name: String,
    pub leases: Vec<String>,
    pub worktree: Option<String>,
    pub gate: Option<String>,
    pub owner: Option<String>,
}

fn task_json(t: &Task) -> Value {
    json!({
        "id": t.id, "name": t.name, "status": t.status, "owner": t.owner,
        "leases": t.leases, "frozen": t.frozen, "worktree": t.worktree, "gate": t.gate, "run": t.run,
        "outcome": t.outcome, "commit": t.commit, "note": t.note,
        "created": t.created, "updated": t.updated,
    })
}

/// Every check `register` makes, shared by the real call and `--dry-run`.
fn check_register(
    ledger: &Ledger,
    leases: &[String],
    sealed: &[String],
    arch: &Arch,
    frozen: &[String],
) -> Result<(), LedgerError> {
    for l in leases {
        if let Some(e) = never_leasable(l, sealed) {
            return Err(e);
        }
    }
    if let Some((held, want, holder)) = find_overlap(ledger, leases) {
        return Err(LedgerError::new(
            "lease_overlap",
            format!(
                "lease `{want}` overlaps `{held}`, held by task {} ({}, {})",
                holder.id, holder.name, holder.status
            ),
            format!(
                "Choose a path disjoint from `{held}`, or wait until task {} reports (`hexa agent report --id {}`) or is released (`hexa agent release --id {}`).",
                holder.id, holder.id, holder.id
            ),
            Some("lease"),
            true,
        ));
    }
    lease_layers::check_contracts(arch, ledger, leases, frozen)
}

pub fn register(cwd: &Path, req: &Register, dry_run: bool) -> Result<Value, LedgerError> {
    if req.name.trim().is_empty() {
        return Err(LedgerError::new(
            "name_invalid",
            "the task name is empty".into(),
            "Pass a short name: --task <name>.".into(),
            Some("task"),
            false,
        ));
    }
    if req.leases.is_empty() {
        return Err(LedgerError::new(
            "lease_invalid",
            "a task needs at least one lease".into(),
            "Pass the file or directory the task edits: --lease src/a/".into(),
            Some("lease"),
            false,
        ));
    }
    // Slices resolve against the task's worktree when it has one, else the caller's checkout.
    let arch_dir = req
        .worktree
        .as_deref()
        .map(PathBuf::from)
        .filter(|w| w.is_dir())
        .unwrap_or_else(|| cwd.to_path_buf());
    let arch = Arch::load(&arch_dir)?;
    let mut leases: Vec<String> = Vec::new();
    for l in &req.leases {
        for n in arch.resolve(&normalize_lease(l)?)? {
            if !leases.contains(&n) {
                leases.push(n);
            }
        }
    }
    let sealed = task_ledger::sealed(cwd);
    for l in &leases {
        if let Some(e) = never_leasable(l, &sealed) {
            return Err(e);
        }
    }
    let frozen = arch.frozen_for(&leases)?;
    let worktree = req.worktree.as_ref().map(|w| {
        task_ledger::canonical_lenient(&PathBuf::from(w))
            .display()
            .to_string()
    });

    if dry_run {
        let ledger = task_ledger::read(cwd)?;
        check_register(&ledger, &leases, &sealed, &arch, &frozen)?;
        return Ok(json!({
            "ok": true, "dry_run": true, "would_register": {
                "name": req.name, "leases": leases, "frozen": frozen, "owner": req.owner, "worktree": worktree, "gate": req.gate,
            }
        }));
    }

    let id = task_ledger::new_task_id();
    let root = task_ledger::main_root(cwd);
    let task = task_ledger::update(cwd, |ledger| {
        check_register(ledger, &leases, &sealed, &arch, &frozen)?;
        let run =
            run_record::begin_for_task(&id, &req.name, req.gate.as_deref(), worktree.as_deref())
                .map_err(|e| {
                    LedgerError::new(
                        "record_unwritable",
                        format!("the run record could not be written: {e}"),
                        "Check that ~/.hexa/runs is writable (or HEXA_HOME), then register again."
                            .into(),
                        None,
                        true,
                    )
                })?;
        let now = task_ledger::now();
        let task = Task {
            id: id.clone(),
            name: req.name.clone(),
            owner: req.owner.clone(),
            worktree: worktree.clone(),
            gate: req.gate.clone(),
            leases: leases.clone(),
            frozen: frozen.clone(),
            status: TaskStatus::Claimed,
            run: Some(run),
            outcome: None,
            commit: None,
            note: None,
            created: now.clone(),
            updated: now,
        };
        ledger.tasks.push(task.clone());
        Ok(task)
    })?;
    let ignored = task_ledger::ensure_ignored(&root).unwrap_or(false);
    let mut out = task_json(&task);
    out["ok"] = json!(true);
    out["ledger"] = json!(task_ledger::ledger_path(cwd).display().to_string());
    out["git_ignored_by"] = json!(if ignored {
        Some(".hexa/.gitignore")
    } else {
        None
    });
    out["bind"] = json!({
        "env": format!("{}={}", task_ledger::ENV_TASK, task.id),
        "hint": format!(
            "Export HEXA_TASK={} in the session that does this work, or register with --owner <agent_id> so Claude Code's agent_id binds it. Then `hexa agent report --id {} --outcome passed --commit <sha>` when done.",
            task.id, task.id
        ),
    });
    Ok(out)
}

fn not_found(id: &str) -> LedgerError {
    LedgerError::new(
        "task_not_found",
        format!("no task {id} in the ledger"),
        "List the ids with `hexa agent list`.".into(),
        Some("id"),
        false,
    )
}

fn not_active(t: &Task) -> LedgerError {
    LedgerError::new(
        "task_not_active",
        format!("task {} ({}) is already {}", t.id, t.name, t.status),
        "A task is reported or released once; register a new task for more work.".into(),
        Some("id"),
        false,
    )
}

fn valid_commit(c: &str) -> bool {
    (4..=64).contains(&c.len()) && c.chars().all(|x| x.is_ascii_hexdigit())
}

/// What closing a task does to the ledger entry; shared by `report`, `release` and the SubagentStop hook.
pub fn close_task(
    cwd: &Path,
    id: &str,
    status: TaskStatus,
    outcome: Option<&str>,
    commit: Option<&str>,
    note: Option<&str>,
    dry_run: bool,
) -> Result<Value, LedgerError> {
    let apply = |ledger: &mut Ledger| -> Result<Task, LedgerError> {
        let t = ledger.get_mut(id).ok_or_else(|| not_found(id))?;
        if !t.status.is_active() {
            return Err(not_active(t));
        }
        t.status = status;
        t.outcome = outcome.map(str::to_string);
        if commit.is_some() {
            t.commit = commit.map(str::to_string);
        }
        if note.is_some() {
            t.note = note.map(str::to_string);
        }
        t.updated = task_ledger::now();
        Ok(t.clone())
    };
    if dry_run {
        let mut ledger = task_ledger::read(cwd)?;
        let t = apply(&mut ledger)?;
        let mut v = task_json(&t);
        v["ok"] = json!(true);
        v["dry_run"] = json!(true);
        return Ok(v);
    }
    let t = task_ledger::update(cwd, apply)?;
    let mut v = task_json(&t);
    v["ok"] = json!(true);
    let mut warnings: Vec<String> = Vec::new();
    match &t.run {
        Some(run) => {
            let run_outcome = outcome.unwrap_or("abandoned");
            if let Err(e) = run_record::close_task_run(run, run_outcome, commit) {
                warnings.push(format!(
                    "the ledger entry is closed but its run record was not: {e}"
                ));
            }
        }
        None => warnings.push("the task has no run record to close".into()),
    }
    v["warnings"] = json!(warnings);
    Ok(v)
}

pub fn report(
    cwd: &Path,
    id: &str,
    outcome: Outcome,
    commit: Option<&str>,
    note: Option<&str>,
    dry_run: bool,
) -> Result<Value, LedgerError> {
    if let Some(c) = commit {
        if !valid_commit(c) {
            return Err(LedgerError::new(
                "commit_invalid",
                format!("`{c}` is not a commit hash"),
                "Pass the hexadecimal hash of the commit: --commit $(git rev-parse --short HEAD)."
                    .into(),
                Some("commit"),
                false,
            ));
        }
    }
    if outcome == Outcome::Passed {
        let ledger = task_ledger::read(cwd)?;
        let t = ledger.get(id).ok_or_else(|| not_found(id))?;
        if !t.status.is_active() {
            return Err(not_active(t));
        }
        earn_passed(cwd, t, commit)?;
    }
    let status = if outcome == Outcome::Passed {
        TaskStatus::Done
    } else {
        TaskStatus::Failed
    };
    close_task(
        cwd,
        id,
        status,
        Some(outcome.as_str()),
        commit,
        note,
        dry_run,
    )
}

/// A `passed` is earned, not stated: the commit resolves to a commit, and the
/// task's registered gate runs here and passes by the one evidence reader —
/// exit 0 and at least one test, never "running 0 tests". It recorded done on
/// `--commit deadbeef` with no check at all.
fn earn_passed(cwd: &Path, t: &Task, commit: Option<&str>) -> Result<(), LedgerError> {
    let dir = t
        .worktree
        .as_deref()
        .map(PathBuf::from)
        .filter(|w| w.is_dir())
        .unwrap_or_else(|| cwd.to_path_buf());
    if let Some(c) = commit {
        let resolved = std::process::Command::new("git")
            .args(["cat-file", "-e", &format!("{c}^{{commit}}")])
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !resolved {
            return Err(LedgerError::new(
                "commit_not_found",
                format!("`{c}` is not a commit in {}", dir.display()),
                "Pass the commit the task produced: --commit $(git rev-parse --short HEAD), from its worktree."
                    .into(),
                Some("commit"),
                false,
            ));
        }
    }
    let Some(gate) = t.gate.as_deref().filter(|g| !g.trim().is_empty()) else {
        return Err(LedgerError::new(
            "gate_missing",
            format!("task {} has no gate, so nothing can show it passed", t.id),
            "Register the task with --gate '<command that must exit 0>', or report --outcome failed.".into(),
            Some("outcome"),
            false,
        ));
    };
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(format!("set -o pipefail; {gate}"))
        .current_dir(&dir)
        .output();
    let (ok, text) = match out {
        Ok(o) => (
            o.status.success(),
            format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
        ),
        Err(e) => (false, format!("the gate did not start: {e}")),
    };
    let verdict = hexa_exec::direct_exec::evidence_verdict(ok, &text, false);
    if verdict.passes() {
        return Ok(());
    }
    // The shared message ends "or declare the evidence --uncounted", the way a
    // `loop gate` or `do run` user closes a gate that runs no tests. A task's
    // gate has no such form, so that clause is dropped: it named a flag this
    // verb does not have.
    let why = hexa_exec::direct_exec::evidence_failure(gate, &verdict, "")
        .replace("Use a test command, or declare the evidence --uncounted", "Use a test command: a task's gate is judged by its test output, and `agent register` has no uncounted form");
    let tail: Vec<&str> = text.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect();
    Err(LedgerError::new(
        "gate_failed",
        format!("task {}: {}", t.id, why.trim_end_matches(|c: char| c == ':' || c.is_whitespace() || c == '.').trim_end_matches(". Output")),
        format!("The gate ran in {} and did not pass; fix it, or report --outcome failed. Its output ended:\n{}", dir.display(), tail.join("\n")),
        Some("outcome"),
        false,
    ))
}

pub fn release(cwd: &Path, id: &str, dry_run: bool) -> Result<Value, LedgerError> {
    close_task(cwd, id, TaskStatus::Released, None, None, None, dry_run)
}

pub fn list(cwd: &Path, active_only: bool) -> Result<Value, LedgerError> {
    let ledger = task_ledger::read(cwd)?;
    let tasks: Vec<Value> = ledger
        .tasks
        .iter()
        .filter(|t| !active_only || t.status.is_active())
        .map(task_json)
        .collect();
    Ok(json!({
        "ok": true,
        "ledger": task_ledger::ledger_path(cwd).display().to_string(),
        "tasks": tasks,
        "count": { "tasks": ledger.tasks.len(), "active": ledger.active().count() },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status()
            .unwrap()
            .success());
        d
    }

    fn req(name: &str, leases: &[&str]) -> Register {
        Register {
            name: name.into(),
            leases: leases.iter().map(|s| s.to_string()).collect(),
            worktree: None,
            gate: None,
            owner: None,
        }
    }

    // `register` writes a run record under the state dir; a test must not touch the real one.
    // The scope is shared with every other test that reads HEXA_HOME (`run_record::with_env_home`).
    fn with_home<T>(f: impl FnOnce() -> T) -> T {
        let home = tempfile::tempdir().unwrap();
        crate::commands::run_record::with_env_home(Some(home.path()), f)
    }

    #[test]
    fn two_disjoint_tasks_register_and_a_third_overlapping_one_is_refused_with_the_four_fields() {
        let d = repo();
        with_home(|| {
            let a = register(d.path(), &req("one", &["src/a/"]), false).unwrap();
            let b = register(d.path(), &req("two", &["src/b/"]), false).unwrap();
            assert_ne!(a["id"], b["id"]);
            let e = register(d.path(), &req("three", &["src/a/x.rs"]), false).unwrap_err();
            let v = serde_json::to_value(&e).unwrap();
            assert_eq!(v["code"], "lease_overlap");
            assert_eq!(v["field"], "lease");
            assert_eq!(v["retryable"], true);
            assert!(
                v["hint"]
                    .as_str()
                    .unwrap()
                    .contains(a["id"].as_str().unwrap()),
                "the hint names the holder: {v}"
            );
            assert_eq!(
                list(d.path(), false).unwrap()["count"]["active"],
                2,
                "the refused task was not recorded"
            );
        });
    }

    #[test]
    fn a_dry_run_checks_but_writes_nothing() {
        let d = repo();
        with_home(|| {
            let v = register(d.path(), &req("one", &["src/a"]), true).unwrap();
            assert_eq!(v["dry_run"], true);
            assert!(
                !task_ledger::ledger_path(d.path()).exists(),
                "no ledger written"
            );
            register(d.path(), &req("one", &["src/a"]), false).unwrap();
            let e = register(d.path(), &req("again", &["src/a"]), true).unwrap_err();
            assert_eq!(
                e.code, "lease_overlap",
                "a dry run refuses what the real call would"
            );
        });
    }

    /// A commit in `d`, so `passed` has something to resolve.
    fn commit(d: &Path) -> String {
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@x", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(d)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        git(&["commit", "-q", "--allow-empty", "-m", "work"]);
        git(&["rev-parse", "--short", "HEAD"])
    }

    const GREEN_GATE: &str = "echo 'test result: ok. 1 passed; 0 failed; 0 ignored'";

    #[test]
    fn report_closes_the_task_and_its_run_record_and_frees_the_lease() {
        let d = repo();
        let sha = commit(d.path());
        with_home(|| {
            let mut with_gate = req("one", &["src/a"]);
            with_gate.gate = Some(GREEN_GATE.into());
            let a = register(d.path(), &with_gate, false).unwrap();
            let id = a["id"].as_str().unwrap();
            let run = a["run"].as_str().unwrap().to_string();
            let r = report(d.path(), id, Outcome::Passed, Some(&sha), None, false).unwrap();
            assert_eq!(
                (
                    r["status"].as_str(),
                    r["outcome"].as_str(),
                    r["commit"].as_str()
                ),
                (Some("done"), Some("passed"), Some(sha.as_str()))
            );
            assert_eq!(r["warnings"].as_array().unwrap().len(), 0, "{r}");
            let rec: Value = serde_json::from_str(
                &std::fs::read_to_string(run_record::runs_dir().join(format!("{run}.json")))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                (rec["outcome"].as_str(), rec["commit"].as_str()),
                (Some("passed"), Some(sha.as_str()))
            );
            assert!(rec["ended"].is_string() && rec["task"] == id);
            register(d.path(), &req("reuse", &["src/a"]), false).expect("the lease is free again");
            assert_eq!(
                report(d.path(), id, Outcome::Passed, None, None, false)
                    .unwrap_err()
                    .code,
                "task_not_active"
            );
        });
    }

    #[test]
    fn report_refuses_a_missing_task_and_a_bad_commit_naming_the_field() {
        let d = repo();
        with_home(|| {
            let e = report(d.path(), "nope", Outcome::Failed, None, None, false).unwrap_err();
            assert_eq!(
                (e.code.as_str(), e.field.as_deref()),
                ("task_not_found", Some("id"))
            );
            let a = register(d.path(), &req("one", &["src/a"]), false).unwrap();
            let e = report(
                d.path(),
                a["id"].as_str().unwrap(),
                Outcome::Passed,
                Some("not-a-sha"),
                None,
                false,
            )
            .unwrap_err();
            assert_eq!(
                (e.code.as_str(), e.field.as_deref()),
                ("commit_invalid", Some("commit"))
            );
        });
    }

    #[test]
    fn passed_is_refused_for_an_unknown_commit_a_failing_or_vacuous_gate_or_no_gate() {
        let d = repo();
        let sha = commit(d.path());
        with_home(|| {
            let task = |gate: Option<&str>| {
                let mut r = req("one", &["src/a"]);
                r.gate = gate.map(str::to_string);
                register(d.path(), &r, false).unwrap()["id"].as_str().unwrap().to_string()
            };
            let id = task(Some(GREEN_GATE));
            let e = report(d.path(), &id, Outcome::Passed, Some("deadbeef"), None, false).unwrap_err();
            assert_eq!((e.code.as_str(), e.field.as_deref()), ("commit_not_found", Some("commit")));
            release(d.path(), &id, false).unwrap();
            for (gate, code) in [
                (Some("false"), "gate_failed"),
                (Some("echo 'test result: ok. 0 passed; 0 failed; 0 ignored'"), "gate_failed"),
                (None, "gate_missing"),
            ] {
                let id = task(gate);
                let e = report(d.path(), &id, Outcome::Passed, Some(&sha), None, false).unwrap_err();
                assert_eq!(e.code, code, "{gate:?}: {}", e.message);
                assert_eq!(list(d.path(), true).unwrap()["count"]["active"], 1, "the refused task stays open");
                release(d.path(), &id, false).unwrap();
            }
        });
    }

    #[test]
    fn a_refused_gate_never_recommends_a_flag_the_task_ledger_does_not_have() {
        // A gate that exits 0 without test-runner output (`test -f file`) is
        // refused. The shared message tells a `loop gate` user to declare the
        // evidence `--uncounted`; `agent register` and `agent report` have no
        // such flag, so naming it sent a peer looking for it.
        let d = repo();
        let sha = commit(d.path());
        with_home(|| {
            let mut r = req("one", &["src/a"]);
            r.gate = Some("test -f .git/HEAD".into());
            let id = register(d.path(), &r, false).unwrap()["id"].as_str().unwrap().to_string();
            let e = report(d.path(), &id, Outcome::Passed, Some(&sha), None, false).unwrap_err();
            assert_eq!(e.code, "gate_failed", "{}", e.message);
            let said = format!("{} {}", e.message, e.hint);
            assert!(!said.contains("--uncounted"), "names a flag this verb does not have: {said}");
            assert!(said.contains("test command"), "still says what to do: {said}");
        });
    }

    #[test]
    fn a_failed_or_killed_report_is_a_failed_task_and_release_leaves_no_outcome() {
        let d = repo();
        with_home(|| {
            let a = register(d.path(), &req("one", &["src/a"]), false).unwrap();
            let b = register(d.path(), &req("two", &["src/b"]), false).unwrap();
            let r = report(
                d.path(),
                a["id"].as_str().unwrap(),
                Outcome::Killed,
                None,
                None,
                false,
            )
            .unwrap();
            assert_eq!(
                (r["status"].as_str(), r["outcome"].as_str()),
                (Some("failed"), Some("killed"))
            );
            let r = release(d.path(), b["id"].as_str().unwrap(), false).unwrap();
            assert_eq!(
                (r["status"].as_str(), r["outcome"].is_null()),
                (Some("released"), true)
            );
        });
    }

    #[test]
    fn sealed_paths_and_credentials_are_refused_before_the_ledger_is_read() {
        let d = repo();
        std::fs::create_dir_all(d.path().join("docs/specs")).unwrap();
        std::fs::write(
            d.path().join("docs/specs/S.json"),
            r#"{"sealed":{"tests/s.rs":"00"}}"#,
        )
        .unwrap();
        // A seal counts as HEAD has it (main's `seals`: an agent cannot loosen one by editing the
        // spec record in its working tree), so the spec must be committed.
        for args in [&["add", "-A"][..], &["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "seal"][..]] {
            assert!(std::process::Command::new("git").args(args).current_dir(d.path()).status().unwrap().success());
        }
        with_home(|| {
            for l in ["tests/s.rs", ".runner", "a/.env"] {
                assert_eq!(
                    register(d.path(), &req("x", &[l]), false).unwrap_err().code,
                    "lease_not_allowed",
                    "{l}"
                );
            }
            assert_eq!(
                register(d.path(), &req("x", &[]), false).unwrap_err().code,
                "lease_invalid"
            );
        });
    }
}
