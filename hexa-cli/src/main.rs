// Pre-existing clippy lints — tracked for cleanup in ADR-2026-03-22-2050
#![allow(
    clippy::manual_strip,
    clippy::ptr_arg,
    clippy::unnecessary_sort_by,
    clippy::literal_string_with_formatting_args
)]
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use hexa_cli::commands;

use commands::{
    adr::AdrAction,
    bootstrap::BootstrapArgs,
    spec::SpecAction,
    analyze,
    doctor,
    hook::HookEvent,
    insight::InsightAction,
    init::InitArgs,
    refresh::RefreshArgs,
    memory::MemoryAction,
    plan::PlanAction,
    skill::SkillAction,
    status,
    worktree::WorktreeAction,
    hey::HeyArgs,
};

#[derive(Parser)]
#[command(
    name = "hexa",
    version = env!("HEXA_VERSION_LINE"),
    about = "Hexagonal architecture for LLM-driven development"
)]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Report the work as an AG-UI event stream (ADR-2611161830): JSON lines on stdout, every other line to stderr; `--events=<path>` writes them to a file instead. Also `HEXA_EVENTS=1` or `HEXA_EVENTS=<path>`
    #[arg(long, global = true, num_args = 0..=1, require_equals = true, default_missing_value = "-", value_name = "PATH")]
    events: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

// ── P2: hexa config — groups trust, taste, inference, enforce, secrets ──
#[derive(Subcommand)]
enum ConfigAction {    /// Manage inference providers (Ollama, vLLM, self-hosted)
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
    /// Show or set this project's models (tiers, and the hexa do loop's list)
    Model {
        /// t1, t2, t2.5, t3, or react — the list hexa do run tries first
        key: Option<String>,
        /// The model id to set; omit both to show what is configured
        model: Option<String>,
    },
}

// ── P3: hexa dev — groups analyze, validate, test, ci, worktree, init, new, report + session ──
#[derive(Subcommand)]
enum DevGroupAction {
    /// Architecture health check
    Analyze {
        /// Project root path
        #[arg(default_value = ".")]
        path: String,
        /// Exit 1 on any rule finding from .hexa/ADR-rules.toml, warnings included
        #[arg(long)]
        strict: bool,
        /// Check only the rules in .hexa/ADR-rules.toml; skip the boundary analysis and the grade
        #[arg(long)]
        adr_compliance: bool,
        /// Print the report as JSON. An ungraded tree has "graded": false and "grade": null
        #[arg(long)]
        json: bool,
        /// Check the imports of one file instead of the whole tree; exits 1 on a violation
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// Leave out the layer inventory and the health detectors (with --file, print nothing when clean)
        #[arg(long)]
        quiet: bool,
        /// Print only the violation lines, no report
        #[arg(long)]
        violations_only: bool,
        /// Exit 1 on any boundary violation or rule error, or when the tree could not be graded
        #[arg(long)]
        exit_code: bool,
        /// Exit 1 when the architecture grade is below this letter (A+, A, B, C, D, F)
        #[arg(long, value_name = "LETTER")]
        grade: Option<String>,
    },
    /// Run full build pipeline (build → test → analyze → validate)
    Validate {
        #[arg(long)]
        skip_test: bool,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        parallel: bool,
    },
    /// Run integration tests (unit, lint, arch, inference)
    Test {
        #[command(subcommand)]
        action: commands::test::TestAction,
    },
    /// Run all hexa enforcement gates
    Ci {
        #[arg(long)]
        standalone_gate: bool,
    },
    /// Does a native build script (C or C++) get the same cache key under every
    /// cargo command? Compares unit graphs; runs no compiler (ADR-2611151330)
    BuildKeys {
        /// A cargo command to compare, as arguments (repeatable), e.g. "test --no-run -p strata-server"
        #[arg(long = "selection", value_name = "ARGS")]
        selection: Vec<String>,
        /// JSON (the default when stdout is not a terminal)
        #[arg(long)]
        json: bool,
        /// A readable report, even when stdout is not a terminal
        #[arg(long)]
        text: bool,
    },
    /// Git worktree management (list, merge, cleanup)
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Initialize hexa in a project directory
    Init(InitArgs),
    /// Refresh hexa-managed sections of CLAUDE.md in place (no interview, no reset)
    Refresh(RefreshArgs),
    /// Structured project intake — create, init, register, seed trust
    New {
        /// Target directory path (default: the current directory)
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Scaffold language: rust | go | ts
        #[arg(long, default_value = "rust")]
        lang: String,
    },
}

