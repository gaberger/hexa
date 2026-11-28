//! `hexa init` — bootstrap hexa into a project directory.
//!
//! Creates the configuration files and directory structure needed for
//! hexa to operate in a target project. This is the "install hexa" step
//! that makes a project hexa-aware.
//!
//! Two modes:
//! - **Config-only** (default): `.hexa/`, `.claude/`, `AGENTS.md`, `CLAUDE.md`
//! - **Scaffold** (`--scaffold`): Also writes a runnable skeleton, with a
//!   README in each layer directory (ADR-2611221600 §5)
//!
//! AGENTS.md is the guide every agent reads; CLAUDE.md imports it and adds
//! only what Claude Code uses (ADR-2611221600 §1, §2).
//!
//! ## Template sourcing
//!
//! Skills, agents, and hooks deployed to target projects come from two places:
//!
//! 1. **`hexa-cli/assets/{skills,agents,hooks}/`** — the canonical embedded
//!    templates. hexa-nexus re-embeds these same directories via `rust-embed`
//!    in `hexa-nexus/src/templates.rs` and serves them through
//!    `POST /api/projects/init`.
//!
//! 2. **`create_scaffold()`** (below) — only creates the `src/` hexagonal
//!    layer directories programmatically. It does NOT read from any embedded
//!    asset prefix.
//!
//! There is no separate `scaffold/` asset subtree — it was removed as a
//! duplicate of the top-level `skills/`, `agents/`, `hooks/` directories.

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Args)]
pub struct InitArgs {
    /// Target directory (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Project name (defaults to directory name)
    #[arg(short, long)]
    pub name: Option<String>,

    /// Also write a runnable hexagonal skeleton (see --lang)
    #[arg(long)]
    pub scaffold: bool,

    /// Scaffold language: rust | go | ts | python | js
    #[arg(long, default_value = "rust")]
    pub lang: String,

    /// Skip creating CLAUDE.md (if you already have one)
    #[arg(long)]
    pub no_claude_md: bool,

    /// Re-run on an initialized project. Merges into the existing config: the
    /// project id, inference settings, analyze excludes and your own hooks are kept
    #[arg(short, long)]
    pub force: bool,

