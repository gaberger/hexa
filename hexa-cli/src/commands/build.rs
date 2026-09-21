//! `hexa build` and `hexa harden` — the cooperative + adversarial harness.
//!
//! Renamed from `hexa swarm build` / `hexa swarm review` (ADR-2608241500 P6.4).
//! The implementation in `hexa_exec::adversarial` is unchanged and is the
//! measured source of quality in this project: it built a ~2,900-LOC durable
//! job queue from a one-line spec, and its adversarial pass found six real
//! bugs the build's own passing tests missed.
//!
//! # Why the name changed and the code did not
//!
//! It is in-process fan-out of inference calls. There is no registry, no
//! heartbeat, no lease and no peer — nothing a "swarm" implies and nothing the
//! deleted coordination tier provided. The word was describing a cluster that
//! is not there. What it actually does is: propose several designs, attack
//! each, pick one, build it, then hunt the result for bugs.
//!
//! The rest of `hexa swarm` — `init`, `status`, `list`, `complete`, `fail`,
//! `cleanup`, `run` — was the fleet registry those verbs managed, kept in
//! SpacetimeDB behind the daemon. It went with them.
//!
//! # The gate is the only authority
//!
//! Both verbs take `--gate`, a shell command that must exit 0. Nothing here
//! decides a build is finished; the gate does.

use clap::Args;
use colored::Colorize;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// The challenge: a plain-language description of the system to build.
    pub challenge: String,
    /// Directory to build into (repo-relative).
    #[arg(long)]
    pub target: String,
    /// Ground-truth gate: a shell command that must exit 0.
    #[arg(long)]
    pub gate: String,
    /// Number of divergent designs (2-4).
    #[arg(long, default_value_t = 3)]
    pub designs: usize,
    /// After building, run the adversarial hunt-and-fix pass (the full harness).
    #[arg(long)]
    pub harden: bool,
    /// Per-call timeout in seconds for the diverge/red-team/synthesize calls.
    /// The build phase scales to 4x this value.
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
    /// Max attempts per call before giving up. Covers transient inference
    /// latency, which the synthesize step is most prone to.
    #[arg(long, default_value_t = 3)]
    pub retries: u32,
    /// Run in its own process group, output to `~/.hexa/runs/`, and return.
    /// The run then belongs to no terminal and no host tool (ADR-2609131611).
    #[arg(long)]
    pub detach: bool,
}

#[derive(Debug, Args)]
pub struct HardenArgs {
    /// Path to review (file or directory), repo-relative.
    pub target: String,
    /// Ground-truth gate: a shell command that must exit 0 after each fix.
    #[arg(long)]
    pub gate: String,
    /// Run in its own process group, output to `~/.hexa/runs/`, and return.
    /// The run then belongs to no terminal and no host tool (ADR-2609131611).
    #[arg(long)]
    pub detach: bool,
}

/// Can this run reach a model at all?
///
/// ADR-2609211600's review found `hexa harden` cycling every hunt dimension —
/// correctness, concurrency, durability-safety — failing each one on the same
/// unreachable provider. Each dimension costs a round trip to discover what
/// the first already knew, and the operator reads three failures where the
/// fact is one: nothing here serves a model.
///
/// Pure, so the decision is testable without a provider. `hexa doctor` asks
/// the same question of the same discovery; this is the check the verbs owed
/// it before their first inference call, not a second opinion.
fn first_blocker(found: &[hexa_infer::Found], verb: &str) -> Option<String> {
    if hexa_infer::discover::any_path(found) {
        return None;
    }
    Some(format!(
        "{verb} needs a model and no path to one is open. Run `hexa doctor` for what it \
         probed, then start a local server or put a frontier CLI on PATH. Nothing was \
         hunted, so nothing about the target is claimed."
    ))
}