#[derive(Subcommand)]
enum GateAction {
    /// Delete each module in turn, run the gate, and report every module whose
    /// deletion the gate survives — those are verified by nothing.
    Coverage {
        /// Directory to measure (repo-relative)
        #[arg(default_value = ".")]
        target: String,
        /// The gate: a shell command that must exit 0
        #[arg(long)]
        gate: String,
        /// The composition root, which is wiring rather than a subject.
        /// Inferred from the tree when omitted.
        #[arg(long = "composition-root")]
        composition_root: Option<String>,
        /// The gate runs no test runner: its exit status alone decides, and
        /// the report says `uncounted` (ADR-2611081151 §6).
        #[arg(long)]
        uncounted: bool,
    },
}

#[derive(Subcommand)]
enum Commands {
    // ════════════════════════════════════════════════════════════════════
    // Grouped parent commands (P2/P3/P4)
    // ════════════════════════════════════════════════════════════════════

    /// Project configuration (inference providers and model tiers)
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Development tools (analyze, validate, test, ci, worktree, init, new, refresh)
    Dev {
        #[command(subcommand)]
        action: DevGroupAction,
    },

    // ════════════════════════════════════════════════════════════════════
    // Standalone commands (not grouped)
    // ════════════════════════════════════════════════════════════════════