    /// Close the hand-edit path (ADR-2611160730 §4): add `Edit(src/**)` and
    /// `Write(src/**)` to `.claude/settings.json` `permissions.deny` and
    /// `Bash(hexa do *)` to `permissions.allow`. Merges, never clobbers, and
    /// is safe to re-run; on an initialized project it does only this.
    #[arg(long)]
    pub strict: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    // The directory exists before anything asks git where it sits.
    create_dir_if_missing(Path::new(&args.path))?;
    let target = PathBuf::from(&args.path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&args.path));

    let project_name = args
        .name
        .clone()
        .unwrap_or_else(|| {
            target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "hexa-project".to_string())
        });

    // ── 0. Claude Code mods (ADR-2610301129 §4) ────────────────────
    // User-global, so before the guard: re-running init on a project that
    // is already initialized still installs them.
    crate::commands::claude_mods::install_mods(false);

    // ── Guard: already initialized? ───────────────────────────────
    let hexa_dir = target.join(".hexa");
    if hexa_dir.exists() && !args.force && args.strict {
        apply_strict_permissions(&target)?;
        println!("  {} .claude/settings.json: strict permissions", "\u{2713}".green());
        return Ok(());
    }
    if hexa_dir.exists() && !args.force {
        anyhow::bail!(
            "Project already initialized at {}. Use --force to reinitialize.",
            target.display()
        );
    }

    println!(
        "{} Initializing hexa in {}",
        "\u{2b21}".cyan(),
        target.display().to_string().bold()
    );

    // Asked before anything is written: is the target already inside a repo?
    // A repo that encloses it, not just `target/.git`: `init apps/sub` inside
    // a repo nested a second one.
    let enclosing_repo = enclosing_git_root(&target);

    // Every file init writes, relative to `target`: the first commit holds
    // these and nothing else.
    let mut written: Vec<PathBuf> = Vec::new();

    // ── 1a. .hexa/project.json ─────────────────────────────────────
    create_project_json(&target, &project_name, args.name.is_some())?;
    written.push(".hexa/project.json".into());

    // ── 1c. .hexa/ADR-rules.toml (enforcement rules) ───────────────
    create_adr_rules_toml(&target)?;
    written.push(".hexa/ADR-rules.toml".into());

    // ── 1d. the task ledger is runtime state: keep it out of git (ADR-2611051850)
    crate::commands::task_ledger::ensure_ignored(&target)
        .context("Failed to write .hexa/.gitignore")?;
    written.push(".hexa/.gitignore".into());

    // ── 3. .claude/settings.json (hooks → hexa hook <event>) ──────
    create_claude_settings(&target)?;
    if args.strict {
        apply_strict_permissions(&target)?;
    }
    written.push(".claude/settings.json".into());

    // ── 4. AGENTS.md, then CLAUDE.md, which imports it ────────────
    // After project.json: "where builds run" is read from it.
    super::refresh::write_agents_md(&target, &project_name)?;
    written.push("AGENTS.md".into());
    if !args.no_claude_md {
        create_claude_md(&target, &project_name)?;
        written.push("CLAUDE.md".into());
    }

    // ── 5. docs/adrs/ ─────────────────────────────────────────────
    create_dir_if_missing(&target.join("docs/adrs"))?;

    // ── 6. Scaffold (optional) ────────────────────────────────────
    if args.scaffold {
        written.extend(scaffold_files(&target, &args.lang, &project_name)?);
        declare_package_root(&target, &args.lang, &project_name)?;
    }

    // ── 6b. .gitignore, before the first commit (ADR-2610281552 §3) ─
    // A new project's first commit holds its source, not its build output,
    // one checkout's session state, or a secret. Lines already there are kept.
    let ignore = target.join(".gitignore");
    let existing = std::fs::read_to_string(&ignore).ok();
    let merged = hexa_core::ports::project_floor::merged_gitignore(existing.as_deref(), &args.lang);
    // The secrets lines are part of every language's lines, so an existing .gitignore that lacks them
    // gets them too, whatever else it says about .env (ADR-2610281552 §3, amended 2026-11-16).
    if let Some(text) = merged {
        std::fs::write(&ignore, text)?;
        println!("  {} .gitignore", "\u{2713}".green());
    }
    written.push(".gitignore".into());

    // ── 7. Write the embedded templates (skills, hooks) ───────────
    // Baked into this binary by rust-embed. Before any commit, so the skills
    // are not left untracked. Extraction never overwrites, so an operator's
    // edits survive a re-init.
    let templates = extract_templates(&target);
    written.extend(shipped_template_paths(&target));

    // ── 8. git ────────────────────────────────────────────────────
    // A directory in no repository gets one, and a first commit of exactly
    // the files above, so `hexa do run`'s evidence-gated commits have
    // somewhere to land. Inside an existing repository nothing is staged or
    // committed: what is already there, and what is untracked (a `.env`, a
    // file in progress), is the user's. Non-fatal either way.
    let git_outcome = match &enclosing_repo {
        Some(root) => GitOutcome::ExistingRepo(root.clone()),
        None => init_repo_and_commit(&target, &project_name, &written),
    };

    // ── Summary ───────────────────────────────────────────────────
    println!();
    println!("  {} .hexa/project.json", "\u{2713}".green());
    println!("  {} .hexa/ADR-rules.toml (enforcement rules)", "\u{2713}".green());
    println!("  {} .claude/settings.json", "\u{2713}".green());
    println!(
        "  {} AGENTS.md (the guide every agent reads)",
        "\u{2713}".green()
    );
    if !args.no_claude_md {
        println!("  {} CLAUDE.md (imports AGENTS.md)", "\u{2713}".green());
    }
    println!("  {} docs/adrs/", "\u{2713}".green());
    if args.scaffold {
        println!("  {} src/ (hexagonal layers)", "\u{2713}".green());
    }
    match &git_outcome {
        GitOutcome::Committed => println!("  {} git init + initial commit", "\u{2713}".green()),
        GitOutcome::ExistingRepo(root) => {
            println!(
                "  {} not committed: {} is already a git repository. Review with `git status` and commit what you want.",
                "\u{2022}".dimmed(),
                root.display()
            );
        }
        GitOutcome::Skipped(why) => println!("  {} no initial commit: {why}", "\u{2717}".yellow()),
    }

    match &templates {
        Ok(created) => {
            let skills = created.iter().filter(|f| f.starts_with(".claude/skills/")).count();
            if skills > 0 {
                println!("  {} .claude/skills/ ({} files)", "\u{2713}".green(), skills);
            }
            let agents = created
                .iter()
                .filter(|f| f.starts_with(".claude/agents/"))
                .count();
            if agents > 0 {
                println!(
                    "  {} .claude/agents/ (hexa-lead, hexa-builder)",
                    "\u{2713}".green()
                );
            }
        }
        Err(e) => {
            println!("  {} skills: not written ({})", "\u{2717}".yellow(), e);
        }
    }

    // The code graph from the first session on (ADR-2611221610 §4).
    crate::commands::graph::build_after_init(&target).await;

    println!();
    println!(
        "{} Project {} is now hexa-aware",
        "\u{2b21}".cyan(),
        project_name.bold()
    );
    println!();
    println!("  Next steps:");
    if let Err(e) = &templates {
        println!("    {} Retry templates:      hexa init --force  ({e})", "\u{2022}".dimmed());
    }
    println!("    {} Calibrate models:     hexa config inference setup", "\u{2022}".dimmed());
    println!("    {} Check architecture:   hexa analyze .", "\u{2022}".dimmed());
    println!("    {} Do some work:         hexa do run \"<task>\" --file <f> --evidence \"<cmd>\"", "\u{2022}".dimmed());
    if !args.scaffold {
        println!("    {} Scaffold src/ dirs:   hexa init --scaffold .", "\u{2022}".dimmed());
    }

    Ok(())
}

