//! `hexa do` — drive the direct executor (ADR-2026-06-04-1740 Path A) from the
//! terminal. The new doing-path: task → one agent → evidence-gated edit → commit.
//! No SOP/persona pipeline. Backed by POST /api/direct/execute + GET /api/direct/runs.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;


#[derive(Subcommand)]
pub enum DoAction {
    /// Run one evidence-gated task: edit a file until the evidence command exits 0, then commit.
    #[command(after_help = "Environment: CLAUDE_TIMEOUT_SECS — seconds a claude-* delegate may run before it is stopped and its files restored (default 300).")]
    Run {
        /// What to do, in plain language.
        instruction: String,
        /// Repo-relative file to edit.
        #[arg(short, long)]
        file: String,
        /// Shell command that must exit 0 (e.g. "cargo test -p hexa-nexus --lib my_test").
        #[arg(short, long)]
        evidence: String,
        /// Reasoning model override.
        #[arg(short, long)]
        model: Option<String>,
        /// Max edit→verify attempts in --fast mode (default 3).
        #[arg(short, long)]
        attempts: Option<u32>,
        /// Use the single-shot path (read → one edit → evidence) instead of the
        /// default multi-step ReAct tool-use loop (ADR-2606071XXX).
        #[arg(long)]
        fast: bool,
        /// Max ReAct loop steps before giving up (default 12). Ignored with --fast.
        #[arg(long)]
        max_steps: Option<u32>,
        /// An additional repo-relative file the run may create or edit besides --file
        /// (repeatable; may not exist yet). Each that exists after a passing run is
        /// committed in the same commit as --file; on a failed run they are restored
        /// (created ones deleted). No other new file is ever committed. Any
        /// other file a claude-* delegate changes is restored before the gate
        /// in an isolated run, and left (and named) on the operator's tree.
        #[arg(long = "also", value_name = "ALSO")]
        also: Vec<String>,
        /// The evidence runs no test runner (a build, a lint): its exit status
        /// alone decides, and the report and commit say `uncounted`
        /// (ADR-2611081151 §6). Without it, output no runner wrote fails.
        #[arg(long)]
        uncounted: bool,
        /// Commit the tracked files you had already modified in the target's
        /// package together with this change. Without it, such files refuse the
        /// run before any change is made: name the ones the change needs with
        /// --also, and commit or stash the rest (ADR-2611160730 §2).
        #[arg(long = "include-dirty")]
        include_dirty: bool,
        /// Print one JSON result object on stdout at exit, and every other line on stderr:
        /// {"ok", "commit", "attempts", "code", "message"} (ADR-2610281500 §9).
        /// Exit status 0 when ok, 1 when not, 64 for code "config".
        #[arg(long)]
        json: bool,
    },
    /// Apply a unified diff with no model, gated like `run`: commit on evidence
    /// exit 0, restore the tree otherwise (ADR-2610281700).
    Apply {
        /// What the change does; the commit subject is `feat(apply): <description>`.
        description: String,
        /// Repo-relative file the diff may touch.
        #[arg(short, long)]
        file: String,
        /// Another file the diff may create, edit or delete (repeatable).
        #[arg(long = "also", value_name = "ALSO")]
        also: Vec<String>,
        /// Shell command that must exit 0 for the change to be committed.
        #[arg(short, long)]
        evidence: String,
        /// The diff to apply; read from stdin when absent.
        #[arg(long)]
        patch: Option<String>,
        /// Print one JSON result object and nothing else.
        #[arg(long)]
        json: bool,
    },
    /// Gate the change already in your tree and commit ONLY the named files (ADR-2611160730 §3).
    /// No model is called: use it for a patch you wrote yourself. A failing or vacuous gate
    /// commits nothing and leaves the change where it is. Under test.remote the gate runs on
    /// the remote runner for a pushed candidate commit. The last stdout line is one JSON
    /// result object: ok, commit, remote_head, tests, and on failure code, hint, field,
    /// retryable (ADR-2611231400).
    #[command(after_help = "Examples:\n  hexa do gate --files tests/t.rs -e 'cargo test --test t' -m 'test: first' --expect-red\n  hexa do gate --files src/a.rs -e 'cargo test --test t' -m 'feat: a'\n  hexa do gate --files src/a.rs -e 'cargo test --test t' -m 'feat: a' --dry-run")]
    Gate {
        /// The files the change is in (repeatable, or several after one --files). Only these are committed.
        #[arg(long = "files", value_name = "PATH", num_args = 1.., required = true)]
        files: Vec<String>,
        /// Shell command that must pass (a test runner that reports at least one test, or any
        /// command with --uncounted).
        #[arg(short, long)]
        evidence: String,
        /// The commit message.
        #[arg(short, long)]
        message: String,
        /// The evidence runs no test runner: its exit status alone decides, and the commit says so.
        #[arg(long)]
        uncounted: bool,
        /// Keep the commit on its branch: do not land it on the base (ADR-2611161800 §4)
        #[arg(long)]
        no_land: bool,
        /// Gate here even when .hexa/project.json sets test.remote; without it
        /// the gate runs on the remote runner for a pushed candidate commit,
        /// and the branch moves to it only on a pass (ADR-2611221610 §7)
        #[arg(long)]
        local: bool,
        /// A tests-first commit: commit the files only if the gate FAILS on test
        /// assertions, with at least one test run. A compile error, zero tests or
        /// a pass is refused. Never lands; prints the `hexa spec red` to run next
        /// (ADR-2611231400 §4, §5)
        #[arg(long, conflicts_with = "uncounted")]
        expect_red: bool,
        /// Say what would be committed, pushed and run, after the same checks,
        /// and do none of it (ADR-2611231400 §6)
        #[arg(long)]
        dry_run: bool,
    },
    /// List recent direct runs (task, evidence verdict, commit) in this repository.
    Runs {
        /// List every repository's runs, and runs logged before runs named theirs.
        #[arg(long)]
        all: bool,
    },
}

