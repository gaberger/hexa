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
    /// commits nothing and leaves the change where it is.
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
        /// Gate here even when .hexa/project.json sets test.remote; without it a
        /// remote project refuses, since no remote runner sees an uncommitted
        /// change (ADR-2611191120 §6)
        #[arg(long)]
        local: bool,
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
        DoAction::Gate { files, evidence, message, uncounted, no_land, local } => {
            let top = std::process::Command::new("git").args(["rev-parse", "--show-toplevel"]).output()?;
            if !top.status.success() {
                anyhow::bail!("hexa do gate runs inside a git repository");
            }
            let root = std::path::PathBuf::from(String::from_utf8_lossy(&top.stdout).trim().to_string());
            // Paths are given from where you stand; the gate runs and commits from the repository root.
            let cwd = std::env::current_dir()?;
            let mut rel: Vec<String> = Vec::new();
            for f in &files {
                match repo_relative(&root, &cwd, f) {
                    // a path that is neither in the tree nor tracked (a deletion) is a mistake, not "no change": `--files a,b` used to report nothing to commit
                    Some(p) if !root.join(&p).exists() && std::process::Command::new("git").args(["ls-files", "--error-unmatch", &p]).current_dir(&root).output().map(|o| !o.status.success()).unwrap_or(true) => {
                        let hint = if f.contains(',') { "; give several files separated by spaces, not commas" } else { "" };
                        anyhow::bail!("{f}: no such file in the tree or in git{hint}")
                    }
                    Some(p) => rel.push(p),
                    None => anyhow::bail!("{f} is outside the repository ({}): only files in it can be gated and committed", root.display()),
                }
            }
            // A remote runner sees only pushed commits; this gates an
            // uncommitted change, so it never silently runs here (ADR-2611191120 §6).
            let cfg = hexa_exec::test_exec::test_config(&root).map_err(|e| anyhow::anyhow!(e))?;
            if cfg.remote.is_some() && !local {
                anyhow::bail!(
                    "refused: .hexa/project.json sets test.remote, so gates run on the remote runner, which sees only pushed \
                     commits, and `hexa do gate` gates an uncommitted change. Gate it here with --local, or commit and push it \
                     and close with `hexa loop stage done` or `hexa spec verify`, which send the gate to the remote. Nothing committed"
                );
            }
            crate::commands::loop_cmd::note_run_gate(&evidence, "build", None);
            if uncounted {
                crate::commands::loop_cmd::note_gate_uncounted(&evidence);
            }
            crate::commands::events::start(&root);
            crate::commands::events::step_start("gate");
            println!("{} {}", "⬡ gate:".cyan().bold(), evidence);
            println!("  {} {}", "files".dimmed(), rel.join(", "));
            let started_at = chrono::Utc::now().to_rfc3339();
            let clock = std::time::Instant::now();
            let outcome = hexa_exec::direct_exec::gate_patch(&root, &rel, &evidence, &message, uncounted).await;
            // `do runs` lists gate runs beside `do run`'s, from one feed.
            hexa_exec::direct_exec::record_gate_run(
                &hexa_exec::run_log(),
                &root,
                &rel,
                &message,
                &outcome,
                started_at,
                u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX),
            );
            crate::commands::events::step_end("gate");
            match outcome {
                Ok(sha) => {
                    println!("{} commit {} · {} file(s)", "✓ gated".green().bold(), sha.yellow(), rel.len());
                    let mut result = json!({ "commit": sha, "files": rel });
                    if !no_land {
                        crate::commands::events::step_start("land");
                        match crate::commands::land::land(&root, &evidence, uncounted, local).await {
                            crate::commands::land::Landing::OnBase => {}
                            crate::commands::land::Landing::Landed { base, sha } => {
                                println!("{} on {} at {}", "✓ landed".green().bold(), base, sha.yellow());
                                result["landed"] = json!({ "base": base, "sha": sha });
                            }
                            crate::commands::land::Landing::NotLanded { base, why } => {
                                println!("{} on {}: {}", "⚠ not landed".yellow().bold(), base, why);
                                result["not_landed"] = json!({ "base": base, "why": why });
                            }
                        }
                        crate::commands::events::step_end("land");
                    }
                    crate::commands::events::finish(Some(result));
                }
                Err(e) => {
                    println!("{}\n  {}", "✗ not committed".red().bold(), e.replace('\n', "\n  "));
                    let code = if e.contains("nothing to commit") { "nothing_to_commit" } else { "gate_failed" };
                    crate::commands::events::fail(code, &format!("not committed: {}", e.lines().next().unwrap_or("the gate did not pass")));
                    anyhow::bail!("the gate did not pass");
                }
            }
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