// ── File generators ──────────────────────────────────────────────────

/// Write `.hexa/project.json`, or merge into the one that is there.
///
/// `--force` used to rewrite the file with a fresh id, losing the configured
/// models and `analyze.exclude`. Now an existing file keeps every key it has;
/// only a missing key is filled in, `hexVersion` is brought current, and
/// `name` changes only when `--name` was given. A file that is not a JSON
/// object is refused, not replaced.
fn create_project_json(target: &Path, name: &str, name_given: bool) -> Result<()> {
    let hexa_dir = target.join(".hexa");
    create_dir_if_missing(&hexa_dir)?;

    let project_json = hexa_dir.join("project.json");
    let existing = match fs::read_to_string(&project_json) {
        Ok(text) => Some(
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .filter(|v| v.is_object())
                .with_context(|| {
                    format!("{} is not a JSON object; fix it before init merges into it", project_json.display())
                })?,
        ),
        Err(_) => None,
    };
    let content = merge_project_json(existing, name, name_given, || {
        (Uuid::new_v4().to_string(), chrono::Utc::now().to_rfc3339())
    });
    fs::write(&project_json, serde_json::to_string_pretty(&content)?)
        .context("Failed to write .hexa/project.json")?;

    Ok(())
}

/// The project.json to write: `existing` with only missing keys filled in.
/// `fresh` mints the id and creation time, and is called only when needed.
fn merge_project_json(
    existing: Option<serde_json::Value>,
    name: &str,
    name_given: bool,
    fresh: impl FnOnce() -> (String, String),
) -> serde_json::Value {
    let mut v = existing.unwrap_or_else(|| serde_json::json!({}));
    let has = |v: &serde_json::Value, k: &str| v.get(k).is_some_and(|x| !x.is_null());
    if !has(&v, "id") || !has(&v, "createdAt") {
        let (id, now) = fresh();
        if !has(&v, "id") {
            v["id"] = serde_json::json!(id);
        }
        if !has(&v, "createdAt") {
            v["createdAt"] = serde_json::json!(now);
        }
    }
    if name_given || !has(&v, "name") {
        v["name"] = serde_json::json!(name);
    }
    v["hexVersion"] = serde_json::json!(env!("CARGO_PKG_VERSION"));
    if !has(&v, "lifecycle_enforcement") {
        v["lifecycle_enforcement"] = serde_json::json!("mandatory");
    }
    // Paths `hexa analyze` skips, relative to the project root. Vendored
    // code, generated output and template data go here, not in the tool.
    if !v["analyze"].is_object() {
        v["analyze"] = serde_json::json!({});
    }
    if !has(&v["analyze"], "exclude") {
        v["analyze"]["exclude"] = serde_json::json!([]);
    }
    v
}

/// Load the embedded settings template (ADR-2026-03-22-1522).
fn settings_template() -> String {
    crate::assets::Assets::get_str("templates/hexa-claude-settings.json")
        .expect("hexa-claude-settings.json must be embedded in assets/templates/")
}

pub fn create_claude_settings(target: &Path) -> Result<()> {
    let claude_dir = target.join(".claude");
    create_dir_if_missing(&claude_dir)?;

    let settings_path = claude_dir.join("settings.json");

    // Parse the embedded template (skip $schema — it's for editor hints only)
    let template: serde_json::Value = serde_json::from_str(&settings_template())
        .context("Failed to parse embedded hexa-claude-settings.json template")?;

    // If settings.json exists, merge template fields in rather than
    // overwriting. A file that is not JSON is refused, not replaced.
    let mut settings: serde_json::Value = match fs::read_to_string(&settings_path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .filter(|v| v.is_object())
            .with_context(|| {
                format!("{} is not a JSON object; fix it before init merges into it", settings_path.display())
            })?,
        _ => serde_json::json!({}),
    };

    // Hooks merge per event: the user's entries stay (init used to replace
    // the whole object and delete them); hexa's are added where missing.
    if let Some(hooks) = template.get("hooks") {
        merge_template_hooks(&mut settings, hooks);
    }
    // The status line and announcements are set only when there are none.
    for key in ["statusLine", "companyAnnouncements"] {
        if settings.get(key).is_none() {
            if let Some(v) = template.get(key) {
                settings[key] = v.clone();
            }
        }
    }

    // Set permissions only if not already configured (don't clobber user customization)
    if settings.get("permissions").is_none() {
        if let Some(perms) = template.get("permissions") {
            settings["permissions"] = perms.clone();
        }
    }

    fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
        .context("Failed to write .claude/settings.json")?;

    Ok(())
}

/// Every hook command hexa's template installs, by event (ADR-2611221600 §6:
/// `hexa doctor` checks each is present).
pub(crate) fn template_hook_commands() -> Vec<(String, String)> {
    let template: serde_json::Value =
        serde_json::from_str(&settings_template()).unwrap_or_default();
    let mut out = Vec::new();
    if let Some(hooks) = template.get("hooks").and_then(|h| h.as_object()) {
        for (event, entries) in hooks {
            for cmd in hook_commands(entries.as_array().map(Vec::as_slice).unwrap_or_default()) {
                out.push((event.clone(), cmd));
            }
        }
    }
    out
}