/// `hexa build` — diverge → red-team → synthesize → build, optionally chaining
/// the adversarial pass for the full pipeline.
pub async fn run_build(args: BuildArgs) -> anyhow::Result<()> {
    if args.detach {
        return detach("build", &args.target);
    }
    // The gate is recorded in the loop, so the hooks can see the work is under one.
    if let Ok(cwd) = std::env::current_dir() {
        let _ = crate::commands::loop_cmd::update_loop(&cwd, serde_json::json!({ "gate": args.gate.clone(), "stage": "build" }));
    }
    let repo_root = std::env::current_dir()?;
    println!(
        "{} {} {} {}",
        "⬡ build".cyan().bold(),
        args.target.yellow(),
        "←".dimmed(),
        args.challenge
    );
    println!(
        "  diverge → red-team → synthesize → build{}",
        if args.harden { " → hunt → fix" } else { "" }
    );

    let b = hexa_exec::adversarial::run_build_with(
        &args.challenge,
        &args.target,
        &args.gate,
        args.designs,
        &repo_root,
        args.timeout,
        args.retries,
        reporter("build", &repo_root),
    )
    .await;
    running_done(&repo_root);
    println!(
        "{} {} designs → {} critiques → spec {}ch → build {}",
        "✓".green().bold(),
        b.designs,
        b.critiques,
        b.spec_chars,
        if b.build_ok { "GREEN".green() } else { "FAILED".red() }
    );
    for n in &b.notes {
        println!("  {} {}", "·".dimmed(), n.dimmed());
    }

    if args.harden && b.build_ok {
        println!("{} adversarial pass", "⬡".cyan());
        let r = hexa_exec::adversarial::run_review_with(&args.target, &args.gate, &repo_root, reporter("harden", &repo_root)).await;
        running_done(&repo_root);
        print_review(&r, "    ");
    }
    Ok(())
}

/// The argv of a detached run: this process's own, with `--detach` removed
/// so the child runs the work instead of detaching again.
fn detached_args(argv: impl IntoIterator<Item = String>) -> Vec<String> {
    argv.into_iter().skip(1).filter(|a| a != "--detach").collect()
}

/// A run's log: one file per verb, target and start, under `~/.hexa/runs/`.
fn run_log_path(verb: &str, target: &str, stamp: &str) -> PathBuf {
    // The file or directory itself, not the path to it: a log name is read
    // at a glance in a listing.
    let base = target.trim_matches('/').rsplit('/').next().unwrap_or(target);
    let slug: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".hexa/runs").join(format!("{verb}-{slug}-{stamp}.log"))
}

/// Re-run this command in its own process group with its output in a log,
/// then return. A host tool's timeout, a closed terminal and a hangup no
/// longer take a run with them mid-fix (ADR-2609131611 §3).
fn detach(verb: &str, target: &str) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let log = run_log_path(verb, target, &stamp);
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out = std::fs::File::create(&log)?;
    let err = out.try_clone()?;
    let exe = std::env::current_exe()?;
    let child = std::process::Command::new(exe)
        .args(detached_args(std::env::args()))
        .current_dir(std::env::current_dir()?)
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()?;
    println!("{} {} detached as pid {}", "⬡".cyan().bold(), verb, child.id());
    println!("  log   {}", log.display());
    println!("  watch tail -f {}", log.display());
    println!("  or    hexa loop, from any session in this checkout");
    Ok(())
}

/// Print each phase of a run as it happens and record it in the loop, so a
/// person at the terminal, a session polling the output, and `hexa loop`
/// all see the same thing (ADR-2609131427). Flushed per line: a run whose
/// output is piped to a file must still show progress as it goes.
fn reporter(verb: &'static str, repo_root: &std::path::Path) -> hexa_exec::adversarial::Reporter {
    use std::io::Write;
    let root = repo_root.to_path_buf();
    let started = chrono::Utc::now().to_rfc3339();
    std::sync::Arc::new(move |p: hexa_exec::adversarial::Progress| {
        let secs = p.elapsed.as_secs();
        println!("  {}  {}: {}", format!("{:>2}m{:02}s", secs / 60, secs % 60).dimmed(), p.phase.cyan(), p.message);
        let _ = std::io::stdout().flush();
        let _ = crate::commands::loop_cmd::update_loop(
            &root,
            serde_json::json!({ "running": {
                "verb": verb, "phase": p.phase, "message": p.message,
                "started": started, "updated": chrono::Utc::now().to_rfc3339(),
            }}),
        );
    })
}

/// The run is over; the loop no longer shows it running.
fn running_done(repo_root: &std::path::Path) {
    let _ = crate::commands::loop_cmd::update_loop(repo_root, serde_json::json!({ "running": serde_json::Value::Null }));
}

/// `hexa harden` — hunt the target for bugs by lens, verify each finding
/// skeptically, and fix the confirmed ones under the gate.
pub async fn run_harden(args: HardenArgs) -> anyhow::Result<()> {
    if args.detach {
        return detach("harden", &args.target);
    }
    // Before anything is recorded or hunted: a run that cannot reach a model
    // fails once, here, rather than once per dimension.
    if let Some(blocker) = first_blocker(&hexa_infer::discover::discover(), "hexa harden") {
        anyhow::bail!(blocker);
    }
    // The gate is recorded in the loop, so the hooks can see the work is under one.
    if let Ok(cwd) = std::env::current_dir() {
        let _ = crate::commands::loop_cmd::update_loop(&cwd, serde_json::json!({ "gate": args.gate.clone(), "stage": "harden" }));
    }
    let repo_root = std::env::current_dir()?;
    println!(
        "{} {} (gate: {})",
        "⬡ harden".cyan().bold(),
        args.target.yellow(),
        args.gate.dimmed()
    );
    println!("  hunt → skeptical-verify → fix-loop");
    let report = hexa_exec::adversarial::run_review_with(&args.target, &args.gate, &repo_root, reporter("harden", &repo_root)).await;
    running_done(&repo_root);
    print_review(&report, "  ");
    // A pass that reviewed nothing is not a pass: `hexa harden && ship`
    // would otherwise proceed on code no reviewer read (ADR-2609131646 §2).
    if !report.reviewed() {
        anyhow::bail!("nothing was reviewed: {} of {} lenses answered", report.answered, report.lenses);
    }
    Ok(())
}

