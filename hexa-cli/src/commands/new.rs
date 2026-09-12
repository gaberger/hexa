//! Structured project intake (ADR-2026-04-13-1500 §2).
//!
//! `hexa new <path>` — create or adopt a directory, run `hexa init`, register the
//! project, seed trust at suggest level, and optionally copy taste preferences.
//!
//! Non-interactive mode: `hexa new ./myapp --name myapp --description "My app"`

use colored::Colorize;

use super::init::InitArgs;

pub async fn run(
    path: &str,
    name: Option<String>,
    _description: Option<String>,
    lang: &str,
) -> anyhow::Result<()> {
    // ── 1. Ensure target directory exists ─────────────────────────────
    let target = std::path::Path::new(path);
    if !target.exists() {
        std::fs::create_dir_all(target)?;
    }
    let abs_path = target
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let dir_name = abs_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());

    let proj_name = name.clone().unwrap_or_else(|| dir_name.clone());

    println!(
        "\n{} Creating project {} at {}",
        "\u{2b21}".cyan(),
        proj_name.bold(),
        abs_path.display().to_string().dimmed()
    );

    // ── 2. Run hexa init (reuse existing init logic) ──────────────────
    let init_args = InitArgs {
        path: abs_path.display().to_string(),
        name: Some(proj_name.clone()),
        // `hexa new` means "give me a project I can run". A skeleton with no
        // manifest and no test is not one, so the scaffold is not optional
        // here — see ADR-2609121400.
        scaffold: true,
        lang: lang.to_string(),
        no_claude_md: false,
        force: false,
    };

    // init::run prints its own progress; if the project is already initialized
    // it will bail with a helpful message (use --force to reinit).
    super::init::run(init_args).await?;

    // ── 6. Summary ───────────────────────────────────────────────────
    println!();
    let separator = "\u{2500}".repeat(50);
    println!("  {}", separator.dimmed());
    println!(
        "  {} Project {} created. Run {} to check it.",
        "\u{2713}".green(),
        proj_name.bold(),
        "hexa status".cyan(),
    );
    println!("  {}", separator.dimmed());
    println!();

    Ok(())
}