/// The hook commands of every event in a settings object, by event.
pub(crate) fn settings_hook_commands(settings: &serde_json::Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(hooks) = settings.get("hooks").and_then(|h| h.as_object()) {
        for (event, entries) in hooks {
            for cmd in hook_commands(entries.as_array().map(Vec::as_slice).unwrap_or_default()) {
                out.push((event.clone(), cmd));
            }
        }
    }
    out
}

/// Hook commands hexa's template once installed and no longer does. Each is
/// broken, so init removes an entry running exactly one of these:
/// - Claude Code sets no `$TOOL_INPUT_PATH` (hook input is JSON on stdin), so
///   the first analyzed the whole directory on every edit and always exited 0.
/// - The second failed every turn in a project with no source files yet.
const RETIRED_HOOK_COMMANDS: &[&str] = &[
    "hexa analyze --file \"$TOOL_INPUT_PATH\" --quiet",
    "hexa analyze --violations-only --exit-code",
];

/// The command strings of every hook in one event's entries.
fn hook_commands(entries: &[serde_json::Value]) -> std::collections::BTreeSet<String> {
    entries
        .iter()
        .filter_map(|e| e.get("hooks").and_then(|h| h.as_array()))
        .flatten()
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()).map(str::to_string))
        .collect()
}

/// Merge the template's hooks into `settings["hooks"]`, per event.
///
/// Every entry already there is kept, except a hook running one of
/// [`RETIRED_HOOK_COMMANDS`]. A template hook is added only when no entry of
/// that event already runs the same command, so a second run changes
/// nothing. Events the template does not name are untouched.
fn merge_template_hooks(settings: &mut serde_json::Value, template_hooks: &serde_json::Value) {
    let Some(template) = template_hooks.as_object() else { return };
    if !settings["hooks"].is_object() {
        settings["hooks"] = serde_json::json!({});
    }
    let hooks = settings["hooks"].as_object_mut().expect("just made an object");
    for (event, template_entries) in template {
        let Some(template_entries) = template_entries.as_array() else { continue };
        let mut entries: Vec<serde_json::Value> =
            hooks.get(event).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        // Drop retired hexa hooks; an entry left with none is dropped too.
        entries.retain_mut(|e| {
            let Some(inner) = e.get_mut("hooks").and_then(|h| h.as_array_mut()) else { return true };
            let before = inner.len();
            inner.retain(|h| !h.get("command").and_then(|c| c.as_str()).is_some_and(|c| RETIRED_HOOK_COMMANDS.contains(&c)));
            !(before > 0 && inner.is_empty())
        });
        for t in template_entries {
            let present = hook_commands(&entries);
            let Some(inner) = t.get("hooks").and_then(|h| h.as_array()) else { continue };
            let missing: Vec<serde_json::Value> = inner
                .iter()
                .filter(|h| !h.get("command").and_then(|c| c.as_str()).is_some_and(|c| present.contains(c)))
                .cloned()
                .collect();
            if !missing.is_empty() {
                let mut entry = t.clone();
                entry["hooks"] = serde_json::Value::Array(missing);
                entries.push(entry);
            }
        }
        hooks.insert(event.clone(), serde_json::Value::Array(entries));
    }
}

/// What `hexa init --strict` denies and allows (ADR-2611160730 §4): source
/// is edited through `hexa do`, which gates the edit, not by hand.
const STRICT_DENY: &[&str] = &["Edit(src/**)", "Write(src/**)"];
const STRICT_ALLOW: &[&str] = &["Bash(hexa do *)"];

/// Merge the strict permissions into a settings object. Existing keys and
/// entries are kept; each rule is added once. Returns whether anything changed.
fn merge_strict_permissions(settings: &mut serde_json::Value) -> bool {
    if !settings.is_object() {
        *settings = serde_json::json!({});
    }
    if !settings["permissions"].is_object() {
        settings["permissions"] = serde_json::json!({});
    }
    let mut changed = false;
    for (key, rules) in [("deny", STRICT_DENY), ("allow", STRICT_ALLOW)] {
        if !settings["permissions"][key].is_array() {
            settings["permissions"][key] = serde_json::json!([]);
        }
        let list = settings["permissions"][key].as_array_mut().expect("just made an array");
        for rule in rules {
            if !list.iter().any(|v| v.as_str() == Some(rule)) {
                list.push(serde_json::json!(rule));
                changed = true;
            }
        }
    }
    changed
}

/// `hexa init --strict`: merge the strict permissions into
/// `.claude/settings.json`, creating it if needed. A file that is not JSON is
/// refused rather than replaced.
pub fn apply_strict_permissions(target: &Path) -> Result<()> {
    let claude_dir = target.join(".claude");
    create_dir_if_missing(&claude_dir)?;
    let path = claude_dir.join("settings.json");
    let mut settings: serde_json::Value = match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON; fix it before `--strict` merges into it", path.display()))?,
        _ => serde_json::json!({}),
    };
    if merge_strict_permissions(&mut settings) {
        fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    Ok(())
}