/// Render a review report. Shared so `hexa build --harden` and `hexa harden`
/// cannot drift into reporting the same result two different ways.
fn print_review(report: &hexa_exec::adversarial::ReviewReport, indent: &str) {
    // A pass that reviewed nothing does not get a tick (ADR-2609131646 §2).
    let mark = if report.reviewed() { "\u{2713}".green().bold() } else { "\u{26d4}".red().bold() };
    println!("{}{} {}", indent, mark, report.verdict_line());
    for f in &report.confirmed {
        let mark = if report.fixed.contains(&f.title) { "✓".green() } else { "•".yellow() };
        println!("{}  {} [{}] {} {}", indent, mark, f.lens, f.title, f.location.dimmed());
    }
    for n in &report.notes {
        println!("{}  {} {}", indent, "·".dimmed(), n.dimmed());
    }
    let gate = match report.gate_line() {
        "PASS" => "PASS".green(),
        "FAIL" => "FAIL".red(),
        other => other.dimmed(),
    };
    println!("{}  final gate: {}", indent, gate);
}

#[cfg(test)]
mod preflight {
    use super::first_blocker;
    use hexa_infer::Found;

    fn path(reachable: Option<bool>) -> Found {
        Found {
            kind: "local",
            name: "Ollama".to_string(),
            detail: "http://127.0.0.1:11434".to_string(),
            via: "default address".to_string(),
            reachable,
            models: Vec::new(),
        }
    }

    /// The observed failure: `hexa harden` hunted correctness, then
    /// concurrency, then durability-safety, failing each on the same
    /// unreachable provider. One fact, reported three times, three round
    /// trips late.
    #[test]
    fn a_run_with_no_reachable_path_is_blocked_before_the_first_hunt() {
        let blocker = first_blocker(&[], "hexa harden").expect("no paths at all is a blocker");
        assert!(blocker.contains("hexa harden"), "it names the verb: {blocker}");
        assert!(blocker.contains("hexa doctor"), "and where to look: {blocker}");
        assert!(
            blocker.contains("nothing about the target is claimed"),
            "a run that hunted nothing must not read as a clean review: {blocker}"
        );

        assert!(
            first_blocker(&[path(Some(false))], "hexa harden").is_some(),
            "a path that failed its probe is not a path"
        );
    }

    /// The other half: a reachable path must not be turned away. A preflight
    /// that blocks a working run is worse than the three failures it saves.
    #[test]
    fn a_reachable_path_is_not_blocked() {
        assert!(first_blocker(&[path(Some(true))], "hexa harden").is_none());
        // Not probed is not the same as unreachable, and the run is allowed
        // to find out for itself.
        assert!(first_blocker(&[path(None)], "hexa harden").is_none());
    }
}

#[cfg(test)]
mod visible_run {
    use super::{detached_args, run_log_path};

    /// ADR-2609131611 §3: the child runs the work, not another detach, and
    /// every other flag survives verbatim.
    #[test]
    fn detach_is_dropped_from_the_command_the_detached_process_runs() {
        let argv = ["hexa", "harden", "src/x.rs", "--gate", "cargo test", "--detach"].map(String::from);
        assert_eq!(detached_args(argv), vec!["harden", "src/x.rs", "--gate", "cargo test"]);
        let argv = ["hexa", "build", "--detach", "a challenge", "--target", "src", "--gate", "g"].map(String::from);
        assert_eq!(detached_args(argv), vec!["build", "a challenge", "--target", "src", "--gate", "g"]);
    }

    #[test]
    fn a_runs_log_is_named_for_its_verb_target_and_start() {
        let p = run_log_path("harden", "hexa-cli/src/commands/loop_cmd.rs", "20260913T1611Z");
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(name, "harden-loop-cmd-rs-20260913T1611Z.log", "{name}");
        assert!(p.to_string_lossy().contains(".hexa/runs/"), "{}", p.display());
    }
}