    /// Bootstrap hexa environment (prerequisites, services, models, config)
    Bootstrap(BootstrapArgs),
    /// Refresh hexa-managed sections of CLAUDE.md in place (no interview, no reset)
    Refresh(RefreshArgs),
    /// The next step for this repo, read from the loop (the `Next:` line of `hexa status`). Read-only
    Go,
    /// Knowledge graph — build/query/path/explain a project's code+docs graph
    Graph {
        #[command(subcommand)]
        action: commands::graph::GraphAction,
    },
    /// Say what you want in plain English; hexa picks the verb, shows it, and runs it on confirmation
    Hey(HeyArgs),
    /// Adversarially verify a claim about the repo — returns CONFIRMED / REFUTED / INCONCLUSIVE
    Verify(commands::verify::VerifyArgs),
    /// One gated change: an agent edits a file, your command runs, and it commits only on exit 0
    #[command(name = "do")]
    Do {
        #[command(subcommand)]
        action: commands::direct::DoAction,
    },
    /// Measure which models pass the agent loop here: the corpus, each model, in isolated worktrees
    Bench {
        #[command(subcommand)]
        action: commands::bench::BenchAction,
    },
    /// Scaffold a described project onto a deterministic hexagonal skeleton, via the frontier path, gated on the build AND the architecture grade
    Scaffold(commands::scaffold::ScaffoldArgs),
    /// Cooperative build — diverge, red-team, synthesize, then build to a gate
    Build(commands::build::BuildArgs),
    /// Adversarial pass — hunt a target for bugs, verify each, fix under a gate
    Harden(commands::build::HardenArgs),
    /// Find follow-ups that were mentioned in recent transcripts and routed nowhere
    Insight {
        #[command(subcommand)]
        action: InsightAction,
    },
    /// Persistent memory — this project's lessons, gaps and decisions
    Memory {
        /// Read and write the shared user store (~/.hexa) instead of this project's
        #[arg(long, global = true)]
        global: bool,
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// The API contract: OpenAPI from @hexa:api-tagged driving ports (spec, check, list)
    Api {
        #[command(subcommand)]
        action: commands::api::ApiAction,
    },
    /// The dead ends hexa knows: how often each is hit, and the pull request that closes one
    Gap {
        #[command(subcommand)]
        action: commands::gap::GapAction,
    },
    /// The repository's forge: record a host, and check this repository's target (writes nothing)
    Forge {
        #[command(subcommand)]
        action: commands::forge::ForgeAction,
    },
    /// File recorded `bug:`/`gap:` keys as issues on the repository's forge, once each. Sends nothing without --send
    Issue {
        #[command(subcommand)]
        action: commands::issue::IssueAction,
    },
    /// Open a pull request for the current, pushed branch. Sends nothing without --send; never pushes
    Pr {
        #[command(subcommand)]
        action: commands::pr::PrAction,
    },
    /// Where this project's work stands: Design → Decide → Gate → Build → Harden
    Loop {
        #[command(subcommand)]
        action: Option<commands::loop_cmd::LoopAction>,
    },
    /// What inference has cost: tokens and dollars, by source and model
    Spend(commands::spend_cmd::SpendArgs),
    /// How well a model uses hexa: run scenarios through `claude -p` and score the transcripts (real runs cost money)
    Eval {
        #[command(subcommand)]
        action: commands::eval::EvalAction,
    },
    /// Architecture Decision Records: list, search, accept, and check every citation and gate
    Adr {
        #[command(subcommand)]
        action: AdrAction,
    },
    /// Check the gate itself: which module can be deleted without the gate noticing
    Gate {
        #[command(subcommand)]
        action: GateAction,
    },
    /// Design before the ADR, with the Elephant-Goldfish Model (EGM, Google LLC, Apache-2.0): brainstorm, draft, goldfish review, resolve
    #[command(long_about = "Design before the ADR, with the Elephant-Goldfish Model (EGM, \
https://github.com/google/elephant-goldfish-model, Google LLC, Apache-2.0; see docs/ATTRIBUTIONS.md).\n\n\
The elephant drafts with the project's context; each goldfish reviewer is one tool-less completion that \
sees only its persona, its standard, the document and the source the document names. A person decides: \
no command settles a design. Commit it under docs/designs/ and take it into the pipeline as an ADR.")]
    Design {
        /// Ask every model afresh instead of answering a repeated prompt from
        /// the cache (sets HEXA_NO_CACHE=1)
        #[arg(long, global = true)]
        fresh: bool,
        #[command(subcommand)]
        action: commands::design::DesignAction,
    },
    /// Report where the docs and the code disagree, one EGM goldfish per document (Elephant-Goldfish Model)
    #[command(long_about = "Report where the docs and the code disagree, one goldfish per document, on the drift \
prompt of the Elephant-Goldfish Model (EGM, https://github.com/google/elephant-goldfish-model, Google LLC, \
Apache-2.0; see docs/ATTRIBUTIONS.md). Writes docs/analysis/<YYYY-MM-DD>_DRIFT_REPORT.md and nothing else. \
ADRs are history and are not checked.")]
    Drift(commands::design::DriftArgs),
    /// The gate-first pipeline: draft an ADR's spec, a person accepts it, seal tests that fail first, verify them
    Spec {
        #[command(subcommand)]
        action: SpecAction,
    },
    /// Workplans for a larger change: create, list, status, and reconcile against the code
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// Claude Code hook handler (called by .claude/settings.json hooks)
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
    /// Manage skills (list, sync, show)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// List and install what is embedded in the binary: scaffold templates, provider presets, schemas
    Assets {
        #[command(subcommand)]
        action: commands::assets_cmd::AssetsAction,
    },
    /// Where am I: the loop, what is in flight in this repo (--all: every project), and the next step. JSON when piped
    Status(commands::status::StatusArgs),
    /// The open work and the command that closes each item: uncommitted files, commits not pushed, tasks not done, branches not on main, worktrees to remove, plans never planned, the ADR not closed (ADR-2611161800)
    Finish(commands::finish::FinishArgs),
    /// The task ledger: register a task and the paths it may edit, report it done, list and release (JSON when piped)
    Agent {
        #[command(subcommand)]
        action: commands::agent::AgentAction,
    },
    /// Check the docs: freshness, terminology and module READMEs
    Docs {
        #[command(subcommand)]
        action: commands::docs::DocsAction,
    },
    /// Check the install: PATH, toolchain, inference, and whether this binary is behind
    Doctor {
        /// Show detailed output
        #[arg(long, short)]
        verbose: bool,
        /// Attempt to fix issues automatically
        #[arg(long, short)]
        fix: bool,
        /// Run a specific check only (e.g. "composition")
        #[arg(value_name = "CHECK")]
        check: Option<String>,
    },
    /// Install the newest published release (a binary built from a checkout is current only with it)
    #[command(name = "self-update")]
    SelfUpdate {
        /// Only check for updates, do not install
        #[arg(long)]
        check: bool,
        /// Install a specific version tag (e.g. v26.4.30)
        #[arg(long)]
        version: Option<String>,
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    // ════════════════════════════════════════════════════════════════════
    // Hidden aliases — old top-level commands still work but don't show in --help
    // ════════════════════════════════════════════════════════════════════
    /// (hidden) Manage inference — use `hexa config inference` instead
    #[command(hide = true)]
    Inference {
        #[command(subcommand)]
        action: commands::inference::InferenceAction,
    },
    /// Grade the architecture: boundary and rule violations, cycles, dead exports (also `hexa dev analyze`)
    Analyze {
        #[arg(default_value = ".")]
        path: String,
        /// Exit 1 on any rule finding from .hexa/ADR-rules.toml, warnings included
        #[arg(long)]
        strict: bool,
        /// Check only the rules in .hexa/ADR-rules.toml; skip the boundary analysis and the grade
        #[arg(long)]
        adr_compliance: bool,
        /// Print the report as JSON. An ungraded tree has "graded": false and "grade": null
        #[arg(long)]
        json: bool,
        /// Check the imports of one file instead of the whole tree; exits 1 on a violation
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// Leave out the layer inventory and the health detectors (with --file, print nothing when clean)
        #[arg(long)]
        quiet: bool,
        /// Print only the violation lines, no report
        #[arg(long)]
        violations_only: bool,
        /// Exit 1 on any boundary violation or rule error, or when the tree could not be graded
        #[arg(long)]
        exit_code: bool,
        /// Exit 1 when the architecture grade is below this letter (A+, A, B, C, D, F)
        #[arg(long, value_name = "LETTER")]
        grade: Option<String>,
    },
    /// (hidden) Validate — use `hexa dev validate` instead
    #[command(hide = true)]
    Validate {
        #[arg(long)]
        skip_test: bool,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        parallel: bool,
    },
    /// (hidden) Run tests — use `hexa dev test` instead
    #[command(hide = true)]
    Test {
        #[command(subcommand)]
        action: commands::test::TestAction,
    },
    /// (hidden) CI gates — use `hexa dev ci` instead
    #[command(hide = true)]
    Ci {
        #[arg(long)]
        standalone_gate: bool,
    },
    /// (hidden) Worktree management — use `hexa dev worktree` instead
    #[command(hide = true)]
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Initialize hexa in a project: .hexa/, CLAUDE.md, hooks and skills (also `hexa dev init`)
    Init(InitArgs),
    /// (hidden) New project — use `hexa dev new` instead
    #[command(hide = true)]
    New {
        /// Target directory path (default: the current directory)
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Scaffold language: rust | go | ts
        #[arg(long, default_value = "rust")]
        lang: String,
    },
}

#[tokio::main]
async fn main() {
    // `main` used to return `anyhow::Result<()>`, so Rust's Termination printed
    // the error with `Debug` — and anyhow's `Debug` appends a backtrace when
    // RUST_BACKTRACE is set. hexa's users are Rust developers, who commonly
    // export that globally, so "write the ADR first" arrived looking like a
    // crash. The message and the exit code were always right; a tool that looks
    // like it crashed while working teaches people to distrust it.
    //
    // `{:#}` prints the whole cause chain on one line and no backtrace. A
    // genuine panic is unaffected — that is a different path, and still says so.
    let result = run().await;
    // a run a verb left open is ended here, so an AG-UI stream always has its terminal event (ADR-2611161830 §8)
    let why = result.as_ref().err().map(|e| format!("{e:#}"));
    commands::events::close(why.as_deref().map_or(Ok(()), Err));
    if let Some(why) = why {
        eprintln!("Error: {why}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    // the verb's path, e.g. "analyze", "loop task add", "spec verify": every
    // working verb is one run, and its own step names it (ADR-2611161830 §2-3)
    let verb = {
        let mut parts = Vec::new();
        let mut m = &matches;
        while let Some((name, sub)) = m.subcommand() {
            parts.push(name.to_string());
            m = sub;
        }
        if parts.is_empty() { "status".to_string() } else { parts.join(" ") }
    };
    let from_env = std::env::var("HEXA_EVENTS").ok().filter(|v| !v.is_empty() && v != "0").map(|v| if v == "1" { "-".to_string() } else { v });
    // A hook's stdout belongs to the agent that runs it: HEXA_EVENTS exported
    // in that agent's shell must not turn it into an event stream. `--events`
    // given to a hook on purpose still works.
    let from_env = from_env.filter(|_| verb != "hook" && !verb.starts_with("hook "));
    if let Some(dest) = cli.events.clone().or(from_env) {
        commands::events::init(&dest)?;
        // 26.11.13 opened a run only in `finish` and `do gate`, so every other
        // verb left the stream empty; the run is opened here, for all of them
        commands::events::start_verb(&std::env::current_dir().unwrap_or_default(), &verb);
    }

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            // hexa with no args → status + getting-started guide (Layer 1)
            commands::status::run_text().await?;
            print_getting_started();
            return Ok(());
        }
    };

    match command {
        // ── Grouped parent commands (P2/P3/P4) ──────────────────────
        Commands::Config { action } => match action {
            ConfigAction::Inference { action } => commands::inference::run(action).await,
            ConfigAction::Model { key, model } => commands::inference::run_model(key, model),
        },
        Commands::Dev { action } => match action {
            DevGroupAction::Analyze { path, strict, adr_compliance, json, file, quiet, violations_only, exit_code, grade } => {
                analyze::run(&path, strict, adr_compliance, json, file.as_deref(), quiet, violations_only, exit_code, grade.as_deref()).await
            }
            DevGroupAction::Validate { skip_test, strict, parallel } => {
                doctor::run_validate_pipeline(skip_test, strict, parallel).await
            }
            DevGroupAction::Test { action } => commands::test::run(action).await,
            DevGroupAction::Ci { standalone_gate } => {
                if standalone_gate { commands::ci::run_standalone_gate().await }
                else { commands::ci::run().await }
            }
            DevGroupAction::BuildKeys { selection, json, text } => commands::build_keys::run(selection, json, text).await,
            DevGroupAction::Worktree { action } => commands::worktree::run(action).await,
            DevGroupAction::Init(args) => commands::init::run(args).await,
            DevGroupAction::Refresh(args) => commands::refresh::run(args).await,
            DevGroupAction::New { path, name, description, lang } => {
                commands::new::run(&path, name, description, &lang).await
            }
        },
        // ── Standalone commands ──────────────────────────────────────
        Commands::Bootstrap(args) => commands::bootstrap::run(args).await,
        Commands::Go => commands::go::run().await,
        Commands::Graph { action } => commands::graph::run(action).await,
        Commands::Hey(args) => commands::hey::run(args).await,
        Commands::Verify(args) => commands::verify::run(args).await,
        Commands::Do { action } => commands::direct::run(action).await,
        Commands::Bench { action } => commands::bench::run(action).await,
        Commands::Scaffold(args) => commands::scaffold::run(args).await,
        Commands::Build(args) => commands::build::run_build(args).await,
        Commands::Harden(args) => commands::build::run_harden(args).await,
        Commands::Insight { action } => commands::insight::run(action).await,
        Commands::Memory { action, global } => {
            let scope = if global {
                hexa_exec::ports::MemoryScope::Shared
            } else {
                hexa_exec::ports::MemoryScope::Project
            };
            commands::memory::run(action, scope).await
        }
        Commands::Api { action } => commands::api::run(action).await,
        Commands::Gap { action } => commands::gap::run(action).await,
        Commands::Forge { action } => commands::forge::run(action).await,
        Commands::Issue { action } => commands::issue::run(action).await,
        Commands::Pr { action } => commands::pr::run(action).await,
        Commands::Loop { action } => commands::loop_cmd::run(action).await,
        Commands::Spend(args) => commands::spend_cmd::run(args).await,
        Commands::Eval { action } => commands::eval::run(action).await,
        Commands::Adr { action } => commands::adr::run(action).await,
        Commands::Gate { action } => match action {
            GateAction::Coverage { target, gate, composition_root, uncounted } => {
                commands::gate::run(target, gate, composition_root, uncounted).await
            }
        },
        Commands::Design { fresh, action } => {
            if fresh {
                commands::design::fresh();
            }
            commands::design::run(action).await
        }
        Commands::Drift(args) => commands::design::run_drift(args).await,
        Commands::Spec { action } => commands::spec::run(action).await,
        Commands::Plan { action } => commands::plan::run(action).await,
        Commands::Hook { event } => commands::hook::run(event).await,
        Commands::Skill { action } => commands::skill::run(action).await,
        Commands::Assets { action } => commands::assets_cmd::run(action).await,
        Commands::Status(args) => status::run(args).await,
        Commands::Finish(args) => commands::finish::run(args).await,
        Commands::Agent { action } => commands::agent::run(action).await,
        Commands::Docs { action } => commands::docs::run(action).await,
        Commands::Doctor { verbose, fix, check } => {
            match check.as_deref() {
                Some("composition") | Some("inference") => {
                    doctor::composition::run_composition_check().await;
                    Ok(())
                }
                _ => doctor::run_doctor(verbose, fix).await,
            }
        }
        Commands::SelfUpdate { check, version, yes } => {
            commands::update::run(check, version, yes).await
        }
        // ── Hidden aliases (old top-level commands) ──────────────────
        Commands::Inference { action } => commands::inference::run(action).await,
        Commands::Analyze { path, strict, adr_compliance, json, file, quiet, violations_only, exit_code, grade } => {
            analyze::run(&path, strict, adr_compliance, json, file.as_deref(), quiet, violations_only, exit_code, grade.as_deref()).await
        }
        Commands::Validate { skip_test, strict, parallel } => {
            doctor::run_validate_pipeline(skip_test, strict, parallel).await
        }
        Commands::Test { action } => commands::test::run(action).await,
        Commands::Ci { standalone_gate } => {
            if standalone_gate { commands::ci::run_standalone_gate().await }
            else { commands::ci::run().await }
        }
        Commands::Worktree { action } => commands::worktree::run(action).await,
        Commands::Init(args) => commands::init::run(args).await,
        Commands::Refresh(args) => commands::refresh::run(args).await,
        Commands::New { path, name, description, lang } => {
            commands::new::run(&path, name, description, &lang).await
        }
    }
}

/// Getting-started guide printed after `hexa` with no args.
/// Progressive disclosure: shows the 5 commands a new user needs.
fn print_getting_started() {
    use colored::Colorize;
    println!();
    println!("{}", "  Getting started".bold());
    println!();
    println!("    {}          Where the work stands, and the next step", "hexa status".cyan());
    println!("    {}           Structured brief of recent activity", "hexa brief".cyan());
    println!("    {}      Workplan lifecycle (create, execute, status)", "hexa plan list".cyan());
    println!("    {}          Configure trust, taste, inference", "hexa config".cyan());
    println!("    {}             Development tools (analyze, test, ci)", "hexa dev".cyan());
    println!("    {}          System health check", "hexa doctor".cyan());
    println!();
    println!("    {} for full command list", "hexa --help".dimmed());
    println!();
}