fn create_claude_md(target: &Path, project_name: &str) -> Result<()> {
    let claude_md_path = target.join("CLAUDE.md");

    // Don't overwrite existing CLAUDE.md — append hexa rules instead
    if claude_md_path.exists() {
        let existing = fs::read_to_string(&claude_md_path)?;
        // Already marker-wrapped: bring the managed section current, so the
        // rollout (`hexa init --force`) leaves CLAUDE.md importing AGENTS.md.
        let body = super::refresh::claude_section();
        match hexa_core::ports::orientation::replace_managed(
            &existing,
            &hexa_core::ports::orientation::CLAUDE_MD,
            &body,
        ) {
            Ok(Some(updated)) => {
                if updated != existing {
                    fs::write(&claude_md_path, updated).context("Failed to update CLAUDE.md")?;
                }
                return Ok(());
            }
            Err(e) => anyhow::bail!("CLAUDE.md: {e}; fix the markers by hand"),
            Ok(None) => {}
        }
        if existing.contains("Hexagonal Architecture Rules") {
            // A legacy hexa section. `hexa refresh` upgrades it; init must not
            // append a second copy.
            return Ok(());
        }
        // Append hexa section
        let appended = format!("{}\n\n{}", existing.trim(), hexa_claude_md_section());
        fs::write(&claude_md_path, appended)
            .context("Failed to append to CLAUDE.md")?;
        return Ok(());
    }

    // Only what Claude Code reads: the import and the Claude-only parts. The
    // project's own rules live in AGENTS.md, for every agent (ADR-2611221600 §2).
    let content = format!("# {project_name}\n\n{}\n", hexa_claude_md_section());

    fs::write(&claude_md_path, content)
        .context("Failed to write CLAUDE.md")?;

    Ok(())
}

/// The hexa-managed section, already wrapped in its refresh markers.
///
/// Written wrapped so `hexa refresh` can replace it later as a pure span swap.
/// A bare section leaves the file with no marker and no legacy heading, and
/// refresh correctly refuses to guess where a hand-written file ends.
fn hexa_claude_md_section() -> String {
    super::refresh::wrapped_hex_section()
}

/// The languages `--scaffold` can emit, and the command that gates each one.
///
/// The gate is part of the scaffold's identity, not an afterthought: a
/// skeleton you cannot run is a skeleton you cannot check, and gate-first
/// development (ADR-2609121400) has nothing to start from.
pub const SCAFFOLD_LANGS: &[(&str, &str)] = &[
    ("rust", "cargo test"),
    ("go", "go test ./..."),
    ("ts", "npm install && npm test"),
    // The standard library's runner: no install, no network (ADR-2611230850 §2).
    ("python", "python3 -m unittest discover -s tests -t ."),
    // Node's own runner: no install, no network (ADR-2611271700 §2).
    ("js", "node --test"),
];

/// The gate command for a language, or `None` if it is not one we emit.
pub fn scaffold_gate(lang: &str) -> Option<&'static str> {
    SCAFFOLD_LANGS.iter().find(|(l, _)| *l == lang).map(|(_, g)| *g)
}

/// Write a runnable hexagonal skeleton for `lang` into `target`.
///
/// **Deterministic by construction.** Every byte comes from a template
/// embedded in this binary plus two substitutions derived from the project
/// name. No inference, no network, no clock, no filesystem scan — the same
/// name produces the same bytes on every machine, which is what makes the
/// output something you can gate.
///
/// It used to create eleven empty directories and one TypeScript file of
/// TODO comments: no manifest, no test runner, nothing to execute. That is
/// the gap this closes.
///
/// Existing files are never overwritten, so re-running `hexa init --scaffold`
/// on a live project is safe.
pub(crate) fn create_scaffold(target: &Path, lang: &str, project_name: &str) -> Result<usize> {
    scaffold_files(target, lang, project_name).map(|w| w.len())
}

/// [`create_scaffold`], returning the files written, relative to `target`.
/// A Python scaffold's package `__init__.py` is in no folder that names a layer, so
/// the project declares it the package's root, as `lib.rs` is a crate's, under
/// `analyze.layers` (ADR-2611230850 §1, amended). Other languages declare nothing.
fn declare_package_root(target: &Path, lang: &str, project_name: &str) -> Result<()> {
    if lang != "python" {
        return Ok(());
    }
    let root = format!("{}/__init__.py", ScaffoldVars::from_name(project_name).name_snake);
    let path = target.join(".hexa/project.json");
    let mut v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    if !v["analyze"].is_object() {
        v["analyze"] = serde_json::json!({});
    }
    if !v["analyze"]["layers"].is_object() {
        v["analyze"]["layers"] = serde_json::json!({});
    }
    v["analyze"]["layers"][root.as_str()] = serde_json::json!("composition-root");
    fs::write(&path, serde_json::to_string_pretty(&v)? + "\n").with_context(|| format!("writing {}", path.display()))
}