/// Where a failed `do run` stopped, read from its result: `commit` when the
/// evidence passed; `evidence` when the evidence ran and failed; otherwise the
/// stage before it — `parse` (a reply that held no usable edit), `inference`
/// (no model, or no reply), or `setup` (the run never reached the model).
///
/// The evidence ran exactly when it left output; a failing command with no
/// output at all is read from the error, which then names an evidence failure.
fn failure_stage(error: &str, evidence_passed: bool, evidence_output: &str) -> &'static str {
    if evidence_passed {
        return "commit";
    }
    if !evidence_output.is_empty() {
        return "evidence";
    }
    // The last attempt's reason is what stopped the run.
    let last = error.rsplit("last: ").next().unwrap_or(error);
    const PARSE: [&str; 7] = [
        "no fenced code block",
        "fenced blocks",
        "non-empty old snippet",
        "empty replacement",
        "apply:",
        "loop ended with no edit",
        "propose_edit",
    ];
    if PARSE.iter().any(|m| last.contains(m)) {
        return "parse";
    }
    if last.starts_with("no model configured") || last.contains("inference:") || last.contains("claude spawn failed") {
        return "inference";
    }
    if last.contains("evidence") && !last.contains("before") {
        return "evidence";
    }
    "setup"
}

/// `f`, given from `cwd`, as a path relative to the repository `root`, or `None`
/// when it ends up outside it. `.` and `..` are resolved first, and symlinks in
/// the part of the path that exists (macOS reaches `/var` through `/private/var`),
/// so `../../docs/a.md` from a subdirectory and an absolute path both work.
fn repo_relative(root: &std::path::Path, cwd: &std::path::Path, f: &str) -> Option<String> {
    use std::path::{Component, Path, PathBuf};
    let joined = if Path::new(f).is_absolute() { PathBuf::from(f) } else { cwd.join(f) };
    let mut lexical = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    // Resolve symlinks through the deepest ancestor that exists; keep the rest as written.
    let mut existing = lexical.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(n) => tail.push(n.to_os_string()),
            None => break,
        }
        if !existing.pop() {
            break;
        }
    }
    let mut real = existing.canonicalize().unwrap_or(existing);
    for n in tail.into_iter().rev() {
        real.push(n);
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = real.strip_prefix(&root).ok()?;
    let s = rel.to_string_lossy().into_owned();
    (!s.is_empty()).then_some(s)
}