fn scaffold_files(target: &Path, lang: &str, project_name: &str) -> Result<Vec<PathBuf>> {
    let Some(gate) = scaffold_gate(lang) else {
        anyhow::bail!(
            "unknown --lang '{}'; expected one of: {}",
            lang,
            SCAFFOLD_LANGS.iter().map(|(l, _)| *l).collect::<Vec<_>>().join(", ")
        );
    };

    let prefix = format!("scaffold/{lang}/");
    let vars = ScaffoldVars::from_name(project_name);
    let mut written: Vec<PathBuf> = Vec::new();
    // Counted separately from `written`, because "every file was already there"
    // and "this language ships no templates" are opposite conditions that both
    // leave `written` at zero. Reporting the first as the second sent a reader
    // looking for a missing asset bundle that was present and complete.
    let mut found = 0usize;
    let mut rels: Vec<String> = Vec::new();

    for path in crate::assets::Assets::iter() {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        // `.tmpl` marks a file whose *name* would otherwise be picked up by a
        // build tool sitting in the assets tree. The suffix is dropped here.
        let rel = rel.strip_suffix(".tmpl").unwrap_or(rel);
        // A path is rendered as contents are, so a Python package directory
        // can carry the project's name (ADR-2611230850 §3).
        let rel = vars.render(rel);
        // A rendered path stays inside the target: a name such as `../x`
        // must not write elsewhere (CLAUDE.md: path-traversal protection on
        // every file write).
        if !rendered_path_stays_inside(&rel) {
            anyhow::bail!("scaffold asset {path} renders to {rel}, which is outside the target directory");
        }
        let rel = rel.as_str();
        found += 1;
        rels.push(rel.to_string());
        let dest = target.join(rel);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            create_dir_if_missing(parent)?;
        }
        let body = crate::assets::Assets::get_str(&path)
            .ok_or_else(|| anyhow::anyhow!("scaffold asset {path} is not embedded"))?;
        fs::write(&dest, vars.render(&body))
            .with_context(|| format!("writing {}", dest.display()))?;
        written.push(PathBuf::from(rel));
    }

    if found == 0 {
        anyhow::bail!("no scaffold assets embedded for --lang {lang}");
    }
    written.extend(write_module_readmes(target, &rels, gate)?);
    if written.is_empty() {
        return Ok(written);
    }
    println!("  {} {} files ({}) — gate: {}", "\u{2713}".green(), written.len(), lang, gate);
    Ok(written)
}

/// Write a README into each layer directory among `rels`' parents, unless one
/// is there (ADR-2611221600 §5). Returns the files written, relative to `target`.
fn write_module_readmes(target: &Path, rels: &[String], gate: &str) -> Result<Vec<PathBuf>> {
    use hexa_core::ports::orientation::{module_readme, Layer};
    let mut dirs = std::collections::BTreeSet::new();
    for rel in rels {
        let mut at = Path::new(rel).parent();
        while let Some(d) = at.filter(|d| !d.as_os_str().is_empty()) {
            if Layer::of_dir(&d.to_string_lossy()).is_some() {
                dirs.insert(d.to_path_buf());
            }
            at = d.parent();
        }
    }
    let mut written = Vec::new();
    for d in dirs {
        let rel = d.join("README.md");
        let dest = target.join(&rel);
        if dest.exists() {
            continue;
        }
        let name = d.to_string_lossy().replace('\\', "/");
        fs::write(&dest, module_readme(&name, Layer::of_dir(&name), gate))
            .with_context(|| format!("writing {}", dest.display()))?;
        written.push(rel);
    }
    Ok(written)
}

/// The substitutions a scaffold template may use.
///
/// Two, deliberately. `{{name}}` is the project as written; `{{name_snake}}`
/// is it as an identifier, because Rust crate paths and Go package names
/// cannot contain a hyphen while directory names routinely do.
struct ScaffoldVars {
    name: String,
    name_snake: String,
}

impl ScaffoldVars {
    fn from_name(name: &str) -> Self {
        let snake: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect();
        // An identifier may not start with a digit, and an empty one is not an
        // identifier at all.
        let snake = match snake.chars().next() {
            Some(c) if c.is_ascii_digit() => format!("p_{snake}"),
            None => "app".to_string(),
            _ => snake,
        };
        Self { name: name.to_string(), name_snake: snake }
    }

    fn render(&self, body: &str) -> String {
        body.replace("{{name_snake}}", &self.name_snake).replace("{{name}}", &self.name)
    }
}