pub async fn run(action: DoAction) -> anyhow::Result<()> {
    // No daemon. `hexa do` used to POST the whole task to /api/direct/execute and require nexus to
    // be up first — for a route that was a one-line passthrough to `execute_direct`, in a crate
    // hexa-cli already depends on. The control plane was relaying a call to a library sitting in
    // the same binary.

    match action {
        DoAction::Run { instruction, file, evidence, model, attempts, fast, max_steps, also, uncounted, include_dirty, json: as_json } => {
            // Managed mode (ADR-2610281500 §2): a bad setting stops here, before any model
            // call, lock, worktree or write.
            let managed = if std::env::var("HEXA_MANAGED").as_deref() == Ok("1") {
                match hexa_infer::managed::active_or_error() {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("hexa do run: managed mode: {e}");
                        if as_json {
                            emit_result(false, None, 0, Some("config"), &format!("managed mode: {e}"));
                        }
                        std::process::exit(64);
                    }
                }
            } else {
                None
            };
            if let Err(e) = hexa_exec::direct_exec::validate_also_paths(&also) {
                if as_json {
                    eprintln!("{e}");
                    emit_result(false, None, 0, Some("internal"), &e.to_string());
                    std::process::exit(1);
                }
                anyhow::bail!("{e}");
            }
            // With --json, stdout carries only the result object; every human line goes to stderr.
            let say = |line: String| {
                if as_json {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            };
            // Past the managed deadline (§8): stop before anything is called, run or written.
            if let Some(e) = hexa_exec::direct_exec::managed_timeout() {
                if as_json {
                    eprintln!("{e}");
                    emit_result(false, None, 0, Some("timeout"), &e);
                    std::process::exit(1);
                }
                anyhow::bail!("{e}");
            }
            // Interactive operator run: commit on the operator's own branch
            // (ADR-2606071323 scopes `hexa do` out of worktree isolation — the
            // human owns their tree). Unset/autonomous callers isolate by default.
            // The evidence command becomes the loop's gate only if it has none
            // (ADR-2611020815): side work must not replace the work's gate.
            // A managed run records it under HEXA_STATE_DIR, never in the repository (ADR-2610281500 §6).
            if let Some(m) = &managed {
                let _ = crate::commands::loop_cmd::record_run_gate(&m.state_dir, &evidence, "build", None);
            } else {
                crate::commands::loop_cmd::note_run_gate(&evidence, "build", None);
                if uncounted {
                    crate::commands::loop_cmd::note_gate_uncounted(&evidence);
                }
            }
            let mut body = json!({ "instruction": instruction, "file": file, "evidence": evidence, "fast": fast, "isolate": false, "also": also, "uncounted": uncounted, "include_dirty": include_dirty });
            if let Some(m) = model {
                body["model"] = json!(m);
            }
            if let Some(a) = attempts {
                body["max_attempts"] = json!(a);
            }
            if let Some(s) = max_steps {
                body["max_steps"] = json!(s);
            }
            let mode = if fast { "single-shot" } else { "react loop" };
            say(format!("{} {} {}", "⬡ direct:".cyan().bold(), instruction, format!("[{}]", mode).dimmed()));
            say(format!("  {} {}  {} {}", "file".dimmed(), file, "evidence".dimmed(), evidence));
            if uncounted {
                say(format!("  {} evidence uncounted: its exit status alone decides (ADR-2611081151 §6)", "!".yellow()));
            }
            if !also.is_empty() {
                say(format!("  {} {}", "also".dimmed(), also.join(", ")));
            }

            let task: hexa_exec::direct_exec::DirectTask = serde_json::from_value(body)?;
            let result = hexa_exec::execute_direct(task).await;
            let code = result.code();
            let r = serde_json::to_value(&result)?;

            let ok = r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let ev = r.get("evidence_passed").and_then(|v| v.as_bool()).unwrap_or(false);
            let attempts_n = r.get("attempts").and_then(|v| v.as_u64()).unwrap_or(0);
            let committed = r.get("committed").and_then(|v| v.as_str());
            let step_word = if fast { "attempt(s)" } else { "step(s)" };
            let ev_label = match (ev, uncounted) {
                (true, true) => "pass (uncounted)".green(),
                (true, false) => "pass".green(),
                (false, _) => "fail".red(),
            };

            // Files the run changed but was not given (parley K17): name them
            // whatever the outcome, so they are never left without a word.
            let names = |key: &str| -> Vec<String> {
                r.get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default()
            };
            let restored = names("restored_unlisted");
            if !restored.is_empty() {
                say(format!(
                    "  {} restored {} file(s) the run was not given: {}",
                    "!".yellow().bold(),
                    restored.len(),
                    restored.join(", ")
                ));
            }
            let left = names("left_modified");
            if !left.is_empty() {
                say(format!(
                    "  {} {} file(s) changed during the run that it was not given were left as they are (operator tree): {}",
                    "!".yellow().bold(),
                    left.len(),
                    left.join(", ")
                ));
            }

            if ok {
                say(format!(
                    "{} evidence {} · {} {} · commit {}",
                    "✓ done".green().bold(),
                    ev_label,
                    attempts_n,
                    step_word,
                    committed.unwrap_or("—").yellow()
                ));
                if as_json {
                    let head = std::process::Command::new("git")
                        .args(["rev-parse", "HEAD"])
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
                    emit_result(true, head.as_deref(), attempts_n, None, "evidence passed; committed");
                }
            } else {
                let err = r.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
                let output = r.get("evidence_output").and_then(|v| v.as_str()).unwrap_or("");
                let stage = failure_stage(err, ev, output);
                // Only a run whose evidence ran has an evidence verdict to show.
                let verdict = match stage {
                    "evidence" | "commit" => format!("evidence {ev_label}"),
                    _ => "evidence not run".to_string(),
                };
                say(format!(
                    "{} {} · {} {}\n  {}",
                    format!("✗ failed at {stage}").red().bold(),
                    verdict,
                    attempts_n,
                    step_word,
                    err.dimmed()
                ));
                if let Some(out) = r.get("evidence_output").and_then(|v| v.as_str()) {
                    let tail: Vec<&str> = out.lines().rev().take(8).collect();
                    for line in tail.into_iter().rev() {
                        say(format!("  {}", line.dimmed()));
                    }
                }
                // Name the half that actually failed. This said "did not pass
                // evidence" for every failure, including a run whose evidence
                // passed and whose commit did not — which is the common case in
                // a fresh clone with no git identity.
                let evidence_passed = r
                    .get("evidence_passed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if as_json {
                    emit_result(false, None, attempts_n, code, err);
                    std::process::exit(1);
                }
                if evidence_passed {
                    anyhow::bail!("evidence passed; the run did not complete (see above)");
                }
                match stage {
                    "evidence" => anyhow::bail!("direct run did not pass evidence"),
                    other => anyhow::bail!("direct run failed at {other}, before its evidence ran"),
                }
            }
        }
        DoAction::Gate { files, evidence, message, uncounted, no_land, local, expect_red, dry_run } => {
            gate(files, evidence, message, uncounted, no_land, local, expect_red, dry_run).await?;
        }
        DoAction::Apply { description, file, also, evidence, patch, json: as_json } => {
            let patch = match patch {
                Some(p) => std::fs::read_to_string(&p).map_err(|e| anyhow::anyhow!("--patch {p}: {e}")),
                None => {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
                        .map(|_| s)
                        .map_err(|e| anyhow::anyhow!("reading the diff from stdin: {e}"))
                }
            };
            let o = match patch {
                Ok(patch) => {
                    let root = std::env::current_dir()?;
                    hexa_exec::direct_apply::apply(&root, &description, &file, &also, &evidence, &patch).await
                }
                Err(e) => hexa_exec::direct_apply::ApplyOutcome {
                    ok: false,
                    commit: None,
                    code: Some("config"),
                    message: e.to_string(),
                    exit: 2,
                    evidence_output: String::new(),
                },
            };
            if as_json {
                println!(
                    "{}",
                    json!({ "ok": o.ok, "commit": o.commit, "attempts": 1, "code": o.code, "message": o.message })
                );
            } else if o.ok {
                println!("{} {} · commit {}", "✓ applied".green().bold(), description, o.commit.as_deref().unwrap_or("—").yellow());
            } else {
                for line in o.evidence_output.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev() {
                    eprintln!("  {}", line.dimmed());
                }
                eprintln!("{} {}", "✗".red().bold(), o.message);
            }
            if o.exit != 0 {
                if as_json {
                    // The message is already in the JSON; keep stderr for humans.
                    eprintln!("{}", o.message);
                }
                std::process::exit(o.exit);
            }
        }
        DoAction::Runs { all } => {
            let here = if all {
                None
            } else {
                let root = std::process::Command::new("git")
                    .args(["rev-parse", "--show-toplevel"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()))
                    .unwrap_or(std::env::current_dir()?);
                Some(hexa_exec::direct_exec::repo_key(&root))
            };
            let (runs, hidden) = hexa_exec::direct_exec::runs_for_repo(&hexa_exec::run_log(), here.as_deref());
            let r = json!({
                "summary": hexa_exec::direct_exec::summary_of(&runs),
                "runs": runs,
            });
            let s = &r["summary"];
            let pass_pct = (s["pass_rate"].as_f64().unwrap_or(0.0) * 100.0).trunc() as u32;
            println!(
                "{}  {} runs · {} passed · {} failed · {} committed · {}% pass",
                "⬡ Direct Runs".cyan().bold(),
                s["total"],
                s["passed"].to_string().green(),
                s["failed"],
                s["committed"].to_string().yellow(),
                pass_pct
            );
            if let Some(runs) = r["runs"].as_array() {
                if runs.is_empty() {
                    println!("  {}", "no runs here yet — `hexa do run …` to start".dimmed());
                }
                for run in runs.iter().take(30) {
                    let ev = run["evidence_passed"].as_bool().unwrap_or(false);
                    let mark = if ev { "✓".green() } else { "✗".red() };
                    let commit = run["committed"].as_str().unwrap_or("—");
                    let file = run["file"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("");
                    let instr: String = run["instruction"].as_str().unwrap_or("").chars().take(64).collect();
                    println!("  {} {:<9} {:<18} {}", mark, commit.yellow(), file.dimmed(), instr);
                }
            }
            if hidden > 0 {
                println!(
                    "  {}",
                    format!("{hidden} run(s) of other repositories (or logged before runs named theirs) not shown; --all lists them")
                        .dimmed()
                );
            }
        }
    }
    Ok(())
}

/// The result object `hexa do gate` ends with (ADR-2611231400 §1): printed as
/// the last line of stdout.
fn print_result(v: &serde_json::Value) {
    println!("{}", serde_json::to_string(v).unwrap_or_else(|_| "{\"ok\":false}".to_string()));
}

fn tests_json(t: Option<hexa_core::ports::evidence::TestCounts>, uncounted: bool) -> serde_json::Value {
    match t {
        _ if uncounted => json!({ "uncounted": true }),
        Some(t) => json!({ "passed": t.passed, "failed": t.failed }),
        None => serde_json::Value::Null,
    }
}

/// End a gate that did not finish: the human lines, the result object, the
/// AG-UI failure, and a non-zero exit (ADR-2611231400 §1).
/// Whether git holds `p`: in the index, or in HEAD. A `git mv` or `git rm`
/// takes the old path out of the index, so only HEAD still names it, and it is
/// a deletion to commit, not a mistake.
fn git_knows(root: &std::path::Path, p: &str) -> bool {
    let ok = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    ok(&["ls-files", "--error-unmatch", p]) || ok(&["cat-file", "-e", &format!("HEAD:{p}")])
}

fn gate_failed(mut base: serde_json::Value, f: hexa_exec::direct_exec::GateFailure, uncounted: bool, started: bool) -> anyhow::Result<()> {
    if let Some(out) = &f.output {
        println!("  {} the gate's output (last lines):", "\u{2192}".cyan());
        for l in out.lines() {
            println!("    {l}");
        }
    }
    if let Some(first) = &f.first_failure {
        println!("  {} first failure: {first}", "\u{2717}".red());
    }
    println!("{} [{}]\n  {}\n  {} {}", "\u{2717} not committed".red().bold(), f.code, f.why.replace('\n', "\n  "), "next:".dimmed(), f.hint);
    base["ok"] = json!(false);
    base["code"] = json!(f.code);
    base["error"] = json!(f.why.lines().next().unwrap_or(""));
    base["hint"] = json!(f.hint);
    base["field"] = json!(f.field);
    base["retryable"] = json!(f.retryable);
    if let Some(c) = &f.commit {
        base["commit"] = json!(c);
    }
    if f.tests.is_some() || (uncounted && f.output.is_some()) {
        base["tests"] = tests_json(f.tests, uncounted);
    }
    if let Some(first) = &f.first_failure {
        base["first_failure"] = json!(first);
    }
    if started {
        crate::commands::events::fail(&f.code.replace('-', "_"), &format!("not committed: {}", f.why.lines().next().unwrap_or("the gate did not pass")));
    }
    print_result(&base);
    anyhow::bail!("the gate did not pass ({})", f.code)
}

#[allow(clippy::too_many_arguments)]
async fn gate(
    files: Vec<String>,
    evidence: String,
    message: String,
    uncounted: bool,
    no_land: bool,
    local: bool,
    expect_red: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    use hexa_exec::direct_exec::{Expect, GateFailure};
    let expect = if expect_red { Expect::Red } else { Expect::Green };
    let top = std::process::Command::new("git").args(["rev-parse", "--show-toplevel"]).output()?;
    if !top.status.success() {
        let base = json!({ "ok": false, "mode": if dry_run { "dry-run" } else if local { "local" } else { "remote" }, "expect": expect.as_str(), "files": files });
        return gate_failed(base, *GateFailure::new("refused", "hexa do gate runs inside a git repository", "cd into the repository, then run the same command", None, false), uncounted, false);
    }
    let root = std::path::PathBuf::from(String::from_utf8_lossy(&top.stdout).trim().to_string());
    let branch = std::process::Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    // Under test.remote the gate runs on the remote runner, for a
    // candidate commit of exactly these files (ADR-2611221610 §7).
    let cfg = hexa_exec::test_exec::test_config(&root).map_err(|e| anyhow::anyhow!(e))?;
    let remote = cfg.remote.clone().filter(|_| !local);
    let mode = if dry_run { "dry-run" } else if remote.is_some() { "remote" } else { "local" };
    let mut base = json!({ "ok": false, "mode": mode, "expect": expect.as_str(), "files": files, "branch": branch });

    // Paths are given from where you stand; the gate runs and commits from the repository root.
    let cwd = std::env::current_dir()?;
    let mut rel: Vec<String> = Vec::new();
    for f in &files {
        match repo_relative(&root, &cwd, f) {
            // a path that is neither in the tree nor tracked (a deletion) is a mistake, not "no change": `--files a,b` used to report nothing to commit
            Some(p) if !root.join(&p).exists() && !git_knows(&root, &p) => {
                let hint = if f.contains(',') { "give several files separated by spaces, not commas" } else { "name a file that is in the tree, or a tracked file you deleted" };
                return gate_failed(base, *GateFailure::new("refused", format!("{f}: no such file in the tree or in git"), hint, Some("--files"), false), uncounted, false);
            }
            Some(p) => rel.push(p),
            None => {
                let why = format!("{f} is outside the repository ({}): only files in it can be gated and committed", root.display());
                return gate_failed(base, *GateFailure::new("refused", why, "name files inside the repository", Some("--files"), false), uncounted, false);
            }
        }
    }
    base["files"] = json!(rel);
    let lands = !no_land && !expect_red;

    if dry_run {
        println!("{} {} {}", "\u{2b21} gate:".cyan().bold(), evidence, "[dry run: nothing is pushed, run or committed]".dimmed());
        return match hexa_exec::test_exec::gate_plan(&root, &rel, &evidence, remote.as_deref()).await {
            Err(f) => gate_failed(base, *f, uncounted, false),
            Ok(plan) => {
                let then = match &plan.upstream {
                    Some((r, m)) => format!(
                        "on a {} verdict: move {} to the candidate, push it to {r} ({m}) and read it back{}",
                        expect.as_str(),
                        plan.branch,
                        if lands { ", then land it on the base" } else { "; no landing" }
                    ),
                    None => format!(
                        "on a {} verdict: commit the files on {}{}",
                        expect.as_str(),
                        plan.branch,
                        if lands { ", then land it on the base" } else { "; no landing" }
                    ),
                };
                let would = json!({
                    "parent": plan.parent,
                    "commit_files": plan.files,
                    "push": plan.upstream.as_ref().map(|(r, _)| json!({ "remote": r, "ref": "refs/heads/hexa-gate/<first 12 of the candidate>" })),
                    "run": plan.run.clone().unwrap_or_else(|| evidence.clone()),
                    "then": then,
                    "lands": lands,
                });
                println!("  {} commit {} on {} (parent {})", "would".dimmed(), plan.files.join(", "), plan.branch, &plan.parent[..12.min(plan.parent.len())]);
                if let Some((r, _)) = &plan.upstream {
                    println!("  {} push the candidate to {r} as refs/heads/hexa-gate/<first 12 of the candidate>", "would".dimmed());
                }
                println!("  {} run {}", "would".dimmed(), plan.run.as_deref().unwrap_or(&evidence));
                println!("  {} {then}", "would".dimmed());
                base["ok"] = json!(true);
                base["branch"] = json!(plan.branch);
                base["would"] = would;
                print_result(&base);
                Ok(())
            }
        };
    }

    crate::commands::loop_cmd::note_run_gate(&evidence, "build", None);
    if uncounted {
        crate::commands::loop_cmd::note_gate_uncounted(&evidence);
    }
    crate::commands::events::start(&root);
    crate::commands::events::step_start("gate");
    println!("{} {}{}", "\u{2b21} gate:".cyan().bold(), evidence, if expect_red { " [expect red]" } else { "" });
    println!("  {} {}", "files".dimmed(), rel.join(", "));
    let started_at = chrono::Utc::now().to_rfc3339();
    let clock = std::time::Instant::now();
    let outcome: Result<serde_json::Value, Box<GateFailure>> = match &remote {
        None => hexa_exec::direct_exec::gate_patch_local(&root, &rel, &evidence, &message, uncounted, expect).await.map(|g| {
            json!({ "commit": g.commit, "tests": tests_json(g.judged.tests, uncounted), "first_failure": g.judged.first_failure })
        }),
        Some(template) => hexa_exec::test_exec::gate_patch_remote(&root, &rel, &evidence, &message, uncounted, template, expect).await.map(|g| {
            println!("  {} the remote runner's output (the evidence, noted on the commit under refs/notes/hexa-gate):", "\u{2192}".cyan());
            for l in g.evidence.lines() {
                println!("    {l}");
            }
            println!("  {} {} pushed to {}, which reports it at {}", "\u{2713}".green(), g.branch, g.remote, &g.remote_head[..12.min(g.remote_head.len())]);
            json!({ "commit": g.commit, "remote": g.remote, "remote_head": g.remote_head,
                    "tests": tests_json(g.judged.tests, uncounted), "first_failure": g.judged.first_failure })
        }),
    };
    // `do runs` lists gate runs beside `do run`'s, from one feed.
    let logged: Result<String, String> = match &outcome {
        Ok(v) => Ok(v["commit"].as_str().unwrap_or("").chars().take(7).collect()),
        Err(f) => Err(f.why.clone()),
    };
    hexa_exec::direct_exec::record_gate_run(
        &hexa_exec::run_log(),
        &root,
        &rel,
        &message,
        &logged,
        started_at,
        u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    crate::commands::events::step_end("gate");
    let done = match outcome {
        Err(f) => return gate_failed(base, *f, uncounted, true),
        Ok(done) => done,
    };
    let sha = done["commit"].as_str().unwrap_or("").to_string();
    for k in ["commit", "remote", "remote_head", "tests"] {
        if !done[k].is_null() {
            base[k] = done[k].clone();
        }
    }
    base["ok"] = json!(true);
    if let Some(t) = done["tests"].as_object() {
        let counts = match (t.get("passed"), t.get("failed")) {
            (Some(p), Some(f)) => format!(" · {p} passed, {f} failed"),
            _ => " · uncounted".to_string(),
        };
        let what = if expect_red { "\u{2713} red, committed" } else { "\u{2713} gated" };
        println!("{} commit {} · {} file(s){counts}", what.green().bold(), sha.chars().take(12).collect::<String>().yellow(), rel.len());
    }
    if expect_red {
        if let Some(first) = done["first_failure"].as_str() {
            base["first_failure"] = json!(first);
        }
        // The red's binding is `hexa spec red`'s to make (ADR-2611231400 §5).
        let adr = crate::commands::loop_cmd::read_loop(&root)
            .and_then(|e| e.get("adr").and_then(|a| a.as_str()).map(str::to_string))
            .filter(|a| !a.trim().is_empty())
            .unwrap_or_else(|| "<ADR>".to_string());
        let next = format!("hexa spec red {adr} {sha}");
        println!("  {} {next}   (binds each scenario with --test S=<file>::<name>; a red never lands)", "next:".dimmed());
        base["next"] = json!(next);
    } else if lands {
        crate::commands::events::step_start("land");
        match crate::commands::land::land(&root, &evidence, uncounted, local).await {
            crate::commands::land::Landing::OnBase => {}
            crate::commands::land::Landing::Landed { base: b, sha } => {
                println!("{} on {} at {}", "\u{2713} landed".green().bold(), b, sha.yellow());
                base["landed"] = json!({ "base": b, "sha": sha });
            }
            crate::commands::land::Landing::NotLanded { base: b, why } => {
                println!("{} on {}: {}", "\u{26a0} not landed".yellow().bold(), b, why);
                base["not_landed"] = json!({ "base": b, "why": why });
            }
        }
        crate::commands::events::step_end("land");
    }
    crate::commands::events::finish(Some(base.clone()));
    print_result(&base);
    Ok(())
}

/// The one JSON object `hexa do run --json` prints on stdout (ADR-2610281500 §9).
fn emit_result(ok: bool, commit: Option<&str>, attempts: u64, code: Option<&str>, message: &str) {
    println!(
        "{}",
        json!({ "ok": ok, "commit": commit, "attempts": attempts, "code": code, "message": message })
    );
}

#[cfg(test)]
mod tests {
    use super::failure_stage;

    #[test]
    fn a_run_that_never_reached_its_evidence_says_where_it_stopped() {
        assert_eq!(failure_stage("no model configured — run `hexa config model t2 <model>`", false, ""), "inference");
        assert_eq!(failure_stage("inference: connection refused", false, ""), "inference");
        assert_eq!(
            failure_stage("exhausted 3 attempts without passing evidence. last: inference: no fenced code block in reply", false, ""),
            "parse"
        );
        assert_eq!(failure_stage("loop ended with no edit: I think the file is fine", false, ""), "parse");
        assert_eq!(failure_stage("workspace: no such worktree", false, ""), "setup");
    }

    #[test]
    fn a_run_whose_evidence_ran_fails_at_evidence_or_commit() {
        assert_eq!(failure_stage("exhausted 3 attempts", false, "test result: FAILED"), "evidence");
        assert_eq!(failure_stage("evidence PASSED but the commit failed", true, "ok"), "commit");
        assert_eq!(
            failure_stage("exhausted 3 attempts without passing evidence. last: evidence `false` exited non-zero", false, ""),
            "evidence"
        );
    }
}