/// Whether a rendered asset path stays inside the target directory: relative,
/// and with no `..` component.
fn rendered_path_stays_inside(rel: &str) -> bool {
    let p = Path::new(rel);
    !p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

pub(crate) fn create_adr_rules_toml(target: &Path) -> Result<()> {
    let hexa_dir = target.join(".hexa");
    create_dir_if_missing(&hexa_dir)?;

    let rules_path = hexa_dir.join("ADR-rules.toml");
    if rules_path.exists() {
        // Never overwrite existing rules
        return Ok(());
    }

    let content = crate::assets::Assets::get_str("templates/ADR-rules.toml")
        .expect("templates/ADR-rules.toml must be embedded in assets/templates/");

    fs::write(&rules_path, content)
        .context("Failed to write .hexa/ADR-rules.toml")?;

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────

fn create_dir_if_missing(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

/// The root of the git repository that contains `dir`, if any. `None` also
/// when git is missing: init then tries `git init`, which says so.
fn enclosing_git_root(dir: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

/// What init did with git.
enum GitOutcome {
    /// A new repository and a first commit of the files init wrote.
    Committed,
    /// The target was already in this repository; nothing staged or committed.
    ExistingRepo(PathBuf),
    /// No commit, and why.
    Skipped(String),
}

/// `git init` a directory that is in no repository, and commit exactly
/// `paths` (relative to `target`) as the first commit.
///
/// Explicit pathspecs, never `add -A`: a `.env` or a file the user had in the
/// directory before init is not init's to commit. Uses the commit-local
/// factory identity (ADR-2606071323 §4: attributable, never masquerades as
/// the operator). Non-fatal: init already succeeded by the time this runs.
fn init_repo_and_commit(target: &Path, project_name: &str, paths: &[PathBuf]) -> GitOutcome {
    let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(target).output();
    match git(&["init", "-q"]) {
        Ok(out) if out.status.success() => {}
        Ok(out) => return GitOutcome::Skipped(format!("git init failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => return GitOutcome::Skipped(format!("git not found ({e})")),
    }

    let mut pathspec: Vec<String> = paths
        .iter()
        .filter(|p| target.join(p).is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    pathspec.sort();
    pathspec.dedup();
    if pathspec.is_empty() {
        return GitOutcome::Skipped("nothing to commit".into());
    }
    let mut add = vec!["add", "--"];
    add.extend(pathspec.iter().map(String::as_str));
    match git(&add) {
        Ok(out) if out.status.success() => {}
        Ok(out) => return GitOutcome::Skipped(format!("git add failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => return GitOutcome::Skipped(format!("git add failed ({e})")),
    }

    let msg = format!(
        "Initial hexa scaffold for {project_name}\n\nCo-Authored-By: hexa-factory <noreply@hexa.local>"
    );
    match git(&["-c", "user.name=hexa-factory", "-c", "user.email=factory@hexa.local", "commit", "-q", "-m", &msg]) {
        Ok(out) if out.status.success() => GitOutcome::Committed,
        Ok(out) => GitOutcome::Skipped(format!("git commit failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => GitOutcome::Skipped(format!("git commit failed ({e})")),
    }
}

/// The embedded template prefixes and the `.claude/` directory each goes to.
/// `agents/` holds the lead and builder roles (ADR-2611221600 §4).
pub(crate) const TEMPLATE_DIRS: &[(&str, &str)] = &[
    ("skills/", "skills"),
    ("agents/", "agents"),
    ("hooks/", "hooks"),
];

/// Write the embedded `skills/`, `agents/` and `hooks/` templates into `.claude/`,
/// returning the paths created, relative to `target`.
///
/// Never overwrites: an existing file is a customisation, not a stale copy.
/// The five skills that once shipped as flat files Claude Code never loaded
/// have their old copies removed once the directory form is in place.
fn extract_templates(target: &Path) -> std::io::Result<Vec<String>> {
    use crate::assets::Assets;
    let claude = target.join(".claude");
    let mut created = Vec::new();
    for (prefix, dir) in TEMPLATE_DIRS {
        created.extend(Assets::extract_to(prefix, &claude.join(dir))?.into_iter().map(|f| format!(".claude/{dir}/{f}")));
    }
    crate::commands::skill::remove_legacy_flat_skills(&claude.join("skills"))?;
    Ok(created)
}

/// Every shipped template file present in `target/.claude/`, relative to
/// `target`: what the first commit carries of them.
fn shipped_template_paths(target: &Path) -> Vec<PathBuf> {
    crate::assets::Assets::iter()
        .filter_map(|path| {
            TEMPLATE_DIRS.iter().find_map(|(prefix, dir)| {
                let rel = path.strip_prefix(prefix)?;
                let rel = rel.strip_suffix(".tmpl").unwrap_or(rel);
                Some(PathBuf::from(format!(".claude/{dir}/{rel}")))
            })
        })
        .filter(|p| target.join(p).is_file())
        .collect()
}

#[cfg(test)]
mod scaffold_tests {
    use super::*;

    #[test]
    fn strict_permissions_merge_once_and_keep_what_is_there() {
        let mut s = serde_json::json!({"model": "m", "permissions": {"deny": ["Read(./.env)"], "ask": ["Bash(rm *)"]}});
        assert!(merge_strict_permissions(&mut s));
        assert!(!merge_strict_permissions(&mut s), "a second merge changes nothing");
        assert_eq!(s["model"], "m");
        assert_eq!(s["permissions"]["ask"], serde_json::json!(["Bash(rm *)"]));
        assert_eq!(
            s["permissions"]["deny"],
            serde_json::json!(["Read(./.env)", "Edit(src/**)", "Write(src/**)"])
        );
        assert_eq!(s["permissions"]["allow"], serde_json::json!(["Bash(hexa do *)"]));
    }

    #[test]
    fn a_retired_hexa_hook_is_dropped_and_a_users_hook_with_it_is_kept() {
        let mut s = serde_json::json!({"hooks": {"Stop": [
            {"hooks": [{"type": "command", "command": "hexa analyze --violations-only --exit-code"}]},
            {"hooks": [{"type": "command", "command": "hexa analyze --violations-only --exit-code"}, {"type": "command", "command": "mine"}]}
        ]}});
        let template = serde_json::json!({"Stop": [{"hooks": [{"type": "command", "command": "hexa hook observe-stop"}]}]});
        merge_template_hooks(&mut s, &template);
        assert_eq!(
            s["hooks"]["Stop"],
            serde_json::json!([
                {"hooks": [{"type": "command", "command": "mine"}]},
                {"hooks": [{"type": "command", "command": "hexa hook observe-stop"}]}
            ])
        );
        let once = s.clone();
        merge_template_hooks(&mut s, &template);
        assert_eq!(s, once, "a second merge changes nothing");
    }

    #[test]
    fn secrets_are_ignored_even_when_the_file_already_names_some_env_line() {
        let text = hexa_core::ports::project_floor::merged_gitignore(Some("*.log\n.env\n"), "go").expect("lines added");
        assert!(text.contains("\n.env*\n") && text.contains("!.env.example"), "{text}");
        assert_eq!(hexa_core::ports::project_floor::merged_gitignore(Some(&text), "go"), None, "added once");
    }

    #[test]
    fn every_language_declares_a_gate() {
        for (lang, gate) in SCAFFOLD_LANGS {
            assert!(!gate.is_empty(), "{lang} has no gate command");
            assert_eq!(scaffold_gate(lang), Some(*gate));
        }
        assert_eq!(scaffold_gate("cobol"), None);
    }

    #[test]
    fn a_hyphenated_name_becomes_a_legal_identifier() {
        let v = ScaffoldVars::from_name("my-cool-app");
        assert_eq!(v.name, "my-cool-app", "the name is kept as written");
        assert_eq!(v.name_snake, "my_cool_app", "the identifier cannot hold a hyphen");
    }

    #[test]
    fn an_identifier_never_starts_with_a_digit() {
        assert_eq!(ScaffoldVars::from_name("2048-game").name_snake, "p_2048_game");
    }

    #[test]
    fn an_empty_name_still_yields_an_identifier() {
        assert_eq!(ScaffoldVars::from_name("").name_snake, "app");
    }

    #[test]
    fn the_longer_placeholder_is_substituted_first() {
        // Replacing `{{name}}` first would leave `_snake` dangling inside
        // `{{name_snake}}`. Order matters and this pins it.
        let v = ScaffoldVars::from_name("my-app");
        assert_eq!(v.render("{{name_snake}}"), "my_app");
        assert_eq!(v.render("mod {{name_snake}}; // {{name}}"), "mod my_app; // my-app");
    }

    #[test]
    fn rendering_is_deterministic() {
        let a = ScaffoldVars::from_name("demo").render("{{name}}/{{name_snake}}");
        let b = ScaffoldVars::from_name("demo").render("{{name}}/{{name_snake}}");
        assert_eq!(a, b);
    }

    /// Every template must be embedded, and every one must render without
    /// leaving a placeholder behind — an unsubstituted `{{…}}` in emitted
    /// source is a syntax error in every scaffolded language.
    #[test]
    fn a_rendered_asset_path_cannot_leave_the_target() {
        assert!(rendered_path_stays_inside("demo_app/domain/count.py"));
        assert!(!rendered_path_stays_inside("../x/domain/count.py"));
        assert!(!rendered_path_stays_inside("a/../../b.py"));
        assert!(!rendered_path_stays_inside("/etc/passwd"));
    }

    #[test]
    fn no_template_leaves_a_placeholder() {
        let vars = ScaffoldVars::from_name("demo-app");
        let mut seen = 0;
        for path in crate::assets::Assets::iter() {
            // Only the language trees. Anything else under `scaffold/` is not
            // a scaffold template and does not go through this substitution.
            if !SCAFFOLD_LANGS.iter().any(|(l, _)| path.starts_with(&format!("scaffold/{l}/"))) {
                continue;
            }
            let body = crate::assets::Assets::get_str(&path).expect("embedded");
            let out = vars.render(&body);
            assert!(!out.contains("{{"), "{path} still contains a placeholder");
            seen += 1;
        }
        assert!(seen >= 20, "expected the three scaffold trees to be embedded, saw {seen}");
    }
}
